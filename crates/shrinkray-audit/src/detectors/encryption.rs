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
    iostore_count: usize,
    iostore_bytes: u64,
    unreadable_count: usize,
    unreadable_bytes: u64,
    encrypted_examples: Vec<(PathBuf, u64)>,
    iostore_examples: Vec<(PathBuf, u64)>,
}

/// A `.pak` shipped next to a `.utoc` + `.ucas` pair is an IoStore stub —
/// the actual content lives in the `.ucas` container. repak can't read these,
/// so they're functionally locked out for shrinkray's content-level ops
/// until Phase 2 ships retoc integration.
fn is_iostore_stub(pak_path: &Path) -> bool {
    let utoc = pak_path.with_extension("utoc");
    let ucas = pak_path.with_extension("ucas");
    utoc.is_file() && ucas.is_file()
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

        // IoStore takes priority over pak-header classification: the .pak file
        // alongside a .utoc/.ucas pair is a stub whose header may parse as
        // anything (Readable, Unreadable, even Encrypted if AES-protected),
        // but the real content is unreachable via repak either way.
        if is_iostore_stub(path) {
            s.iostore_count += 1;
            s.iostore_bytes = s.iostore_bytes.saturating_add(size);
            if s.iostore_examples.len() < 5 {
                let rel = path.strip_prefix(root).unwrap_or(path).to_path_buf();
                s.iostore_examples.push((rel, size));
            }
            continue;
        }

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

#[derive(Debug, Clone, Copy)]
enum BlockReason {
    Encrypted,
    IoStore,
    Signed,
    Unreadable,
}

fn dominant_block_reason(s: &ScanStats) -> BlockReason {
    let mut best = (BlockReason::Unreadable, s.unreadable_bytes);
    if s.signed_bytes > best.1 {
        best = (BlockReason::Signed, s.signed_bytes);
    }
    if s.iostore_bytes > best.1 {
        best = (BlockReason::IoStore, s.iostore_bytes);
    }
    if s.encrypted_bytes > best.1 {
        best = (BlockReason::Encrypted, s.encrypted_bytes);
    }
    best.0
}

fn blocked_recommendation(_s: &ScanStats, reason: BlockReason) -> String {
    match reason {
        BlockReason::Encrypted => {
            "Every pak is encrypted. Content-level optimization is impossible \
             without the AES key, and modifying encrypted paks would break the \
             anti-cheat / integrity check. shrinkray on this install is limited \
             to filesystem-level garbage collection (stale dirs, launcher \
             leftovers). Phase 2 will add --aes-key + keys.json support for \
             unlocked installs."
                .to_string()
        }
        BlockReason::IoStore => {
            "Every pak ships as an IoStore container (.utoc + .ucas pair). \
             repak cannot read these — shrinkray's strip + recompress ops are \
             unavailable until Phase 2 (retoc integration). Only filesystem-level \
             audit findings (stale dirs, editor leftovers, launcher satellites) \
             apply on this install."
                .to_string()
        }
        BlockReason::Signed => {
            "Every pak is signed (.sig sibling present). Modifying a signed pak \
             guarantees the game refuses to start. shrinkray is limited to \
             filesystem-level cleanup; no in-pak surgery is safe here."
                .to_string()
        }
        BlockReason::Unreadable => {
            "Every pak has a header repak cannot parse (unknown version or \
             non-standard format). shrinkray's content-level ops require a \
             readable index, so this install is limited to filesystem-level \
             cleanup until repak upstream adds support."
                .to_string()
        }
    }
}

fn build_finding(_root: &Path, s: ScanStats) -> Finding {
    // "Blocked" = paks shrinkray can't reach with current content-level ops.
    // Encryption, IoStore, signing, and unknown-format unreadables all block.
    let blocked_count = s.encrypted_count
        + s.iostore_count
        + s.signed_count
        + s.unreadable_count;
    let blocked_bytes = s
        .encrypted_bytes
        .saturating_add(s.iostore_bytes)
        .saturating_add(s.signed_bytes)
        .saturating_add(s.unreadable_bytes);
    let blocked_pct = if s.total_bytes > 0 {
        (blocked_bytes as f64 / s.total_bytes as f64) * 100.0
    } else {
        0.0
    };

    // Each blocking reason gets its own escalation; we pick the dominant
    // reason when multiple are present, then encode severity by how much of
    // the install is reachable to content-level ops.
    let dominant_reason = dominant_block_reason(&s);

    let (severity, title, recommendation) = if blocked_count == 0 {
        (
            Severity::Info,
            format!(
                "Pak access: clear ({} pak(s) all readable to repak)",
                s.total_paks
            ),
            "No encryption, IoStore, or signing blocks downstream optimization. \
             shrinkray's strip + recompress operations are available on every pak."
                .to_string(),
        )
    } else if blocked_count == s.total_paks {
        (
            Severity::Critical,
            match dominant_reason {
                BlockReason::Encrypted => format!(
                    "Pak access: blocked ({} pak(s), {} all AES-encrypted)",
                    s.total_paks,
                    format_bytes(s.total_bytes)
                ),
                BlockReason::IoStore => format!(
                    "Pak access: blocked ({} pak(s) all IoStore — Phase 2 retoc needed)",
                    s.total_paks
                ),
                BlockReason::Signed => format!(
                    "Pak access: blocked ({} pak(s) all signed — modifying breaks integrity check)",
                    s.total_paks
                ),
                BlockReason::Unreadable => format!(
                    "Pak access: blocked ({} pak(s) unreadable — unknown format/version)",
                    s.total_paks
                ),
            },
            blocked_recommendation(&s, dominant_reason),
        )
    } else {
        (
            Severity::Warning,
            format!(
                "Pak access: partial ({:.0}% of pak bytes blocked, {} reachable)",
                blocked_pct,
                format_bytes(s.readable_bytes)
            ),
            format!(
                "{} reachable pak(s) totalling {} are available for shrinkray's \
                 strip + recompress ops. The remaining {} pak(s) are blocked: \
                 {} encrypted, {} IoStore, {} signed, {} unreadable. Target the \
                 reachable subset first; the rest needs Phase 2 (retoc / .NET \
                 sidecar / AES key) or is out of scope entirely.",
                s.readable_count,
                format_bytes(s.readable_bytes),
                blocked_count,
                s.encrypted_count,
                s.iostore_count,
                s.signed_count,
                s.unreadable_count,
            ),
        )
    };

    let summary = format!(
        "Scanned {} pak file(s) totalling {}: \
         {} readable, {} signed, {} encrypted, {} IoStore stub(s), {} unreadable. \
         IoStore stubs are .pak files paired with .utoc/.ucas containers — the \
         real content sits in the .ucas, which repak cannot read. Encryption, \
         signing, and IoStore each independently lock shrinkray and every other \
         third-party tool out of content-level optimization on the affected paks.",
        s.total_paks,
        format_bytes(s.total_bytes),
        s.readable_count,
        s.signed_count,
        s.encrypted_count,
        s.iostore_count,
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
        .chain(s.iostore_examples.into_iter().map(|(path, size)| Evidence {
            path,
            size_bytes: size,
            note: Some("IoStore stub (.utoc/.ucas pair)".to_string()),
        }))
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

    fn write_iostore_triple(stem_no_ext: &Path, pak_bytes: u64) {
        write_garbage_pak(&stem_no_ext.with_extension("pak"), pak_bytes);
        // Real IoStore .utoc/.ucas would have valid headers; for the
        // detector we only need the sibling files to exist.
        fs::write(stem_no_ext.with_extension("utoc"), b"utoc").unwrap();
        fs::write(stem_no_ext.with_extension("ucas"), b"ucas").unwrap();
    }

    #[test]
    fn signed_pak_is_blocking_not_info() {
        // A 100%-signed install is unreachable for content-level surgery:
        // modifying a signed pak guarantees a broken launch. Should escalate
        // past Info even though no encryption is present.
        let tmp = TempDir::new().unwrap();
        write_signed_pair(&tmp.path().join("Content/Paks/pakchunk0.pak"), 1024);

        let d = EncryptionDetector;
        let findings = d.run(tmp.path()).unwrap();
        assert_eq!(findings.len(), 1);
        let f = &findings[0];
        assert_eq!(f.severity, Severity::Critical, "100% signed = blocked");
        assert!(
            f.title.contains("signed"),
            "title should name the dominant block reason, got: {}",
            f.title
        );
    }

    #[test]
    fn fully_unreadable_install_is_blocking_not_info() {
        // Old behavior treated unreadable paks as "still Info" because no
        // encryption was present. New behavior: any 100%-blocked install is
        // a Critical finding, with the dominant reason in the title.
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
        assert_eq!(f.severity, Severity::Critical);
        assert!(f.summary.contains("3 unreadable"));
        assert!(
            f.title.contains("unreadable"),
            "title should name unreadable as dominant block reason, got: {}",
            f.title
        );
    }

    #[test]
    fn iostore_triple_is_detected_and_dominant() {
        // The Stellar Blade case: 7 IoStore-paired chunks (each pak has
        // .utoc + .ucas siblings). Should classify as IoStore, escalate to
        // Critical (100% blocked), and recommend Phase 2 retoc.
        let tmp = TempDir::new().unwrap();
        for i in 0..7 {
            write_iostore_triple(
                &tmp.path().join(format!("Content/Paks/pakchunk{}", i)),
                2048,
            );
        }

        let d = EncryptionDetector;
        let findings = d.run(tmp.path()).unwrap();
        assert_eq!(findings.len(), 1);
        let f = &findings[0];
        assert_eq!(
            f.severity,
            Severity::Critical,
            "100% IoStore = fully blocked"
        );
        assert!(
            f.title.contains("IoStore"),
            "title should name IoStore as dominant, got: {}",
            f.title
        );
        assert!(
            f.summary.contains("7 IoStore stub(s)"),
            "summary should count IoStore stubs, got: {}",
            f.summary
        );
        assert!(
            f.recommendation.contains("retoc"),
            "recommendation should point at Phase 2 retoc"
        );
        // Evidence list should include the IoStore stubs (capped at 5).
        assert_eq!(f.evidence.len(), 5);
        assert!(f
            .evidence
            .iter()
            .all(|e| e.note.as_deref() == Some("IoStore stub (.utoc/.ucas pair)")));
    }

    #[test]
    fn iostore_overrides_pak_header_classification() {
        // A .pak with .utoc/.ucas siblings should classify as IoStore even
        // if the .pak header itself would otherwise parse as Readable or
        // Unreadable. This is the key correctness property: IoStore is a
        // disk-layout fact, not a header fact.
        let tmp = TempDir::new().unwrap();
        // Two IoStore chunks (paired with utoc/ucas).
        write_iostore_triple(&tmp.path().join("Content/Paks/pakchunk0"), 1024);
        write_iostore_triple(&tmp.path().join("Content/Paks/pakchunk1"), 1024);
        // One plain garbage pak (no siblings) — should classify as Unreadable.
        write_garbage_pak(&tmp.path().join("Content/Paks/loose.pak"), 1024);

        let d = EncryptionDetector;
        let findings = d.run(tmp.path()).unwrap();
        let f = &findings[0];
        assert!(
            f.summary.contains("2 IoStore stub(s)") && f.summary.contains("1 unreadable"),
            "expected 2 IoStore + 1 unreadable, got: {}",
            f.summary
        );
    }
}
