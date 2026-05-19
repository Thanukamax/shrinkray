//! shrinkray-audit — read-only bloat audit.
//!
//! Walks a UE game install and surfaces structural inefficiencies that
//! don't require pak content access. The audit never writes. Output is an
//! [`AuditReport`] serializable to JSON (via serde) or Markdown (via
//! [`render_markdown`]).

pub mod detectors;
pub mod report;
pub mod types;

pub use detectors::Detector;
pub use report::{format_bytes, render_markdown};
pub use types::{Aggregate, AuditMeta, AuditReport, Category, Evidence, Finding, Severity};

use std::path::{Path, PathBuf};
use walkdir::WalkDir;

/// Default detector roster. Order is stable so report grouping is deterministic.
pub fn default_detectors() -> Vec<Box<dyn Detector>> {
    vec![
        Box::new(detectors::patch_overlay::PatchOverlayDetector),
        Box::new(detectors::stale_versions::StaleVersionDirDetector),
        Box::new(detectors::sharded_videos::ShardedVideosDetector),
        Box::new(detectors::large_chunks::LargeChunkDetector),
        Box::new(detectors::encryption::EncryptionDetector),
        Box::new(detectors::editor_leftovers::EditorLeftoverDetector),
    ]
}

/// Audit the given root with the default detector roster. The audit never
/// writes to disk.
pub fn audit(root: &Path) -> anyhow::Result<AuditReport> {
    audit_with(root, default_detectors())
}

/// Audit with an explicit detector list (tests, future tuning).
pub fn audit_with(root: &Path, detectors: Vec<Box<dyn Detector>>) -> anyhow::Result<AuditReport> {
    let total = total_size_bytes(root)?;
    let mut findings = Vec::new();
    let mut names = Vec::with_capacity(detectors.len());
    for d in &detectors {
        let name = d.name();
        names.push(name.to_string());
        match d.run(root) {
            Ok(mut fs) => findings.append(&mut fs),
            Err(err) => findings.push(detector_error_finding(name, err)),
        }
    }
    Ok(AuditReport::assemble(
        PathBuf::from(root),
        total,
        findings,
        names,
    ))
}

/// Sum of file sizes under `root`. Skips files we can't stat (broken
/// symlinks, permission errors) silently — they don't contribute to total.
fn total_size_bytes(root: &Path) -> anyhow::Result<u64> {
    let mut total: u64 = 0;
    for entry in WalkDir::new(root)
        .follow_links(false)
        .into_iter()
        .filter_map(|e| e.ok())
    {
        if entry.file_type().is_file() {
            if let Ok(md) = entry.metadata() {
                total = total.saturating_add(md.len());
            }
        }
    }
    Ok(total)
}

fn detector_error_finding(detector: &str, err: anyhow::Error) -> Finding {
    Finding {
        detector: detector.to_string(),
        category: Category::ChunkingQuality, // generic; meta-finding
        severity: Severity::Warning,
        title: format!("Detector `{}` failed", detector),
        summary: format!(
            "This detector errored while running. The rest of the audit \
             completed normally, but its findings are missing. Error: {}",
            err
        ),
        evidence: vec![],
        reclaimable_bytes: None,
        recommendation:
            "Re-run the audit. If the error persists, file an issue with the audit log."
                .to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::io::Write;
    use tempfile::TempDir;

    #[test]
    fn audit_empty_dir_is_clean() {
        let tmp = TempDir::new().unwrap();
        let r = audit(tmp.path()).unwrap();
        assert_eq!(r.total_size_bytes, 0);
        assert_eq!(r.aggregate.bloat_score, 0);
        assert!(r.findings.is_empty());
    }

    #[test]
    fn audit_picks_up_patch_overlay() {
        let tmp = TempDir::new().unwrap();
        let paks = tmp.path().join("Content/Paks");
        let overlays = tmp.path().join("Saved/Resources/3.3.0/Resource/3.3.11");
        fs::create_dir_all(&paks).unwrap();
        fs::create_dir_all(&overlays).unwrap();
        fs::File::create(paks.join("pakchunk0-WindowsNoEditor.pak"))
            .unwrap()
            .write_all(&vec![0u8; 4096])
            .unwrap();
        fs::File::create(overlays.join("pakchunk0-WindowsNoEditor_P.pak"))
            .unwrap()
            .write_all(&vec![0u8; 2048])
            .unwrap();

        let r = audit(tmp.path()).unwrap();
        let overlay_findings: Vec<&Finding> = r
            .findings
            .iter()
            .filter(|f| f.category == Category::PatchOverlay)
            .collect();
        assert_eq!(overlay_findings.len(), 1);
        assert_eq!(r.aggregate.total_reclaimable_bytes, 1024);
        assert!(
            r.meta.detectors.contains(&"patch_overlay".to_string()),
            "detector name surfaces in metadata"
        );
    }

    #[test]
    fn total_size_sums_files() {
        let tmp = TempDir::new().unwrap();
        let a = tmp.path().join("a.bin");
        let b = tmp.path().join("nested/b.bin");
        fs::create_dir_all(b.parent().unwrap()).unwrap();
        fs::File::create(&a).unwrap().write_all(&[0u8; 100]).unwrap();
        fs::File::create(&b)
            .unwrap()
            .write_all(&[0u8; 250])
            .unwrap();
        assert_eq!(total_size_bytes(tmp.path()).unwrap(), 350);
    }
}
