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
        Box::new(detectors::launcher_satellite::LauncherSatelliteDetector),
        Box::new(detectors::shader_rhi_redundancy::ShaderRhiRedundancyDetector),
        Box::new(detectors::redist_installer::RedistInstallerDetector),
        Box::new(detectors::platform_siblings::PlatformSiblingsDetector),
        Box::new(detectors::mod_manager_artifacts::ModManagerArtifactsDetector),
        Box::new(detectors::duplicate_content::DuplicateContentDetector),
        Box::new(detectors::cef_locales::CefLocalesDetector),
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

    /// End-to-end integration: build a fixture shaped like a real Wuthering
    /// Waves install (patch overlays + stale version dirs + sharded video
    /// paks + oversized chunk + launcher satellites + editor leftovers) and
    /// verify the full audit pipeline produces a coherent multi-detector
    /// report. This is the regression gate for cross-detector wiring; if
    /// adding a new detector breaks orchestration this test catches it
    /// before the per-detector unit tests have a chance.
    #[test]
    fn integration_wuwa_shaped_fixture() {
        use std::io::Write;
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();

        // 1. Base paks + patch overlays — exercises patch_overlay detector.
        //    Overlay ratio is intentionally ≥40% so the finding crosses the
        //    Critical threshold (matches the real WuWa chunk 0 ratio).
        let paks = root.join("Client/Content/Paks");
        let overlays = root.join("Client/Saved/Resources/3.3.0/Resource/3.3.11");
        fs::create_dir_all(&paks).unwrap();
        fs::create_dir_all(&overlays).unwrap();
        for chunk in [0, 5, 7, 53] {
            let base = paks.join(format!("pakchunk{}-WindowsNoEditor.pak", chunk));
            fs::File::create(&base)
                .unwrap()
                .set_len(8 * 1024 * 1024)
                .unwrap();
            let patch = overlays.join(format!("pakchunk{}-WindowsNoEditor_P.pak", chunk));
            fs::File::create(&patch)
                .unwrap()
                .set_len(5 * 1024 * 1024) // 62.5% overlay → Critical
                .unwrap();
        }

        // 2. Critically-oversized monolithic chunk (mirrors WuWa's 27 GB
        //    pakchunk70). Sparse 12 GB so it crosses the 10 GB critical
        //    threshold without consuming disk.
        fs::File::create(paks.join("pakchunk70-WindowsNoEditor.pak"))
            .unwrap()
            .set_len(12 * 1024 * 1024 * 1024)
            .unwrap();

        // 3. Stale version directories — stale_versions detector.
        fs::create_dir_all(root.join("launcherDownload/3.1.0")).unwrap();
        fs::create_dir_all(root.join("launcherDownload/3.2.2")).unwrap();
        fs::create_dir_all(root.join("launcherDownload/3.3.0")).unwrap();
        for v in ["3.1.0", "3.2.2", "3.3.0"] {
            fs::File::create(root.join(format!("launcherDownload/{}/m.json", v)))
                .unwrap()
                .write_all(&[0u8; 50 * 1024 * 1024])
                .unwrap();
        }

        // 4. Sharded video paks — sharded_videos detector.
        let video_paks = root.join("Client/Saved/Resources/Video/Paks");
        for i in 0..25 {
            let d = video_paks.join(format!("{}_0", 100 + i));
            fs::create_dir_all(&d).unwrap();
            fs::File::create(d.join(format!("Video_{}_0-WindowsNoEditor.pak", 100 + i)))
                .unwrap()
                .set_len(50 * 1024 * 1024)
                .unwrap();
        }

        // 5. Launcher satellite assemblies — launcher_satellite detector.
        let launcher = root.join("2.6.1.0");
        for lang in ["cs", "de", "es", "fr", "it", "ja", "ko"] {
            fs::create_dir_all(launcher.join(lang)).unwrap();
            fs::File::create(launcher.join(format!("{}/Launcher.resources.dll", lang)))
                .unwrap()
                .write_all(&[0u8; 2 * 1024 * 1024])
                .unwrap();
        }

        // 6. Editor leftover (.pdb in binaries dir) — editor_leftovers detector.
        let bin = root.join("Client/Binaries/Win64");
        fs::create_dir_all(&bin).unwrap();
        fs::File::create(bin.join("game.pdb"))
            .unwrap()
            .write_all(&[0u8; 5 * 1024 * 1024])
            .unwrap();

        // 7. (No signed/encrypted paks in this fixture — encryption detector
        //    surfaces an Info finding.)

        // Run the audit and inspect the report.
        let r = audit(root).expect("audit succeeds");
        assert!(r.findings.len() >= 6, "expected ≥6 findings, got {}", r.findings.len());

        let categories: std::collections::BTreeSet<_> =
            r.findings.iter().map(|f| f.category).collect();
        assert!(categories.contains(&Category::PatchOverlay));
        assert!(categories.contains(&Category::LargeChunk));
        assert!(categories.contains(&Category::StaleVersionDir));
        assert!(categories.contains(&Category::ShardedVideos));
        assert!(categories.contains(&Category::LauncherSatellite));
        assert!(categories.contains(&Category::EditorLeftovers));
        assert!(categories.contains(&Category::Encryption));

        // Bloat score should be meaningfully elevated. We hit Warning across
        // most categories + Critical on large_chunk (3 GB pak above 2 GB
        // threshold but below 10 GB so still Warning, but the 3 GB sparse
        // file dominates the size signal and the per-cat reclaimable sums).
        assert!(
            r.aggregate.bloat_score >= 20,
            "expected meaningful bloat score, got {} (findings: {})",
            r.aggregate.bloat_score,
            r.findings
                .iter()
                .map(|f| format!("[{}] {}", f.severity.label(), f.title))
                .collect::<Vec<_>>()
                .join(", ")
        );

        // Markdown rendering doesn't panic on a real-shaped report.
        let md = render_markdown(&r);
        assert!(md.contains("# Bloat Audit"));
        assert!(md.contains("Bloat score"));

        // JSON serialization round-trips.
        let json = serde_json::to_string(&r).unwrap();
        let _back: AuditReport = serde_json::from_str(&json).unwrap();
    }
}
