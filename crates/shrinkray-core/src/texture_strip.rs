//! v0.6.1 — pak rewrite for cooked-texture mip strips.
//!
//! The sidecar produces modified `.uasset/.uexp/.ubulk` bytes per texture
//! (see `crates/shrinkray-sidecar::apply_strip_mips`). This module takes those
//! bytes, substitutes them into the original pak via `repak`, records the
//! rewrite through `Backup::record_pak_rewrite_with_strips`, and atomic-renames
//! the new pak into place.
//!
//! Why core (not src-tauri or the sidecar crate):
//!   - L10N strip already lives here and does the analogous repak rewrite —
//!     keeping both pak rewrites side-by-side makes the shared invariants
//!     (atomic rename, mount-point preservation, version pin) easier to keep
//!     consistent.
//!   - Pure-Rust + a pure-data input. The Tauri layer constructs the input
//!     from the sidecar's IPC response and hands it in, so core has zero
//!     knowledge of the IPC transport and tests don't need the .NET binary.

use std::collections::HashMap;
use std::fs::{self, File};
use std::io::{BufReader, BufWriter};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use repak::PakBuilder;
use serde::Serialize;

use crate::backup::{Backup, TextureStripRecord};

/// One sidecar-applied texture, with its modified triple bytes and the
/// metadata we need to surface to the v0.7 AI re-expand path. The Tauri
/// layer converts `shrinkray_sidecar::StripAppliedTexture` into this shape
/// after base64-decoding the file payloads.
#[derive(Debug, Clone)]
pub struct AppliedTexture {
    pub asset_path: String,
    pub export_name: String,
    pub drop_mip_count: u32,
    pub kept_mip_count: u32,
    pub original_top_dim: u32,
    pub stripped_top_dim: u32,
    pub saved_bytes: u64,
    pub pixel_format: String,
    pub compression_settings: Option<String>,
    /// `pak_path` (POSIX-style, pak-internal) → modified bytes.
    pub files: HashMap<String, Vec<u8>>,
}

/// One sidecar-rejected texture, surfaced so the UI can render an inline
/// diagnostic next to the texture it failed on.
#[derive(Debug, Clone, Serialize)]
pub struct SkippedTexture {
    pub asset_path: String,
    pub reason: String,
}

#[derive(Debug, Serialize, Default)]
pub struct PakStripReport {
    /// Host-fs path to the pak that was rewritten (or the input pak if
    /// nothing applied).
    pub pak: String,
    /// Original pak size in bytes (pre-rewrite).
    pub original_size: u64,
    /// New pak size in bytes (post-rewrite). Equals `original_size` when no
    /// rewrite was performed.
    pub new_size: u64,
    /// Per-texture records that made it into the new pak.
    pub applied: Vec<TextureStripRecord>,
    /// Sidecar-rejected textures, mirrored from the IPC response.
    pub skipped: Vec<SkippedTexture>,
    /// Bytes saved across all applied textures (sum of dropped mips' SizeOnDisk).
    pub total_saved_bytes: u64,
}

/// Apply pre-computed sidecar mip strips to a single pak. Reads the pak's
/// entries, substitutes the modified bytes for each applied texture's triple,
/// re-emits the pak via `repak::PakBuilder` preserving version + mount +
/// path-hash-seed, records via the backup manifest, and atomic-renames.
///
/// If `applied` is empty (every target was skipped), this is a no-op — we
/// don't rewrite the pak just to copy its bytes verbatim.
pub fn apply_to_pak(
    pak_path: &Path,
    applied: Vec<AppliedTexture>,
    skipped: Vec<SkippedTexture>,
    backup: &mut Backup,
) -> Result<PakStripReport> {
    let original_size = fs::metadata(pak_path).map(|m| m.len()).unwrap_or(0);

    if applied.is_empty() {
        return Ok(PakStripReport {
            pak: pak_path.to_string_lossy().into_owned(),
            original_size,
            new_size: original_size,
            applied: Vec::new(),
            skipped,
            total_saved_bytes: 0,
        });
    }

    // Build a substitution map (pak-internal path → modified bytes) from all
    // applied textures' triple file maps. Same key collisions across textures
    // would be a bug — assert on debug builds.
    let mut substitutions: HashMap<String, Vec<u8>> = HashMap::new();
    for tex in &applied {
        for (pak_internal_path, bytes) in &tex.files {
            debug_assert!(
                !substitutions.contains_key(pak_internal_path),
                "two textures claim the same pak entry: {}", pak_internal_path,
            );
            substitutions.insert(pak_internal_path.clone(), bytes.clone());
        }
    }

    // Phase 1: read every entry from the original pak. Substitute as we go.
    // Capture the input pak's compression slots so the new pak preserves the
    // same compression profile — Pamali (and most cooked UE paks) ships zlib-
    // compressed entries, and writing them back uncompressed would 2x the pak
    // size and undo the whole point of stripping.
    let (entries, version, mount_point, path_hash_seed, allowed_compression) = {
        let file = File::open(pak_path)
            .with_context(|| format!("open {}", pak_path.display()))?;
        let mut reader = BufReader::new(file);
        let pak = PakBuilder::new().reader(&mut reader)
            .with_context(|| format!("read pak {}", pak_path.display()))?;

        let version = pak.version();
        let mount_point = pak.mount_point().to_string();
        let path_hash_seed = pak.path_hash_seed();
        let allowed_compression = pak.used_compression();

        let mut entries: Vec<(String, Vec<u8>)> = Vec::new();
        for entry_path in pak.files() {
            let bytes = if let Some(replacement) = substitutions.remove(&entry_path) {
                replacement
            } else {
                pak.get(&entry_path, &mut reader)
                    .with_context(|| format!("extract {} from {}", entry_path, pak_path.display()))?
            };
            entries.push((entry_path, bytes));
        }
        (entries, version, mount_point, path_hash_seed, allowed_compression)
    };

    if !substitutions.is_empty() {
        let missing: Vec<&str> = substitutions.keys().map(|s| s.as_str()).collect();
        anyhow::bail!(
            "{} sidecar-modified entr{} not found in pak {}: {}",
            missing.len(),
            if missing.len() == 1 { "y" } else { "ies" },
            pak_path.display(),
            missing.join(", "),
        );
    }

    // Phase 2: write the new pak to a sibling temp file (same dir → same fs
    // → atomic rename works). Forward the input pak's allowed compression
    // set so re-emitted entries stay compressed (otherwise the pak inflates).
    let tmp_path: PathBuf = pak_path.with_extension("pak.shrinkray-tmp");
    {
        let out_file = File::create(&tmp_path)
            .with_context(|| format!("create {}", tmp_path.display()))?;
        let out_writer = BufWriter::new(out_file);
        let mut builder = PakBuilder::new();
        if !allowed_compression.is_empty() {
            builder = builder.compression(allowed_compression);
        }
        let mut writer = builder.writer(out_writer, version, mount_point, path_hash_seed);
        for (path, bytes) in entries {
            writer.write_file(&path, true, bytes)
                .with_context(|| format!("write entry {} into {}", path, tmp_path.display()))?;
        }
        writer.write_index()
            .with_context(|| format!("finalise {}", tmp_path.display()))?;
    }

    // Phase 3: read new pak bytes for the backup payload, build strip records,
    // record via backup, then atomic-rename.
    let new_bytes = fs::read(&tmp_path)
        .with_context(|| format!("read {}", tmp_path.display()))?;
    let new_size = new_bytes.len() as u64;

    // Inflation safety gate. v0.6.2 vendors a patched repak that uses
    // `Compression::best()` so most cooked UE paks shrink correctly, but we
    // keep the gate as a belt-and-braces check: if a future repak update or
    // an edge-case pak layout produces a larger rewrite anyway, we'd rather
    // fail loudly than silently grow the user's game folder.
    if new_size > original_size {
        let _ = fs::remove_file(&tmp_path);
        anyhow::bail!(
            "rewrite would inflate pak from {} → {} bytes — aborted before touching the original pak. \
             This is unexpected with v0.6.2's patched repak; please report the pak + targets.",
            original_size,
            new_size,
        );
    }

    let mut total_saved_bytes = 0u64;
    let strip_records: Vec<TextureStripRecord> = applied.iter().map(|t| {
        total_saved_bytes += t.saved_bytes;
        TextureStripRecord {
            asset_path: t.asset_path.clone(),
            export_name: t.export_name.clone(),
            original_top_dim: t.original_top_dim,
            stripped_top_dim: t.stripped_top_dim,
            drop_mip_count: t.drop_mip_count,
            kept_mip_count: t.kept_mip_count,
            pixel_format: t.pixel_format.clone(),
            compression_settings: t.compression_settings.clone(),
            saved_bytes: t.saved_bytes,
        }
    }).collect();

    backup.record_pak_rewrite_with_strips(pak_path, &new_bytes, strip_records.clone())?;
    fs::rename(&tmp_path, pak_path)
        .with_context(|| format!("rename {} -> {}", tmp_path.display(), pak_path.display()))?;

    Ok(PakStripReport {
        pak: pak_path.to_string_lossy().into_owned(),
        original_size,
        new_size,
        applied: strip_records,
        skipped,
        total_saved_bytes,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backup::BackupMode;
    use repak::{PakBuilder, Version};

    /// Build a minimal real pak with the given entries — same helper as
    /// strip.rs uses but inlined here to keep tests self-contained.
    fn make_pak(path: &Path, entries: &[(&str, &[u8])]) {
        let file = File::create(path).unwrap();
        let out = BufWriter::new(file);
        let mut writer = PakBuilder::new().writer(
            out,
            Version::V11,
            "../../../".to_string(),
            Some(0xDEADBEEF),
        );
        for (p, bytes) in entries {
            writer.write_file(p, false, bytes.to_vec()).unwrap();
        }
        writer.write_index().unwrap();
    }

    fn make_game(tmp: &Path) -> (PathBuf, PathBuf) {
        let root = tmp.join("GameFolder");
        std::fs::create_dir_all(root.join("Content/Paks")).unwrap();
        let pak = root.join("Content/Paks/pakchunk0.pak");
        make_pak(
            &pak,
            &[
                ("Game/Content/T_hair.uasset", b"original_uasset_bytes_for_hair_xxx"),
                ("Game/Content/T_hair.uexp", b"original_uexp_bytes_for_hair_xxxxx"),
                ("Game/Content/T_hair.ubulk", b"original_ubulk_bytes_for_hair_xxx_padded_to_be_larger_than_replacement"),
                ("Game/Content/Other.uasset", b"this_is_untouched_by_the_strip"),
            ],
        );
        (root, pak)
    }

    fn fake_applied(asset_path: &str) -> AppliedTexture {
        let mut files = HashMap::new();
        files.insert("Game/Content/T_hair.uasset".into(), b"new_uasset".to_vec());
        files.insert("Game/Content/T_hair.uexp".into(), b"new_uexp_smaller".to_vec());
        files.insert("Game/Content/T_hair.ubulk".into(), b"new_ubulk_much_smaller".to_vec());
        AppliedTexture {
            asset_path: asset_path.into(),
            export_name: "T_hair".into(),
            drop_mip_count: 2,
            kept_mip_count: 11,
            original_top_dim: 4096,
            stripped_top_dim: 1024,
            saved_bytes: 20_971_520,
            pixel_format: "PF_DXT5".into(),
            compression_settings: None,
            files,
        }
    }

    #[test]
    fn empty_applied_is_noop() {
        let tmp = tempfile::tempdir().unwrap();
        let (root, pak) = make_game(tmp.path());
        let original_size = fs::metadata(&pak).unwrap().len();

        let mut backup = Backup::new(&root, BackupMode::Differential).unwrap();
        let report = apply_to_pak(
            &pak,
            Vec::new(),
            vec![SkippedTexture { asset_path: "Game/Content/Foo.uasset".into(), reason: "test".into() }],
            &mut backup,
        ).unwrap();

        // Pak is untouched.
        assert_eq!(fs::metadata(&pak).unwrap().len(), original_size);
        assert_eq!(report.original_size, original_size);
        assert_eq!(report.new_size, original_size);
        assert_eq!(report.applied.len(), 0);
        assert_eq!(report.skipped.len(), 1);
        assert_eq!(report.total_saved_bytes, 0);
        // No manifest entry recorded for a no-op.
        assert_eq!(backup.entries().len(), 0);
    }

    #[test]
    fn substitution_rewrites_pak_with_smaller_triple() {
        let tmp = tempfile::tempdir().unwrap();
        let (root, pak) = make_game(tmp.path());
        let original_size = fs::metadata(&pak).unwrap().len();

        let mut backup = Backup::new(&root, BackupMode::Differential).unwrap();
        let report = apply_to_pak(
            &pak,
            vec![fake_applied("Game/Content/T_hair.uasset")],
            Vec::new(),
            &mut backup,
        ).unwrap();

        // New pak is smaller (we shrunk the .ubulk dramatically).
        assert!(report.new_size < report.original_size, "new pak should be smaller: orig={} new={}", report.original_size, report.new_size);
        assert_eq!(report.original_size, original_size);
        assert_eq!(report.applied.len(), 1);
        assert_eq!(report.applied[0].stripped_top_dim, 1024);
        assert_eq!(report.total_saved_bytes, 20_971_520);

        // Substituted entries hold the new bytes.
        let file = File::open(&pak).unwrap();
        let mut reader = BufReader::new(file);
        let new_pak = PakBuilder::new().reader(&mut reader).unwrap();
        let uasset = new_pak.get("Game/Content/T_hair.uasset", &mut reader).unwrap();
        let other = new_pak.get("Game/Content/Other.uasset", &mut reader).unwrap();
        assert_eq!(uasset, b"new_uasset");
        assert_eq!(other, b"this_is_untouched_by_the_strip");

        // Backup manifest carries the strip metadata.
        assert_eq!(backup.entries().len(), 1);
        assert_eq!(backup.entries()[0].texture_strips.len(), 1);
        assert_eq!(backup.entries()[0].texture_strips[0].original_top_dim, 4096);
    }

    #[test]
    fn restore_recovers_original_pak() {
        let tmp = tempfile::tempdir().unwrap();
        let (root, pak) = make_game(tmp.path());
        let original_bytes = fs::read(&pak).unwrap();

        let mut backup = Backup::new(&root, BackupMode::Differential).unwrap();
        apply_to_pak(
            &pak,
            vec![fake_applied("Game/Content/T_hair.uasset")],
            Vec::new(),
            &mut backup,
        ).unwrap();

        // After apply: pak is rewritten with smaller bytes.
        assert_ne!(fs::read(&pak).unwrap(), original_bytes);

        // Restore reverts byte-exact via the saved payload.
        let report = backup.restore().unwrap();
        assert!(report.failures.is_empty(), "restore failures: {:?}", report.failures);
        assert_eq!(fs::read(&pak).unwrap(), original_bytes);
    }

    #[test]
    fn inflation_gate_aborts_before_touching_original_pak() {
        // Force the inflation case: substitute every entry with a much-larger
        // blob, so the rewrite is strictly bigger than the input pak.
        let tmp = tempfile::tempdir().unwrap();
        let (root, pak) = make_game(tmp.path());
        let original_bytes = fs::read(&pak).unwrap();

        let mut backup = Backup::new(&root, BackupMode::Differential).unwrap();
        let mut tex = fake_applied("Game/Content/T_hair.uasset");
        // Replace the original triple with 100 KB blobs each → much bigger.
        let bloat = vec![0xABu8; 100_000];
        tex.files.insert("Game/Content/T_hair.uasset".into(), bloat.clone());
        tex.files.insert("Game/Content/T_hair.uexp".into(), bloat.clone());
        tex.files.insert("Game/Content/T_hair.ubulk".into(), bloat);

        let err = apply_to_pak(&pak, vec![tex], Vec::new(), &mut backup).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("inflate"), "expected inflation diagnostic, got: {msg}");
        // Original pak preserved byte-exact.
        assert_eq!(fs::read(&pak).unwrap(), original_bytes);
        // No partial manifest entry recorded.
        assert_eq!(backup.entries().len(), 0);
        // Tempdir file cleaned up.
        let tmp_path = pak.with_extension("pak.shrinkray-tmp");
        assert!(!tmp_path.exists(), "temp pak file leaked: {}", tmp_path.display());
    }

    #[test]
    fn missing_entry_in_pak_errors_cleanly() {
        let tmp = tempfile::tempdir().unwrap();
        let (root, pak) = make_game(tmp.path());

        let mut backup = Backup::new(&root, BackupMode::Differential).unwrap();
        // Build an AppliedTexture whose `files` reference an entry that
        // doesn't exist in the pak. Should bail before writing.
        let mut tex = fake_applied("Game/Content/Ghost.uasset");
        tex.files.clear();
        tex.files.insert("Game/Content/DoesNotExist.uasset".into(), b"unused".to_vec());

        let err = apply_to_pak(&pak, vec![tex], Vec::new(), &mut backup).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("DoesNotExist"), "expected missing-entry diagnostic, got: {msg}");
        // Original pak preserved.
        assert!(pak.exists());
        // No partial backup entry left behind.
        assert_eq!(backup.entries().len(), 0);
    }
}
