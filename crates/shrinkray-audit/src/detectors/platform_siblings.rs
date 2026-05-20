//! Detect multi-platform binary trees in a single install.
//!
//! UE projects can cook for multiple platforms simultaneously. A consumer
//! Windows download sometimes still ships `Engine/Binaries/Linux/` or
//! `<Game>/Binaries/Mac/` alongside the Win64 build. Those foreign-platform
//! binaries are pure ballast for that user.
//!
//! Conservative: only flags if at least two platform dirs exist AND the
//! non-largest platforms total >= 50 MB.

use super::Detector;
use crate::report::format_bytes;
use crate::types::{Category, Evidence, Finding, Severity};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

const PLATFORM_DIRS: &[&str] = &[
    "Win64",
    "Win32",
    "WinGDK",
    "Linux",
    "LinuxArm64",
    "LinuxAArch64",
    "Mac",
    "IOS",
    "TVOS",
    "Android",
    "Switch",
    "PS4",
    "PS5",
    "XSX",
    "XB1",
];

#[derive(Debug, Default)]
pub struct PlatformSiblingsDetector;

impl Detector for PlatformSiblingsDetector {
    fn name(&self) -> &'static str {
        "platform_siblings"
    }

    fn run(&self, root: &Path) -> anyhow::Result<Vec<Finding>> {
        let buckets = scan(root);
        if buckets.len() < 2 {
            return Ok(vec![]);
        }
        let total: u64 = buckets.values().map(|b| b.total_bytes).sum();
        let largest = buckets.values().map(|b| b.total_bytes).max().unwrap_or(0);
        let reclaimable = total.saturating_sub(largest);
        if reclaimable < 50 * 1024 * 1024 {
            return Ok(vec![]);
        }
        Ok(vec![build_finding(buckets, reclaimable, largest)])
    }
}

#[derive(Debug, Default)]
struct PlatformBucket {
    name: &'static str,
    /// One representative directory path (for the evidence list).
    sample_dir: Option<PathBuf>,
    file_count: usize,
    total_bytes: u64,
}

fn scan(root: &Path) -> BTreeMap<&'static str, PlatformBucket> {
    let mut out: BTreeMap<&'static str, PlatformBucket> = BTreeMap::new();
    for entry in WalkDir::new(root)
        .follow_links(false)
        .into_iter()
        .filter_map(|e| e.ok())
    {
        if !entry.file_type().is_file() {
            continue;
        }
        let path = entry.path();
        let display = path
            .to_string_lossy()
            .replace('\\', "/");
        // Only consider files under a /Binaries/<Platform>/ path.
        let lc_display = display.to_ascii_lowercase();
        if !lc_display.contains("/binaries/") {
            continue;
        }
        // Find which platform dir name this file sits under.
        for plat in PLATFORM_DIRS {
            let needle = format!("/binaries/{}/", plat.to_ascii_lowercase());
            if lc_display.contains(&needle) {
                let size = entry.metadata().map(|m| m.len()).unwrap_or(0);
                let bucket = out.entry(*plat).or_insert_with(|| PlatformBucket {
                    name: plat,
                    sample_dir: None,
                    file_count: 0,
                    total_bytes: 0,
                });
                if bucket.sample_dir.is_none() {
                    // Walk up until we hit the `<Platform>` dir component.
                    let mut anc = path.to_path_buf();
                    while let Some(p) = anc.parent() {
                        if p.file_name()
                            .map(|n| n.to_string_lossy().eq_ignore_ascii_case(plat))
                            .unwrap_or(false)
                        {
                            let rel = p.strip_prefix(root).unwrap_or(p).to_path_buf();
                            bucket.sample_dir = Some(rel);
                            break;
                        }
                        anc = p.to_path_buf();
                    }
                }
                bucket.file_count += 1;
                bucket.total_bytes = bucket.total_bytes.saturating_add(size);
                break;
            }
        }
    }
    out
}

fn build_finding(
    buckets: BTreeMap<&'static str, PlatformBucket>,
    reclaimable: u64,
    largest: u64,
) -> Finding {
    let severity = if reclaimable >= 500 * 1024 * 1024 {
        Severity::Warning
    } else {
        Severity::Info
    };

    let mut sorted: Vec<&PlatformBucket> = buckets.values().collect();
    sorted.sort_by(|a, b| b.total_bytes.cmp(&a.total_bytes));

    let evidence: Vec<Evidence> = sorted
        .iter()
        .filter_map(|b| {
            b.sample_dir.clone().map(|p| Evidence {
                path: p,
                size_bytes: b.total_bytes,
                note: Some(format!(
                    "{} ({} file(s))",
                    b.name, b.file_count
                )),
            })
        })
        .collect();

    let listing: Vec<String> = sorted
        .iter()
        .enumerate()
        .map(|(i, b)| {
            let role = if i == 0 { "kept (largest)" } else { "redundant" };
            format!("{} {} ({})", b.name, format_bytes(b.total_bytes), role)
        })
        .collect();

    let title = format!(
        "Multi-platform binaries: {} foreign-platform across {} dir(s)",
        format_bytes(reclaimable),
        sorted.len().saturating_sub(1),
    );

    let summary = format!(
        "Found binaries for {} platforms in this install: {}. \
         A given OS only runs one of them at a time. We assume the \
         largest tree is the user's primary platform; the rest are \
         pure ballast on this machine. Cross-platform tooling for \
         developers can re-cook these on demand.",
        sorted.len(),
        listing.join(", "),
    );

    let recommendation = format!(
        "Delete the non-primary platform binary trees \
         (e.g. `Engine/Binaries/Linux/`, `Engine/Binaries/Mac/`) — \
         keeping {} ({}). If you ever want to run on a different \
         OS, restore from backup or re-download.",
        sorted[0].name,
        format_bytes(largest),
    );

    Finding {
        detector: "platform_siblings".to_string(),
        category: Category::PlatformSiblings,
        severity,
        title,
        summary,
        evidence,
        reclaimable_bytes: Some(reclaimable),
        recommendation,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::io::Write;
    use tempfile::TempDir;

    fn write_file(path: &Path, bytes: u64) {
        if let Some(p) = path.parent() {
            fs::create_dir_all(p).unwrap();
        }
        fs::File::create(path)
            .unwrap()
            .write_all(&vec![0u8; bytes as usize])
            .unwrap();
    }

    #[test]
    fn no_finding_on_single_platform() {
        let tmp = TempDir::new().unwrap();
        write_file(
            &tmp.path().join("Engine/Binaries/Win64/game.exe"),
            40 * 1024 * 1024,
        );
        let d = PlatformSiblingsDetector;
        assert!(d.run(tmp.path()).unwrap().is_empty());
    }

    #[test]
    fn flags_win64_plus_linux() {
        let tmp = TempDir::new().unwrap();
        write_file(
            &tmp.path().join("Engine/Binaries/Win64/game.exe"),
            120 * 1024 * 1024,
        );
        write_file(
            &tmp.path().join("Engine/Binaries/Linux/game"),
            60 * 1024 * 1024,
        );
        let d = PlatformSiblingsDetector;
        let findings = d.run(tmp.path()).unwrap();
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].category, Category::PlatformSiblings);
        assert_eq!(findings[0].reclaimable_bytes, Some(60 * 1024 * 1024));
    }

    #[test]
    fn below_50mb_ignored() {
        let tmp = TempDir::new().unwrap();
        write_file(
            &tmp.path().join("Engine/Binaries/Win64/game.exe"),
            60 * 1024 * 1024,
        );
        write_file(
            &tmp.path().join("Engine/Binaries/Mac/game.app"),
            10 * 1024 * 1024,
        );
        let d = PlatformSiblingsDetector;
        // Reclaimable = 10 MB < 50 MB threshold → ignored.
        assert!(d.run(tmp.path()).unwrap().is_empty());
    }

    #[test]
    fn three_platforms_aggregate_reclaimable() {
        let tmp = TempDir::new().unwrap();
        write_file(
            &tmp.path().join("Engine/Binaries/Win64/game.exe"),
            120 * 1024 * 1024,
        );
        write_file(
            &tmp.path().join("Engine/Binaries/Linux/game"),
            60 * 1024 * 1024,
        );
        write_file(
            &tmp.path().join("Engine/Binaries/Mac/game.app/bin"),
            70 * 1024 * 1024,
        );
        let d = PlatformSiblingsDetector;
        let findings = d.run(tmp.path()).unwrap();
        assert_eq!(findings.len(), 1);
        // Largest = Win64 (120). Reclaimable = 60 + 70 = 130 MB.
        assert_eq!(findings[0].reclaimable_bytes, Some(130 * 1024 * 1024));
    }

    #[test]
    fn ignores_non_binaries_path() {
        // `<Game>/Win64/something.dat` outside /Binaries/ shouldn't count.
        let tmp = TempDir::new().unwrap();
        write_file(
            &tmp.path().join("Game/Win64/blob.dat"),
            100 * 1024 * 1024,
        );
        write_file(
            &tmp.path().join("Game/Linux/blob.dat"),
            100 * 1024 * 1024,
        );
        let d = PlatformSiblingsDetector;
        assert!(d.run(tmp.path()).unwrap().is_empty());
    }
}
