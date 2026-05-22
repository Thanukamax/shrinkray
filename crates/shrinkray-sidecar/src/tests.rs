use super::*;
use std::fs::File;
use std::io::BufWriter;
use std::path::PathBuf;

/// Build a minimal real pak file with the given entries (path -> bytes).
fn make_pak(path: &Path, entries: &[(&str, &[u8])]) {
    let file = File::create(path).unwrap();
    let out = BufWriter::new(file);
    let mut writer = repak::PakBuilder::new().writer(
        out,
        repak::Version::V11,
        "../../../".to_string(),
        Some(0xDEADBEEF),
    );
    for (p, bytes) in entries {
        writer.write_file(p, false, bytes.to_vec()).unwrap();
    }
    writer.write_index().unwrap();
}

/// Skip all sidecar tests when the binary isn't located. Lets the workspace test pass
/// when running `cargo test` from a fresh clone before anyone runs `dotnet build`.
fn sidecar_or_skip() -> Option<Sidecar> {
    match Sidecar::locate() {
        Ok(p) if p.exists() => Sidecar::spawn(p).ok(),
        _ => None,
    }
}

#[test]
fn ping_returns_version() {
    let Some(mut s) = sidecar_or_skip() else {
        eprintln!("SKIP: sidecar binary not found; run `dotnet build` in sidecar/");
        return;
    };
    let r = s.ping().expect("ping ok");
    assert!(!r.version.is_empty(), "version string is empty");
}

#[test]
fn list_assets_against_synthetic_pak() {
    let Some(mut s) = sidecar_or_skip() else {
        eprintln!("SKIP: sidecar binary not found; run `dotnet build` in sidecar/");
        return;
    };
    let tmp = tempfile::tempdir().unwrap();
    let pak_path = tmp.path().join("test.pak");
    make_pak(
        &pak_path,
        &[
            ("Game/Content/Foo.uasset", b"fakeuassetbytes"),
            ("Game/Content/Foo.uexp", b"fakeuexpbytes"),
            ("Game/Content/Audio/Hello.bnk", b"fakebankbytes"),
        ],
    );

    let r = s.list_assets(&pak_path).expect("list_assets ok");
    assert_eq!(r.encrypted, false);
    assert_eq!(r.entry_count, 3, "expected 3 entries, got {}", r.entry_count);
    assert_eq!(r.entries.len(), 3);
    let names: std::collections::HashSet<_> =
        r.entries.iter().map(|e| e.path.as_str()).collect();
    assert!(names.iter().any(|n| n.ends_with("Foo.uasset")));
    assert!(names.iter().any(|n| n.ends_with("Foo.uexp")));
    assert!(names.iter().any(|n| n.ends_with("Hello.bnk")));
}

#[test]
fn list_assets_limit_truncates() {
    let Some(mut s) = sidecar_or_skip() else { return };
    let tmp = tempfile::tempdir().unwrap();
    let pak_path = tmp.path().join("big.pak");
    let entries: Vec<(String, Vec<u8>)> = (0..10)
        .map(|i| (format!("Game/Content/F{i}.uasset"), vec![0u8; 64]))
        .collect();
    let entry_refs: Vec<(&str, &[u8])> =
        entries.iter().map(|(p, b)| (p.as_str(), b.as_slice())).collect();
    make_pak(&pak_path, &entry_refs);

    let r = s.list_assets_with(&pak_path, Some(3), None).expect("ok");
    assert_eq!(r.entry_count, 10, "entry_count is total, not view size");
    assert_eq!(r.entries.len(), 3);
    assert!(r.truncated);
}

/// Smoke test for the v0.5 mip-strip planner. We can't synthesize a fully
/// valid cooked Texture2D with this minimal `make_pak` helper (the bytes
/// are placeholder data, not real FTexturePlatformData), so this test
/// covers the IPC contract and the empty-result path: pak parses, no
/// textures are recognised, the result is well-formed.
///
/// Real-world byte-exact validation lives in manual pre-release checks
/// against the user's UE4 demo corpus — see CHANGELOG v0.5.0 for the
/// 185-texture / 570 MB save number on Pamali.
#[test]
fn plan_strip_mips_against_synthetic_pak() {
    let Some(mut s) = sidecar_or_skip() else { return };
    let tmp = tempfile::tempdir().unwrap();
    let pak_path = tmp.path().join("synthetic.pak");
    make_pak(
        &pak_path,
        &[
            ("Game/Content/Foo.uasset", b"fakeuassetbytes"),
            ("Game/Content/Foo.uexp", b"fakeuexpbytes"),
        ],
    );

    let r = s
        .plan_strip_mips(&pak_path, 1024, Some(100), Some("GAME_UE4_LATEST"))
        .expect("plan_strip_mips ok");

    // Result shape is what we promise the frontend.
    assert_eq!(r.max_dim, 1024);
    assert_eq!(r.texture_count, 0, "synthetic bytes aren't real textures");
    assert_eq!(r.total_save_bytes, 0);
    assert_eq!(r.total_texture_bytes, 0);
    assert!(r.items.is_empty());
    // class_histogram is the diagnostic surface — empty here because no
    // package successfully deserializes.
    // (No assert on length; just confirm it's a valid Vec, not null.)
    let _: &[crate::ClassCount] = &r.class_histogram;
}

/// v0.6.0-rc1: apply_strip_mips with empty targets returns a well-formed
/// empty result. The full apply path against real cooked content is exercised
/// in the manual Pamali pre-release pass (see CHANGELOG).
#[test]
fn apply_strip_mips_empty_targets_returns_empty_result() {
    let Some(mut s) = sidecar_or_skip() else { return };
    let tmp = tempfile::tempdir().unwrap();
    let pak_path = tmp.path().join("any.pak");
    make_pak(&pak_path, &[("Game/Content/A.uasset", b"x")]);
    let r = s
        .apply_strip_mips(&pak_path, &[], Some("GAME_UE4_LATEST"), None)
        .expect("apply_strip_mips ok");
    assert_eq!(r.applied.len(), 0);
    assert_eq!(r.skipped.len(), 0);
    assert_eq!(r.total_saved_bytes, 0);
}

/// v0.6.0: the parser's tolerance against garbage bytes. We jam a triple of
/// non-UE bytes into a pak, point apply_strip_mips at it, and assert the
/// applier returns a structured skip — UAssetAPI throws on the bad header,
/// the sidecar wraps the error in a StripSkipped, and the IPC layer hands
/// it back as a normal result. Catches regressions where a parser change
/// turns a UAssetAPI exception into an unhandled panic that crashes the
/// sidecar mid-pipeline.
#[test]
fn apply_strip_mips_garbage_uasset_skipped_not_errored() {
    let Some(mut s) = sidecar_or_skip() else { return };
    let tmp = tempfile::tempdir().unwrap();
    let pak_path = tmp.path().join("garbage.pak");
    // Build the triple with bytes that look superficially like a UE asset
    // (the .uasset has a recognisable magic so UAssetAPI gets past the
    // open() call before failing on the malformed name map).
    make_pak(
        &pak_path,
        &[
            ("Game/Content/Bad.uasset", &[0xC1, 0x83, 0x2A, 0x9E, 0, 0, 0, 0, 0x40, 0, 0, 0]),
            ("Game/Content/Bad.uexp", b"not a real uexp"),
            ("Game/Content/Bad.ubulk", b"not a real ubulk"),
        ],
    );
    let targets = vec![StripTarget {
        asset_path: "Game/Content/Bad.uasset".into(),
        max_dim: 1024,
    }];
    let r = s
        .apply_strip_mips(&pak_path, &targets, Some("GAME_UE4_22"), Some("VER_UE4_22"))
        .expect("apply_strip_mips should not error — bad input is a skip");
    assert_eq!(r.applied.len(), 0, "garbage input must not appear to succeed");
    assert_eq!(r.skipped.len(), 1, "garbage input should produce one skip");
    assert!(
        !r.skipped[0].reason.is_empty(),
        "skip reason must be populated for the UI"
    );
}

/// v0.6.0 final byte-exact regression. Runs the applier against a real
/// Pamali pak when one is present at the documented local fixture path —
/// asserts T_hairMask03 strips 4096→1024 with exactly 20 MB saved. Skips
/// cleanly on machines/CI where the pak isn't checked out (we don't ship
/// game assets in the public repo). Pre-release runs explicitly enable this
/// via `SHRINKRAY_PAMALI_PAK=/path/to/pakchunk0-WindowsNoEditor.pak` or by
/// placing the pak at the documented dev path.
#[test]
fn apply_strip_mips_pamali_t_hairmask03_saves_20mb() {
    let Some(mut s) = sidecar_or_skip() else { return };
    let pamali_pak: PathBuf = match std::env::var("SHRINKRAY_PAMALI_PAK") {
        Ok(p) => PathBuf::from(p),
        Err(_) => {
            let default = PathBuf::from(
                "/home/thankamax/Downloads/Misc/Test/Test 2/pjtRedLipstickDemo/Content/Paks/pakchunk0-WindowsNoEditor.pak",
            );
            if !default.exists() {
                eprintln!("SKIP: Pamali pak not present at {} (set SHRINKRAY_PAMALI_PAK to override)", default.display());
                return;
            }
            default
        }
    };
    if !pamali_pak.exists() {
        eprintln!("SKIP: Pamali pak missing at {}", pamali_pak.display());
        return;
    }
    let targets = vec![StripTarget {
        asset_path: "pjtRedLipstickDemo/Content/MainModules/GhostRigs/FL01_whiteLady/TWL_Material/T_hairMask03.uasset".into(),
        max_dim: 1024,
    }];
    let r = s
        .apply_strip_mips(&pamali_pak, &targets, Some("GAME_UE4_22"), Some("VER_UE4_22"))
        .expect("apply_strip_mips ok on Pamali pak");
    assert_eq!(r.skipped.len(), 0, "T_hairMask03 must apply cleanly: skipped={:?}", r.skipped);
    assert_eq!(r.applied.len(), 1, "exactly one texture applied");
    let a = &r.applied[0];
    assert_eq!(a.drop_mip_count, 2, "drop mip 0 (4096) + mip 1 (2048)");
    assert_eq!(a.kept_mip_count, 11, "kept mips 2-12");
    assert_eq!(a.original_top_dim, 4096);
    assert_eq!(a.kept_top_dim, 1024);
    // 4096² + 2048² BC3 = 16 MB + 4 MB = exactly 20 MB.
    assert_eq!(
        a.saved_bytes, 20_971_520,
        "exact 20 MB save expected (mip 0 16 MB + mip 1 4 MB)"
    );
    assert_eq!(r.total_saved_bytes, 20_971_520);
}

/// rc1: pointing the applier at a non-existent asset path inside a pak yields
/// a structured skip rather than a hard error. Important for the UI — bad
/// inputs surface as inline diagnostics, not invoke() rejections.
#[test]
fn apply_strip_mips_missing_asset_skipped_not_errored() {
    let Some(mut s) = sidecar_or_skip() else { return };
    let tmp = tempfile::tempdir().unwrap();
    let pak_path = tmp.path().join("syn.pak");
    make_pak(
        &pak_path,
        &[("Game/Content/Foo.uasset", b"fakebytes"), ("Game/Content/Foo.uexp", b"fake")],
    );
    let targets = vec![StripTarget {
        asset_path: "Game/Content/DoesNotExist.uasset".into(),
        max_dim: 1024,
    }];
    let r = s
        .apply_strip_mips(&pak_path, &targets, Some("GAME_UE4_LATEST"), None)
        .expect("apply_strip_mips ok");
    assert_eq!(r.applied.len(), 0);
    assert_eq!(r.skipped.len(), 1);
    assert!(r.skipped[0].reason.contains("not found"));
}
