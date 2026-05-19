//! Detect pak chunks well above the recommended 1-2 GB sweet spot.
//!
//! UE chunking advice (and observed practice in well-cooked AAA): keep
//! individual pak chunks under ~2 GB so that patches stay small (only
//! the changed chunks need re-downloading). Wuthering Waves shipped
//! `pakchunk70-WindowsNoEditor.pak` at 27.6 GB — anything inside that
//! chunk that changes forces either a 27.6 GB re-download OR a
//! continuously-growing `_P.pak` overlay (the latter is what they chose,
//! see [`super::patch_overlay`]).
//!
//! This detector flags individual large paks; it's a structural finding,
//! not a "delete these bytes" finding. No `reclaimable_bytes` because the
//! fix is "re-chunk" which requires publisher cooperation.

use super::Detector;
use crate::report::format_bytes;
use crate::types::{Category, Evidence, Finding, Severity};
use std::path::Path;
use walkdir::WalkDir;

/// Above this size, a pak file is considered oversized.
const WARNING_THRESHOLD_BYTES: u64 = 2 * 1024 * 1024 * 1024;
/// Above this size, a pak file is considered a structural failure.
const CRITICAL_THRESHOLD_BYTES: u64 = 10 * 1024 * 1024 * 1024;

#[derive(Debug, Default)]
pub struct LargeChunkDetector;

impl Detector for LargeChunkDetector {
    fn name(&self) -> &'static str {
        "large_chunk"
    }

    fn run(&self, root: &Path) -> anyhow::Result<Vec<Finding>> {
        let large = find_large_paks(root);
        if large.is_empty() {
            return Ok(vec![]);
        }
        Ok(vec![build_finding(root, large)])
    }
}

#[derive(Debug)]
struct LargePak {
    rel_path: std::path::PathBuf,
    size_bytes: u64,
}

fn find_large_paks(root: &Path) -> Vec<LargePak> {
    let mut out = Vec::new();
    for entry in WalkDir::new(root)
        .follow_links(false)
        .into_iter()
        .filter_map(|e| e.ok())
    {
        if !entry.file_type().is_file() {
            continue;
        }
        if entry.path().extension().and_then(|s| s.to_str()) != Some("pak") {
            continue;
        }
        let size = match entry.metadata() {
            Ok(md) => md.len(),
            Err(_) => continue,
        };
        if size < WARNING_THRESHOLD_BYTES {
            continue;
        }
        let path = entry.path().to_path_buf();
        let rel_path = path.strip_prefix(root).unwrap_or(&path).to_path_buf();
        out.push(LargePak {
            rel_path,
            size_bytes: size,
        });
    }
    out.sort_by(|a, b| b.size_bytes.cmp(&a.size_bytes));
    out
}

fn build_finding(_root: &Path, large: Vec<LargePak>) -> Finding {
    let max_size = large.iter().map(|p| p.size_bytes).max().unwrap_or(0);
    let total_size: u64 = large.iter().map(|p| p.size_bytes).sum();
    let crit_count = large
        .iter()
        .filter(|p| p.size_bytes >= CRITICAL_THRESHOLD_BYTES)
        .count();

    let severity = if max_size >= CRITICAL_THRESHOLD_BYTES {
        Severity::Critical
    } else {
        Severity::Warning
    };

    let evidence: Vec<Evidence> = large
        .iter()
        .map(|p| {
            let cmp = if p.size_bytes >= CRITICAL_THRESHOLD_BYTES {
                "critical"
            } else {
                "oversized"
            };
            Evidence {
                path: p.rel_path.clone(),
                size_bytes: p.size_bytes,
                note: Some(cmp.to_string()),
            }
        })
        .collect();

    let title = format!(
        "Oversized pak chunks: {} pak(s) above {}, largest {}",
        large.len(),
        format_bytes(WARNING_THRESHOLD_BYTES),
        format_bytes(max_size),
    );

    let summary = format!(
        "Found {} pak file(s) above the {} chunk-size guidance, including \
         {} at or above the {} critical threshold. Large monolithic chunks \
         force every patch touching their contents to either re-download \
         the whole chunk or ship as a `_P.pak` overlay (the latter \
         compounds storage debt — see the patch_overlay finding). Total \
         oversized bytes: {}.",
        large.len(),
        format_bytes(WARNING_THRESHOLD_BYTES),
        crit_count,
        format_bytes(CRITICAL_THRESHOLD_BYTES),
        format_bytes(total_size),
    );

    let recommendation = "Re-chunk in the next major cook pass — target \
         1-2 GB per chunk, with co-loaded assets clustered together (so a \
         single patch typically touches one chunk, not many). This is a \
         publisher-side change; third-party tools cannot safely re-chunk \
         shipped paks because the AES keys + integrity hashes would break."
        .to_string();

    Finding {
        detector: "large_chunk".to_string(),
        category: Category::LargeChunk,
        severity,
        title,
        summary,
        evidence,
        // No direct byte savings from this finding alone — the win comes
        // from preventing future overlay accumulation. Don't double-count
        // with patch_overlay's estimate.
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

    fn write_pak(path: &Path, size_bytes: u64) {
        if let Some(p) = path.parent() {
            fs::create_dir_all(p).unwrap();
        }
        // For multi-GB tests we use `set_len` (sparse file) to avoid allocating
        // actual gigabytes on disk during testing. Detector reads metadata().len().
        let f = fs::File::create(path).unwrap();
        f.set_len(size_bytes).unwrap();
    }

    fn small_pak(path: &Path, bytes: u64) {
        if let Some(p) = path.parent() {
            fs::create_dir_all(p).unwrap();
        }
        fs::File::create(path)
            .unwrap()
            .write_all(&vec![0u8; bytes as usize])
            .unwrap();
    }

    #[test]
    fn no_finding_when_paks_are_small() {
        let tmp = TempDir::new().unwrap();
        small_pak(&tmp.path().join("Content/Paks/pak0.pak"), 1024);
        small_pak(&tmp.path().join("Content/Paks/pak1.pak"), 1024);

        let d = LargeChunkDetector;
        let findings = d.run(tmp.path()).unwrap();
        assert!(findings.is_empty());
    }

    #[test]
    fn flags_oversized_pak_as_warning() {
        let tmp = TempDir::new().unwrap();
        // 3 GB sparse pak: above WARNING but below CRITICAL.
        write_pak(
            &tmp.path().join("Content/Paks/pakchunk5.pak"),
            3 * 1024 * 1024 * 1024,
        );
        small_pak(&tmp.path().join("Content/Paks/pakchunk0.pak"), 1024);

        let d = LargeChunkDetector;
        let findings = d.run(tmp.path()).unwrap();
        assert_eq!(findings.len(), 1);
        let f = &findings[0];
        assert_eq!(f.severity, Severity::Warning);
        assert_eq!(f.evidence.len(), 1);
        assert!(f.title.contains("3.00 GB"));
    }

    #[test]
    fn flags_critical_when_pak_is_huge() {
        let tmp = TempDir::new().unwrap();
        // 12 GB sparse pak: above CRITICAL_THRESHOLD_BYTES.
        write_pak(
            &tmp.path().join("Content/Paks/pakchunk70.pak"),
            12 * 1024 * 1024 * 1024,
        );

        let d = LargeChunkDetector;
        let findings = d.run(tmp.path()).unwrap();
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].severity, Severity::Critical);
        assert_eq!(findings[0].reclaimable_bytes, None);
    }

    #[test]
    fn sorts_evidence_by_size_descending() {
        let tmp = TempDir::new().unwrap();
        write_pak(
            &tmp.path().join("Content/Paks/pakchunk5.pak"),
            3 * 1024 * 1024 * 1024,
        );
        write_pak(
            &tmp.path().join("Content/Paks/pakchunk70.pak"),
            12 * 1024 * 1024 * 1024,
        );
        write_pak(
            &tmp.path().join("Content/Paks/pakchunk1.pak"),
            5 * 1024 * 1024 * 1024,
        );

        let d = LargeChunkDetector;
        let findings = d.run(tmp.path()).unwrap();
        let f = &findings[0];
        // Largest first.
        assert!(f.evidence[0].path.to_string_lossy().contains("pakchunk70"));
        assert!(f.evidence[1].path.to_string_lossy().contains("pakchunk1"));
        assert!(f.evidence[2].path.to_string_lossy().contains("pakchunk5"));
    }
}
