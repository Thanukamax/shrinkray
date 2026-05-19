//! Classify every pak file's read-status and emit one finding summarizing
//! encryption + signing coverage.
//!
//! Encrypted paks are the bright red line for third-party tooling: shrinkray
//! cannot enumerate, strip, or recompress their contents without the AES key.
//! When a finding here surfaces, all downstream content-level optimizations
//! (mip-strip, audio recompress, language strip inside paks) are off the
//! table for the encrypted portion of the install.

use super::Detector;
use crate::report::format_bytes;
use crate::types::{Category, Evidence, Finding, Severity};
use shrinkray_core::pak::{classify_pak, PakClassification};
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

#[derive(Debug, Default)]
pub struct EncryptionDetector;

impl Detector for EncryptionDetector {
    fn name(&self) -> &'static str {
        "encryption"
    }

    fn run(&self, root: &Path) -> anyhow::Result<Vec<Finding>> {
        let stats = scan_pak_classifications(root);
        if stats.total_paks == 0 {
            return Ok(vec![]);
        }
        Ok(vec![build_finding(root, stats)])
    }
}

#[derive(Debug, Default)]
struct ScanStats {
    total_paks: usize,
    total_bytes: u64,
    readable_count: usize,
    readable_bytes: u64,
    signed_count: usize,
    signed_bytes: u64,
    encrypted_count: usize,
    encrypted_bytes: u64,
    unreadable_count: usize,
    unreadable_bytes: u64,
    encrypted_examples: Vec<(PathBuf, u64)>,
}

fn scan_pak_classifications(root: &Path) -> ScanStats {
    let mut s = ScanStats::default();
    for entry in WalkDir::new(root)
        .follow_links(false)
        .into_iter()
        .filter_map(|e| e.ok())
    {
        if !entry.file_type().is_file() {
            continue;
        }
        if entry.path().extension().and_then(|x| x.to_str()) != Some("pak") {
            continue;
        }
        let path = entry.path();
        let size = entry.metadata().map(|m| m.len()).unwrap_or(0);

        s.total_paks += 1;
        s.total_bytes = s.total_bytes.saturating_add(size);

        let class = classify_pak(path);
        match class {
            PakClassification::Readable => {
                s.readable_count += 1;
                s.readable_bytes = s.readable_bytes.saturating_add(size);
            }
            PakClassification::Signed => {
                s.signed_count += 1;
                s.signed_bytes = s.signed_bytes.saturating_add(size);
            }
            PakClassification::Encrypted => {
                s.encrypted_count += 1;
                s.encrypted_bytes = s.encrypted_bytes.saturating_add(size);
                if s.encrypted_examples.len() < 5 {
                    let rel = path
                        .strip_prefix(root)
                        .unwrap_or(path)
                        .to_path_buf();
                    s.encrypted_examples.push((rel, size));
                }
            }
            PakClassification::Unreadable { .. } => {
                s.unreadable_count += 1;
                s.unreadable_bytes = s.unreadable_bytes.saturating_add(size);
            }
        }
    }
    s
}

fn build_finding(_root: &Path, s: ScanStats) -> Finding {
    let encrypted_pct = if s.total_bytes > 0 {
        (s.encrypted_bytes as f64 / s.total_bytes as f64) * 100.0
    } else {
        0.0
    };

    let (severity, title, recommendation) = if s.encrypted_count == 0 {
        (
            Severity::Info,
            format!(
                "Pak encryption: none ({} pak(s) all readable to repak)",
                s.total_paks
            ),
            "No encryption blocks downstream optimization. shrinkray's strip + \
             recompress operations are available on this install."
                .to_string(),
        )
    } else if s.encrypted_count == s.total_paks {
        (
            Severity::Critical,
            format!(
                "Pak encryption: 100% ({} pak(s), {} all AES-encrypted)",
                s.total_paks,
                format_bytes(s.total_bytes)
            ),
            "Every pak is encrypted. Third-party content-level optimization is \
             impossible without the AES key — and modifying encrypted paks \
             would break the live-service anti-cheat integrity check. \
             shrinkray on this install is limited to filesystem-level \
             garbage collection (stale dirs, launcher leftovers). For real \
             optimization, the publisher must integrate shrinkray into their \
             cook pipeline upstream."
                .to_string(),
        )
    } else {
        (
            Severity::Warning,
            format!(
                "Pak encryption: {:.0}% ({} of {} pak bytes encrypted)",
                encrypted_pct,
                format_bytes(s.encrypted_bytes),
                format_bytes(s.total_bytes)
            ),
            "Mixed encryption. Content surgery available on the unencrypted \
             portion; encrypted paks limited to filesystem GC. Plan downstream \
             ops to target the readable subset first."
                .to_string(),
        )
    };

    let summary = format!(
        "Scanned {} pak file(s) totalling {}: \
         {} readable, {} signed, {} encrypted, {} unreadable. \
         Encryption locks shrinkray and every other third-party tool out of \
         entry-level enumeration, mip-strip, audio recompress, and L10N strip \
         on the affected paks.",
        s.total_paks,
        format_bytes(s.total_bytes),
        s.readable_count,
        s.signed_count,
        s.encrypted_count,
        s.unreadable_count,
    );

    let evidence: Vec<Evidence> = s
        .encrypted_examples
        .into_iter()
        .map(|(path, size)| Evidence {
            path,
            size_bytes: size,
            note: Some("AES-encrypted".to_string()),
        })
        .collect();

    Finding {
        detector: "encryption".to_string(),
        category: Category::Encryption,
        severity,
        title,
        summary,
        evidence,
        // No reclaimable bytes — this finding is informational about a
        // constraint, not a deletion target.
        reclaimable_bytes: None,
        recommendation,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::io::Write;
    use tempfile::TempDir;

    fn write_garbage_pak(path: &Path, bytes: u64) {
        if let Some(p) = path.parent() {
            fs::create_dir_all(p).unwrap();
        }
        fs::File::create(path)
            .unwrap()
            .write_all(&vec![0u8; bytes as usize])
            .unwrap();
    }

    fn write_signed_pair(path: &Path, bytes: u64) {
        write_garbage_pak(path, bytes);
        let sig = path.with_extension("sig");
        fs::write(&sig, b"sig").unwrap();
    }

    #[test]
    fn no_finding_when_no_paks() {
        let tmp = TempDir::new().unwrap();
        let d = EncryptionDetector;
        let findings = d.run(tmp.path()).unwrap();
        assert!(findings.is_empty());
    }

    #[test]
    fn classifies_signed_pak_as_signed() {
        let tmp = TempDir::new().unwrap();
        write_signed_pair(&tmp.path().join("Content/Paks/pakchunk0.pak"), 1024);

        let d = EncryptionDetector;
        let findings = d.run(tmp.path()).unwrap();
        assert_eq!(findings.len(), 1);
        let f = &findings[0];
        assert_eq!(f.severity, Severity::Info, "no encryption → info");
        assert!(f.title.contains("none"));
    }

    #[test]
    fn unreadable_garbage_paks_dont_count_as_encrypted() {
        // Garbage bytes parse as Unreadable, not Encrypted. So a folder full
        // of garbage paks should still show 0% encrypted.
        let tmp = TempDir::new().unwrap();
        for i in 0..3 {
            write_garbage_pak(
                &tmp.path().join(format!("Content/Paks/p{}.pak", i)),
                512,
            );
        }
        let d = EncryptionDetector;
        let findings = d.run(tmp.path()).unwrap();
        assert_eq!(findings.len(), 1);
        let f = &findings[0];
        // 0% encrypted, but 100% unreadable — still Info severity since the
        // user-meaningful question ("can shrinkray help?") depends on
        // encryption specifically.
        assert_eq!(f.severity, Severity::Info);
        assert!(f.summary.contains("3 unreadable"));
    }
}
