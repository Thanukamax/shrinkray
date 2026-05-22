//! v0.6.1 end-to-end test: copy Pamali into a tempdir, ensure a backup,
//! run the full apply pipeline (sidecar IPC → bytes → repak rewrite → backup
//! manifest entry).
//!
//! Pamali specifically trips the v0.6.1 inflation gate: repak's `flate2::
//! Compression::fast()` (zlib level 1) re-emits Pamali's level-9-cooked
//! entries ~13% larger, so even with a 20 MB mip drop the rewrite is bigger
//! than the original. The pipeline detects this and refuses to touch the
//! original pak — we assert that bail behaviour here (it's the load-bearing
//! safety property of v0.6.1). v0.6.2 will patch repak's compression level
//! and this test flips to assert real shrinkage instead.
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
fn pamali_end_to_end_trips_inflation_gate_and_preserves_pak() {
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

    // Initialise a fresh backup — apply gate requires it.
    Backup::new(&folder, BackupMode::Differential).expect("backup init");

    let targets = vec![StripTarget {
        asset_path:
            "pjtRedLipstickDemo/Content/MainModules/GhostRigs/FL01_whiteLady/TWL_Material/T_hairMask03.uasset"
                .into(),
        max_dim: 1024,
    }];

    let err = apply_strip_mips_to_folder_impl(
        &mut sidecar,
        &folder.to_string_lossy(),
        &pak_copy.to_string_lossy(),
        &targets,
        Some("GAME_UE4_22"),
        Some("VER_UE4_22"),
    )
    .expect_err("Pamali should trip the v0.6.1 inflation gate");

    let msg = err.to_string();
    assert!(
        msg.contains("inflate"),
        "expected inflation diagnostic, got: {msg}",
    );

    // The original pak on disk is preserved byte-exact (load-bearing safety).
    assert_eq!(
        std::fs::read(&pak_copy).expect("re-read pak"),
        original_bytes,
        "inflation gate must not touch the original pak",
    );

    // No partial manifest entry — backup is still empty.
    let backup_loaded = Backup::load(&folder).expect("backup reload");
    assert_eq!(
        backup_loaded.entries().len(),
        0,
        "no manifest entry should be recorded on a gated failure",
    );

    // No leftover temp file.
    let tmp_pak = pak_copy.with_extension("pak.shrinkray-tmp");
    assert!(
        !tmp_pak.exists(),
        "temp pak file leaked: {}", tmp_pak.display(),
    );
}
