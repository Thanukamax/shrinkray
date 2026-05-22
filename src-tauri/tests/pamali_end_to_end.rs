//! v0.6.2 end-to-end test: copy Pamali into a tempdir, ensure a backup,
//! run the full apply pipeline (sidecar IPC → bytes → patched-repak rewrite
//! → backup manifest entry), assert the pak shrinks on disk, then assert
//! `Backup::restore()` reverts it byte-exact.
//!
//! v0.6.1 history: this test originally asserted the inflation gate tripped
//! because upstream repak compressed at zlib level 1 and bloated Pamali's
//! level-9-cooked entries ~13%. v0.6.2 vendors a patched repak that uses
//! `Compression::best()`, landing Pamali rewrites at modest net shrinkage
//! (13 MB net on the T_hairMask03 strip).
//!
//! Gated on the Pamali pak being present at the documented local fixture
//! path (`SHRINKRAY_PAMALI_PAK` env override). Skips cleanly on CI / fresh
//! clones — we don't ship game assets in the public repo.

use std::path::PathBuf;

use shrinkray_core::backup::{Backup, BackupMode};
use shrinkray_lib::apply_strip_mips_to_folder_impl;
use shrinkray_sidecar::{Sidecar, StripTarget};

fn pamali_pak_or_skip() -> Option<PathBuf> {
    let candidate = match std::env::var("SHRINKRAY_PAMALI_PAK") {
        Ok(p) => PathBuf::from(p),
        Err(_) => PathBuf::from(
            "/home/thankamax/Downloads/Misc/Test/Test 2/pjtRedLipstickDemo/Content/Paks/pakchunk0-WindowsNoEditor.pak",
        ),
    };
    if !candidate.exists() {
        eprintln!(
            "SKIP: Pamali pak missing at {} (set SHRINKRAY_PAMALI_PAK to override)",
            candidate.display()
        );
        return None;
    }
    Some(candidate)
}

fn sidecar_or_skip() -> Option<Sidecar> {
    match Sidecar::locate() {
        Ok(p) if p.exists() => Sidecar::spawn(p).ok(),
        _ => {
            eprintln!("SKIP: sidecar binary not found; run scripts/build-sidecar.sh");
            None
        }
    }
}

#[test]
fn pamali_end_to_end_rewrites_pak_smaller_and_restores_byte_exact() {
    let Some(real_pak) = pamali_pak_or_skip() else { return };
    let Some(mut sidecar) = sidecar_or_skip() else { return };

    // Mirror Pamali's relative layout under a temp game-folder root so the
    // backup's relative-path normalisation works as it would for a real install.
    let tmp = tempfile::tempdir().expect("tempdir");
    let folder = tmp.path().join("PamaliFolder");
    let pak_dir = folder.join("Content/Paks");
    std::fs::create_dir_all(&pak_dir).expect("mkdir pak_dir");
    let pak_copy = pak_dir.join("pakchunk0-WindowsNoEditor.pak");
    std::fs::copy(&real_pak, &pak_copy).expect("copy pamali pak");
    let original_bytes = std::fs::read(&pak_copy).expect("read copied pak");
    let original_size = original_bytes.len() as u64;

    // Initialise a fresh backup — apply gate requires it.
    Backup::new(&folder, BackupMode::Differential).expect("backup init");

    let targets = vec![StripTarget {
        asset_path:
            "pjtRedLipstickDemo/Content/MainModules/GhostRigs/FL01_whiteLady/TWL_Material/T_hairMask03.uasset"
                .into(),
        max_dim: 1024,
    }];

    let report = apply_strip_mips_to_folder_impl(
        &mut sidecar,
        &folder.to_string_lossy(),
        &pak_copy.to_string_lossy(),
        &targets,
        Some("GAME_UE4_22"),
        Some("VER_UE4_22"),
    )
    .expect("apply ok with v0.6.2 patched repak");

    // One texture applied, none skipped.
    assert_eq!(report.skipped.len(), 0, "skipped: {:?}", report.skipped);
    assert_eq!(report.applied.len(), 1);
    let a = &report.applied[0];
    assert_eq!(a.original_top_dim, 4096);
    assert_eq!(a.stripped_top_dim, 1024);
    assert_eq!(a.drop_mip_count, 2);
    assert_eq!(a.kept_mip_count, 11);
    assert_eq!(a.saved_bytes, 20_971_520, "exact 20 MB mip-drop savings");
    assert_eq!(a.pixel_format, "PF_DXT5");

    // Pak on disk got smaller. v0.6.2's level-9 zlib lands ~1.5% behind UE's
    // cook on Pamali, so net shrinkage is mip-drop − compression-overhead.
    // Observed empirically: ~13 MB net on a single texture strip.
    assert_eq!(report.original_size, original_size);
    assert!(
        report.new_size < report.original_size,
        "pak should shrink: original={} new={}",
        report.original_size, report.new_size,
    );
    let net_savings = report.original_size - report.new_size;
    assert!(
        net_savings >= 8_000_000,
        "expected at least 8 MB net pak shrinkage on Pamali T_hairMask03; got {net_savings} bytes (orig={} new={})",
        report.original_size, report.new_size,
    );

    // Backup manifest carries the strip metadata for the v0.7 AI re-expand path.
    let backup_loaded = Backup::load(&folder).expect("backup reload");
    assert_eq!(backup_loaded.entries().len(), 1);
    let entry = &backup_loaded.entries()[0];
    assert_eq!(entry.texture_strips.len(), 1);
    assert_eq!(entry.texture_strips[0].original_top_dim, 4096);
    assert_eq!(entry.texture_strips[0].stripped_top_dim, 1024);
    assert_eq!(entry.texture_strips[0].pixel_format, "PF_DXT5");

    // Restore reverts byte-exact via the saved pak payload.
    let restore = backup_loaded.restore().expect("restore");
    assert!(restore.failures.is_empty(), "restore failures: {:?}", restore.failures);
    assert_eq!(
        std::fs::read(&pak_copy).expect("re-read pak"),
        original_bytes,
        "restored pak must match original byte-for-byte",
    );
}
