//! UE pak file classification.
//!
//! Step 1 only inspects paks. Trimming (re-emit with dropped entries) lands in
//! Step 3 and uses `repak`'s reader + writer; encryption write and compression
//! write are not supported by repak 0.2.x — see `research/A_pak_iostore.md`.

use serde::Serialize;
use std::path::Path;

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "status", rename_all = "lowercase")]
pub enum PakClassification {
    /// repak opened the index successfully — safe to trim in Step 3.
    Readable,
    /// `.sig` sibling present — the game verifies the pak hash at launch.
    /// Modifying a signed pak guarantees the game refuses to start.
    Signed,
    /// AES-encrypted; needs a user-supplied key (Phase 2: `--aes-key` flag).
    Encrypted,
    /// Header parsed but version/format is unknown to repak 0.2.x.
    Unreadable { reason: String },
}

pub fn classify_pak(path: &Path) -> PakClassification {
    if has_signature(path) {
        return PakClassification::Signed;
    }
    let file = match std::fs::File::open(path) {
        Ok(f) => f,
        Err(e) => {
            return PakClassification::Unreadable {
                reason: format!("open: {e}"),
            }
        }
    };
    let mut reader = std::io::BufReader::new(file);
    match repak::PakBuilder::new().reader(&mut reader) {
        Ok(_) => PakClassification::Readable,
        Err(e) => classify_error(&e.to_string()),
    }
}

fn has_signature(path: &Path) -> bool {
    path.with_extension("sig").exists()
}

/// Heuristic: repak's error variants have shifted between minor versions, so we
/// pattern-match against the Display string instead of the variant. The
/// `UnsupportedOrEncrypted` variant contains both words; in that ambiguous case
/// we surface the literal reason rather than falsely claim a key is missing.
fn classify_error(msg: &str) -> PakClassification {
    let lc = msg.to_ascii_lowercase();
    if lc.contains("unsupported") {
        return PakClassification::Unreadable {
            reason: msg.to_string(),
        };
    }
    if lc.contains("encrypt") || lc.contains("aes") {
        return PakClassification::Encrypted;
    }
    PakClassification::Unreadable {
        reason: msg.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_signed_pak_by_sig_sibling() {
        let tmp = tempfile::tempdir().unwrap();
        let pak_path = tmp.path().join("pakchunk0.pak");
        let sig_path = tmp.path().join("pakchunk0.sig");
        std::fs::write(&pak_path, b"junk").unwrap();
        std::fs::write(&sig_path, b"junk").unwrap();
        assert!(matches!(classify_pak(&pak_path), PakClassification::Signed));
    }

    #[test]
    fn classifies_missing_file_as_unreadable() {
        let tmp = tempfile::tempdir().unwrap();
        let pak_path = tmp.path().join("missing.pak");
        assert!(matches!(
            classify_pak(&pak_path),
            PakClassification::Unreadable { .. }
        ));
    }

    #[test]
    fn classifies_garbage_pak_as_unreadable() {
        let tmp = tempfile::tempdir().unwrap();
        let pak_path = tmp.path().join("garbage.pak");
        std::fs::write(&pak_path, b"this is not a pak").unwrap();
        let got = classify_pak(&pak_path);
        eprintln!("garbage classified as: {:?}", got);
        assert!(matches!(got, PakClassification::Unreadable { .. }));
    }

    #[test]
    fn classify_error_routes_encryption_messages() {
        assert!(matches!(
            classify_error("pak is encrypted but no key was provided"),
            PakClassification::Encrypted
        ));
        assert!(matches!(
            classify_error("expect 256 bit AES key as base64 or hex string"),
            PakClassification::Encrypted
        ));
        assert!(matches!(
            classify_error("enable the encryption feature to read encrypted paks"),
            PakClassification::Encrypted
        ));
    }

    #[test]
    fn classify_error_routes_other_messages_as_unreadable() {
        match classify_error("unsupported pak version 99") {
            PakClassification::Unreadable { reason } => assert!(reason.contains("99")),
            _ => panic!("expected Unreadable"),
        }
    }
}
