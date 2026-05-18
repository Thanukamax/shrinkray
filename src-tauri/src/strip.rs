//! Step 3 — L10N stripping + pak trimming (the v0.1.0 MVP gate).
//!
//! Two destructive operations, both gated by an existing backup manifest:
//! 1. Loose-file delete for anything under `L10N/<lang>/` or
//!    `Localization/<target>/<lang>/` where the user dropped `<lang>`.
//! 2. Pak trimming: open each Readable pak, drop entries matching the same
//!    rule, re-emit the survivors via repak's `into_pakwriter` (preserving
//!    version + mount + path-hash-seed). If every entry would be dropped, the
//!    pak file itself is deleted instead of writing an empty one.
//!
//! Signed (.sig sibling) and encrypted paks are skipped — Stream E + A.
//! IoStore `.utoc/.ucas` are out of scope until Step 2 of the v2 sidecar.

use anyhow::{Context, Result};
use repak::PakBuilder;
use serde::Serialize;
use std::collections::HashSet;
use std::fs::{self, File};
use std::io::{BufReader, BufWriter};
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

use crate::analyze;
use crate::backup::Backup;
use crate::pak::{self, PakClassification};

#[derive(Debug, Serialize, Clone)]
pub struct PlannedFile {
    pub path: String,
    pub size: u64,
    pub language: String,
}

#[derive(Debug, Serialize, Clone)]
pub struct PlannedPakChange {
    pub pak: String,
    pub dropped_entries: usize,
    pub kept_entries: usize,
    pub becomes_empty: bool,
}

#[derive(Debug, Serialize, Default)]
pub struct StripPlan {
    pub root: String,
    pub drop_languages: Vec<String>,
    pub loose_files: Vec<PlannedFile>,
    pub pak_changes: Vec<PlannedPakChange>,
    pub skipped_signed_paks: Vec<String>,
    pub skipped_encrypted_paks: Vec<String>,
    pub skipped_unreadable_paks: Vec<String>,
    pub total_loose_bytes: u64,
}

#[derive(Debug, Serialize)]
pub struct PakRewrite {
    pub pak: String,
    pub original_size: u64,
    pub new_size: u64,
    pub dropped_entries: usize,
}

#[derive(Debug, Serialize)]
pub struct StripFailure {
    pub path: String,
    pub reason: String,
}

#[derive(Debug, Serialize, Default)]
pub struct StripReport {
    pub deleted_files: Vec<String>,
    pub rewritten_paks: Vec<PakRewrite>,
    pub deleted_paks: Vec<String>,
    pub failures: Vec<StripFailure>,
    pub total_bytes_saved: u64,
}

/// Read-only dry-run: classifies what would be deleted / rewritten without
/// touching disk. Cheap enough to recompute as the user toggles languages.
pub fn plan(root: &Path, drop_languages: &HashSet<String>) -> Result<StripPlan> {
    let canonical = root.canonicalize()
        .with_context(|| format!("canonicalize {}", root.display()))?;
    let mut out = StripPlan {
        root: canonical.to_string_lossy().into_owned(),
        drop_languages: {
            let mut v: Vec<String> = drop_languages.iter().cloned().collect();
            v.sort();
            v
        },
        ..Default::default()
    };

    let mut pak_paths: Vec<PathBuf> = Vec::new();

    for entry in WalkDir::new(&canonical).into_iter().filter_map(Result::ok) {
        if !entry.file_type().is_file() {
            continue;
        }
        let abs = entry.path();
        let ext = abs.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
        if ext == "pak" {
            pak_paths.push(abs.to_path_buf());
            continue;
        }
        if let Some(lang) = analyze::detect_language(abs) {
            if drop_languages.contains(&lang) {
                let size = entry.metadata().map(|m| m.len()).unwrap_or(0);
                out.loose_files.push(PlannedFile {
                    path: rel_posix(abs, &canonical),
                    size,
                    language: lang,
                });
                out.total_loose_bytes += size;
            }
        }
    }

    for pak_path in &pak_paths {
        let rel = rel_posix(pak_path, &canonical);
        match pak::classify_pak(pak_path) {
            PakClassification::Signed => out.skipped_signed_paks.push(rel),
            PakClassification::Encrypted => out.skipped_encrypted_paks.push(rel),
            PakClassification::Unreadable { .. } => out.skipped_unreadable_paks.push(rel),
            PakClassification::Readable => {
                if let Some(change) = plan_pak_change(pak_path, &rel, drop_languages)? {
                    out.pak_changes.push(change);
                }
            }
        }
    }

    Ok(out)
}

fn plan_pak_change(
    pak_path: &Path,
    rel: &str,
    drop_languages: &HashSet<String>,
) -> Result<Option<PlannedPakChange>> {
    let file = File::open(pak_path)
        .with_context(|| format!("open {}", pak_path.display()))?;
    let mut reader = BufReader::new(file);
    let pak = PakBuilder::new().reader(&mut reader)
        .with_context(|| format!("read pak {}", pak_path.display()))?;

    let mut dropped = 0usize;
    let mut kept = 0usize;
    for entry_path in pak.files() {
        if entry_matches_dropped_language(&entry_path, drop_languages) {
            dropped += 1;
        } else {
            kept += 1;
        }
    }
    if dropped == 0 {
        return Ok(None);
    }
    Ok(Some(PlannedPakChange {
        pak: rel.to_string(),
        dropped_entries: dropped,
        kept_entries: kept,
        becomes_empty: kept == 0,
    }))
}

/// Apply the plan, mutating disk. Caller MUST pass a live `Backup` so every
/// destructive op is recorded first. Per-pak / per-file failures collected
/// into `failures`; one bad pak does not halt the run.
pub fn apply(
    root: &Path,
    drop_languages: &HashSet<String>,
    backup: &mut Backup,
) -> Result<StripReport> {
    let plan = plan(root, drop_languages)?;
    let canonical = PathBuf::from(&plan.root);
    let mut report = StripReport::default();

    for file in &plan.loose_files {
        let abs = canonical.join(&file.path);
        match delete_loose_file(&abs, backup) {
            Ok(saved) => {
                report.deleted_files.push(file.path.clone());
                report.total_bytes_saved += saved;
            }
            Err(e) => report.failures.push(StripFailure {
                path: file.path.clone(),
                reason: e.to_string(),
            }),
        }
    }

    for change in &plan.pak_changes {
        let abs = canonical.join(&change.pak);
        if change.becomes_empty {
            match delete_pak_entirely(&abs, backup) {
                Ok(saved) => {
                    report.deleted_paks.push(change.pak.clone());
                    report.total_bytes_saved += saved;
                }
                Err(e) => report.failures.push(StripFailure {
                    path: change.pak.clone(),
                    reason: e.to_string(),
                }),
            }
        } else {
            match rewrite_pak(&abs, drop_languages, backup) {
                Ok(rewrite) => {
                    report.total_bytes_saved += rewrite.original_size.saturating_sub(rewrite.new_size);
                    report.rewritten_paks.push(rewrite);
                }
                Err(e) => report.failures.push(StripFailure {
                    path: change.pak.clone(),
                    reason: e.to_string(),
                }),
            }
        }
    }

    Ok(report)
}

fn delete_loose_file(abs: &Path, backup: &mut Backup) -> Result<u64> {
    let size = fs::metadata(abs).map(|m| m.len()).unwrap_or(0);
    backup.record_delete(abs)?;
    fs::remove_file(abs).with_context(|| format!("remove {}", abs.display()))?;
    Ok(size)
}

fn delete_pak_entirely(abs: &Path, backup: &mut Backup) -> Result<u64> {
    let size = fs::metadata(abs).map(|m| m.len()).unwrap_or(0);
    backup.record_delete(abs)?;
    fs::remove_file(abs).with_context(|| format!("remove {}", abs.display()))?;
    Ok(size)
}

fn rewrite_pak(
    pak_path: &Path,
    drop_languages: &HashSet<String>,
    backup: &mut Backup,
) -> Result<PakRewrite> {
    let original_size = fs::metadata(pak_path).map(|m| m.len()).unwrap_or(0);

    // Phase 1: read everything we want to keep, while the input file is open.
    let (kept_entries, dropped_count, version, mount_point, path_hash_seed) = {
        let file = File::open(pak_path)
            .with_context(|| format!("open {}", pak_path.display()))?;
        let mut reader = BufReader::new(file);
        let pak = PakBuilder::new().reader(&mut reader)
            .with_context(|| format!("read pak {}", pak_path.display()))?;

        let version = pak.version();
        let mount_point = pak.mount_point().to_string();
        let path_hash_seed = pak.path_hash_seed();

        let mut kept: Vec<(String, Vec<u8>)> = Vec::new();
        let mut dropped = 0usize;
        for entry_path in pak.files() {
            if entry_matches_dropped_language(&entry_path, drop_languages) {
                dropped += 1;
                continue;
            }
            let bytes = pak.get(&entry_path, &mut reader)
                .with_context(|| format!("extract {} from {}", entry_path, pak_path.display()))?;
            kept.push((entry_path, bytes));
        }
        (kept, dropped, version, mount_point, path_hash_seed)
    };

    if dropped_count == 0 {
        // Nothing to do — shouldn't reach here if plan() did its job.
        return Ok(PakRewrite {
            pak: pak_path.to_string_lossy().into_owned(),
            original_size,
            new_size: original_size,
            dropped_entries: 0,
        });
    }

    // Phase 2: write trimmed pak to a temp file in the same directory so the
    // final rename is atomic on the same filesystem.
    let tmp_path = pak_path.with_extension("pak.shrinkray-tmp");
    {
        let out_file = File::create(&tmp_path)
            .with_context(|| format!("create {}", tmp_path.display()))?;
        let out_writer = BufWriter::new(out_file);
        let mut writer = PakBuilder::new().writer(out_writer, version, mount_point, path_hash_seed);
        for (path, bytes) in kept_entries {
            writer.write_file(&path, false, bytes)
                .with_context(|| format!("write entry {} into {}", path, tmp_path.display()))?;
        }
        writer.write_index()
            .with_context(|| format!("finalise {}", tmp_path.display()))?;
    }

    // Phase 3: read new pak bytes, record backup with both original + new content,
    // then atomic rename.
    let new_bytes = fs::read(&tmp_path)
        .with_context(|| format!("read {}", tmp_path.display()))?;
    let new_size = new_bytes.len() as u64;
    backup.record_full_replace(pak_path, &new_bytes)?;
    fs::rename(&tmp_path, pak_path)
        .with_context(|| format!("rename {} -> {}", tmp_path.display(), pak_path.display()))?;

    Ok(PakRewrite {
        pak: pak_path.to_string_lossy().into_owned(),
        original_size,
        new_size,
        dropped_entries: dropped_count,
    })
}

fn entry_matches_dropped_language(entry_path: &str, drop_languages: &HashSet<String>) -> bool {
    // Pak entry paths can carry either separator; normalise to a POSIX path so
    // analyze::detect_language's component iteration works on every platform.
    let normalized = entry_path.replace('\\', "/");
    match analyze::detect_language(Path::new(&normalized)) {
        Some(lang) => drop_languages.contains(&lang),
        None => false,
    }
}

fn rel_posix(abs: &Path, root: &Path) -> String {
    let stripped = abs.strip_prefix(root).unwrap_or(abs);
    stripped
        .components()
        .map(|c| c.as_os_str().to_string_lossy().into_owned())
        .collect::<Vec<_>>()
        .join("/")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backup::BackupMode;
    use repak::{PakBuilder, Version};

    /// Build a minimal real pak file with the given entries (path -> bytes).
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

    fn setup_game_with_pak(tmp: &Path, entries: &[(&str, &[u8])]) -> PathBuf {
        let root = tmp.join("Game");
        let paks = root.join("Content/Paks");
        fs::create_dir_all(&paks).unwrap();
        make_pak(&paks.join("pakchunk0.pak"), entries);
        root
    }

    fn drop_set(langs: &[&str]) -> HashSet<String> {
        langs.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn plan_finds_loose_l10n_files() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("Game");
        fs::create_dir_all(root.join("Content/L10N/fr")).unwrap();
        fs::create_dir_all(root.join("Content/L10N/en")).unwrap();
        fs::write(root.join("Content/L10N/fr/voice.uasset"), vec![0u8; 3000]).unwrap();
        fs::write(root.join("Content/L10N/en/voice.uasset"), vec![0u8; 2000]).unwrap();

        let plan = plan(&root, &drop_set(&["fr"])).unwrap();
        assert_eq!(plan.loose_files.len(), 1);
        assert_eq!(plan.loose_files[0].language, "fr");
        assert_eq!(plan.loose_files[0].size, 3000);
        assert_eq!(plan.total_loose_bytes, 3000);
    }

    #[test]
    fn plan_classifies_pak_entries() {
        let tmp = tempfile::tempdir().unwrap();
        let root = setup_game_with_pak(
            tmp.path(),
            &[
                ("MyGame/Content/L10N/fr/voice.uasset", b"french"),
                ("MyGame/Content/L10N/en/voice.uasset", b"english"),
                ("MyGame/Content/Maps/main.umap", b"map"),
            ],
        );

        let plan = plan(&root, &drop_set(&["fr"])).unwrap();
        assert_eq!(plan.pak_changes.len(), 1);
        let ch = &plan.pak_changes[0];
        assert_eq!(ch.dropped_entries, 1);
        assert_eq!(ch.kept_entries, 2);
        assert!(!ch.becomes_empty);
    }

    #[test]
    fn plan_marks_pak_as_becoming_empty() {
        let tmp = tempfile::tempdir().unwrap();
        let root = setup_game_with_pak(
            tmp.path(),
            &[
                ("MyGame/Content/L10N/fr/voice1.uasset", b"french1"),
                ("MyGame/Content/L10N/fr/voice2.uasset", b"french2"),
            ],
        );

        let plan = plan(&root, &drop_set(&["fr"])).unwrap();
        assert_eq!(plan.pak_changes.len(), 1);
        let ch = &plan.pak_changes[0];
        assert_eq!(ch.dropped_entries, 2);
        assert_eq!(ch.kept_entries, 0);
        assert!(ch.becomes_empty);
    }

    #[test]
    fn plan_skips_signed_pak() {
        let tmp = tempfile::tempdir().unwrap();
        let root = setup_game_with_pak(
            tmp.path(),
            &[("MyGame/Content/L10N/fr/voice.uasset", b"french")],
        );
        let sig = root.join("Content/Paks/pakchunk0.sig");
        fs::write(&sig, b"signature").unwrap();

        let plan = plan(&root, &drop_set(&["fr"])).unwrap();
        assert_eq!(plan.skipped_signed_paks.len(), 1);
        assert_eq!(plan.pak_changes.len(), 0);
    }

    #[test]
    fn apply_deletes_loose_files_then_restores() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("Game");
        fs::create_dir_all(root.join("Content/L10N/fr")).unwrap();
        fs::write(root.join("Content/L10N/fr/voice.uasset"), b"french_audio").unwrap();
        fs::write(root.join("Content/L10N/fr/extra.uasset"), b"more_french").unwrap();

        let mut backup = Backup::new(&root, BackupMode::Differential).unwrap();
        let report = apply(&root, &drop_set(&["fr"]), &mut backup).unwrap();
        assert_eq!(report.deleted_files.len(), 2);
        assert_eq!(report.failures.len(), 0);
        assert!(report.total_bytes_saved > 0);
        assert!(!root.join("Content/L10N/fr/voice.uasset").exists());

        let restore = backup.restore().unwrap();
        assert!(restore.failures.is_empty());
        assert!(root.join("Content/L10N/fr/voice.uasset").exists());
        assert_eq!(
            fs::read(root.join("Content/L10N/fr/voice.uasset")).unwrap(),
            b"french_audio"
        );
    }

    #[test]
    fn apply_rewrites_pak_dropping_matching_entries() {
        let tmp = tempfile::tempdir().unwrap();
        let root = setup_game_with_pak(
            tmp.path(),
            &[
                ("MyGame/Content/L10N/fr/voice.uasset", b"french_audio_bytes"),
                ("MyGame/Content/L10N/en/voice.uasset", b"english_audio_bytes"),
                ("MyGame/Content/Maps/main.umap", b"map_bytes"),
            ],
        );
        let pak_path = root.join("Content/Paks/pakchunk0.pak");
        let original_pak = fs::read(&pak_path).unwrap();

        let mut backup = Backup::new(&root, BackupMode::Differential).unwrap();
        let report = apply(&root, &drop_set(&["fr"]), &mut backup).unwrap();
        assert_eq!(report.rewritten_paks.len(), 1);
        assert_eq!(report.deleted_paks.len(), 0);
        assert_eq!(report.failures.len(), 0);
        assert_eq!(report.rewritten_paks[0].dropped_entries, 1);

        // The new pak should still open + only contain en + Maps.
        let new_file = File::open(&pak_path).unwrap();
        let mut new_reader = BufReader::new(new_file);
        let new_pak = PakBuilder::new().reader(&mut new_reader).unwrap();
        let mut files = new_pak.files();
        files.sort();
        assert_eq!(
            files,
            vec![
                "MyGame/Content/L10N/en/voice.uasset".to_string(),
                "MyGame/Content/Maps/main.umap".to_string(),
            ]
        );

        // Restore must put the original byte-identical pak back.
        let restore = backup.restore().unwrap();
        assert!(restore.failures.is_empty());
        assert_eq!(fs::read(&pak_path).unwrap(), original_pak);
    }

    #[test]
    fn apply_deletes_pak_that_becomes_empty() {
        let tmp = tempfile::tempdir().unwrap();
        let root = setup_game_with_pak(
            tmp.path(),
            &[
                ("MyGame/Content/L10N/fr/voice.uasset", b"french_audio_bytes"),
            ],
        );
        let pak_path = root.join("Content/Paks/pakchunk0.pak");
        let original_pak = fs::read(&pak_path).unwrap();

        let mut backup = Backup::new(&root, BackupMode::Differential).unwrap();
        let report = apply(&root, &drop_set(&["fr"]), &mut backup).unwrap();
        assert_eq!(report.deleted_paks.len(), 1);
        assert_eq!(report.rewritten_paks.len(), 0);
        assert!(!pak_path.exists());

        let restore = backup.restore().unwrap();
        assert!(restore.failures.is_empty());
        assert!(pak_path.exists());
        assert_eq!(fs::read(&pak_path).unwrap(), original_pak);
    }

    #[test]
    fn apply_is_noop_when_no_matches() {
        let tmp = tempfile::tempdir().unwrap();
        let root = setup_game_with_pak(
            tmp.path(),
            &[("MyGame/Content/Maps/main.umap", b"map_bytes")],
        );
        fs::create_dir_all(root.join("Content/L10N/en")).unwrap();
        fs::write(root.join("Content/L10N/en/voice.uasset"), b"english").unwrap();

        let mut backup = Backup::new(&root, BackupMode::Differential).unwrap();
        let report = apply(&root, &drop_set(&["fr"]), &mut backup).unwrap();
        assert_eq!(report.deleted_files.len(), 0);
        assert_eq!(report.rewritten_paks.len(), 0);
        assert_eq!(report.deleted_paks.len(), 0);
        assert_eq!(report.total_bytes_saved, 0);
    }

    #[test]
    fn entry_matcher_normalises_separators() {
        let drops = drop_set(&["fr"]);
        assert!(entry_matches_dropped_language(
            "MyGame/Content/L10N/fr/voice.uasset",
            &drops
        ));
        assert!(entry_matches_dropped_language(
            "MyGame\\Content\\L10N\\fr\\voice.uasset",
            &drops
        ));
        assert!(!entry_matches_dropped_language(
            "MyGame/Content/L10N/en/voice.uasset",
            &drops
        ));
    }
}
