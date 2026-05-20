use super::*;
use std::fs::File;
use std::io::BufWriter;

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
