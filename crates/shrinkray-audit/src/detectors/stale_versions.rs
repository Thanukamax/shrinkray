//! Detect stale version-named directories.
//!
//! Pattern observed in live-service titles: subdirectories named
//! `X.Y.Z` (semver-ish) accumulate after patches without cleanup. Example
//! from a real Wuthering Waves install:
//!
//! - `launcherDownload/3.1.0/`, `3.2.2/`, `3.3.0/` — three patch-version
//!   metadata dirs, only the highest is current
//! - `Saved/Resources/Video/3.2.0/`, `3.3.0/` — old video manifest stubs
//! - `Saved/Resources/3.3.0/Lang_en/3.3.9/` — superseded language patch
//!   (current is 3.3.11)
//!
//! v0.4 heuristic (conservative): within each parent directory, if there
//! are two or more children whose names parse as `X[.Y[.Z[.W]]]`, every
//! version below the maximum is flagged as stale. The "lone version dir"
//! case (`Lang_en/3.3.9/` with no peer) needs a cross-tree highest-version
//! heuristic — deferred to a later detector to avoid false positives where
//! a tree contains unrelated version namespaces (game vs engine).

use super::Detector;
use crate::report::format_bytes;
use crate::types::{Category, Evidence, Finding, Severity};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

#[derive(Debug, Default)]
pub struct StaleVersionDirDetector;

impl Detector for StaleVersionDirDetector {
    fn name(&self) -> &'static str {
        "stale_version_dir"
    }

    fn run(&self, root: &Path) -> anyhow::Result<Vec<Finding>> {
        let parents = collect_versioned_parents(root);

        // For each parent with 2+ version children, everything below the max
        // version is stale.
        let mut stale: Vec<StaleDir> = Vec::new();
        for (_parent, mut versions) in parents {
            if versions.len() < 2 {
                continue;
            }
            // Sort ascending; pop the max; rest are stale candidates.
            versions.sort_by(|a, b| a.version.cmp(&b.version));
            let _current = versions.pop();
            for v in versions {
                stale.push(v);
            }
        }

        if stale.is_empty() {
            return Ok(vec![]);
        }

        Ok(vec![build_finding(root, stale)])
    }
}

#[derive(Debug, Clone)]
struct StaleDir {
    parent_rel: PathBuf,
    name: String,
    version: Version,
    full_path: PathBuf,
    size_bytes: u64,
}

/// Up to 4-segment version. Comparison is segment-wise ascending so 3.3.9 <
/// 3.3.11 (semver semantics, not lexical).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct Version(Vec<u32>);

fn parse_version(name: &str) -> Option<Version> {
    let segments: Vec<&str> = name.split('.').collect();
    if segments.is_empty() || segments.len() > 4 {
        return None;
    }
    let mut parts = Vec::with_capacity(segments.len());
    for seg in segments {
        if seg.is_empty() {
            return None;
        }
        let n: u32 = seg.parse().ok()?;
        parts.push(n);
    }
    Some(Version(parts))
}

/// For each directory in the tree, return the list of version-named children.
fn collect_versioned_parents(root: &Path) -> BTreeMap<PathBuf, Vec<StaleDir>> {
    let mut by_parent: BTreeMap<PathBuf, Vec<StaleDir>> = BTreeMap::new();

    for entry in WalkDir::new(root)
        .follow_links(false)
        .into_iter()
        .filter_map(|e| e.ok())
    {
        if !entry.file_type().is_dir() {
            continue;
        }
        let path = entry.path();
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        let Some(version) = parse_version(name) else {
            continue;
        };
        let Some(parent) = path.parent() else { continue };
        let parent_rel = parent.strip_prefix(root).unwrap_or(parent).to_path_buf();

        let size_bytes = dir_size(path);
        let entry = StaleDir {
            parent_rel: parent_rel.clone(),
            name: name.to_string(),
            version,
            full_path: path.to_path_buf(),
            size_bytes,
        };
        by_parent.entry(parent_rel).or_default().push(entry);
    }
    by_parent
}

fn dir_size(path: &Path) -> u64 {
    let mut total: u64 = 0;
    for e in WalkDir::new(path)
        .follow_links(false)
        .into_iter()
        .filter_map(|x| x.ok())
    {
        if e.file_type().is_file() {
            if let Ok(md) = e.metadata() {
                total = total.saturating_add(md.len());
            }
        }
    }
    total
}

fn build_finding(root: &Path, stale: Vec<StaleDir>) -> Finding {
    let total_bytes: u64 = stale.iter().map(|d| d.size_bytes).sum();
    let parent_count = stale
        .iter()
        .map(|d| d.parent_rel.clone())
        .collect::<std::collections::BTreeSet<_>>()
        .len();

    let evidence: Vec<Evidence> = stale
        .iter()
        .map(|d| Evidence {
            path: d
                .full_path
                .strip_prefix(root)
                .unwrap_or(&d.full_path)
                .to_path_buf(),
            size_bytes: d.size_bytes,
            note: Some(format!("v{} (superseded)", d.name)),
        })
        .collect();

    let severity = if total_bytes >= 100 * 1024 * 1024 {
        Severity::Warning
    } else {
        Severity::Info
    };

    let title = format!(
        "Stale version directories: {} across {} parent location(s)",
        format_bytes(total_bytes),
        parent_count
    );

    let summary = format!(
        "{} version-named directories are sitting alongside a newer version of \
         the same content. The launcher (or installer) didn't garbage-collect \
         them. These are leftovers from previous patches and can be deleted \
         without affecting the current game version.",
        stale.len()
    );

    let recommendation = "Delete each listed directory after confirming the game \
         currently launches (the live version uses the highest-version sibling). \
         For installs with active anti-cheat, take a launcher integrity check \
         pass first."
        .to_string();

    Finding {
        detector: "stale_version_dir".to_string(),
        category: Category::StaleVersionDir,
        severity,
        title,
        summary,
        evidence,
        reclaimable_bytes: Some(total_bytes),
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
    fn version_parses_numeric_dotted() {
        assert_eq!(parse_version("3.3.11"), Some(Version(vec![3, 3, 11])));
        assert_eq!(parse_version("1.0.0.0"), Some(Version(vec![1, 0, 0, 0])));
        assert_eq!(parse_version("3"), Some(Version(vec![3])));
    }

    #[test]
    fn version_rejects_non_numeric() {
        assert_eq!(parse_version("Base"), None);
        assert_eq!(parse_version("3.3.alpha"), None);
        assert_eq!(parse_version("Lang_en"), None);
        assert_eq!(parse_version(""), None);
        assert_eq!(parse_version("3."), None);
    }

    #[test]
    fn version_orders_numerically_not_lexically() {
        // The bug-magnet case: 3.3.9 must compare LESS THAN 3.3.11.
        let a = parse_version("3.3.9").unwrap();
        let b = parse_version("3.3.11").unwrap();
        assert!(a < b, "3.3.9 should be older than 3.3.11");
    }

    #[test]
    fn no_finding_when_no_versions() {
        let tmp = TempDir::new().unwrap();
        write_file(&tmp.path().join("a.bin"), 100);
        write_file(&tmp.path().join("sub/b.bin"), 100);

        let d = StaleVersionDirDetector;
        let findings = d.run(tmp.path()).unwrap();
        assert!(findings.is_empty());
    }

    #[test]
    fn no_finding_for_lone_version_dir() {
        // Only one version dir under parent → can't tell if it's stale.
        let tmp = TempDir::new().unwrap();
        write_file(&tmp.path().join("Resource/3.3.11/data.bin"), 100);

        let d = StaleVersionDirDetector;
        let findings = d.run(tmp.path()).unwrap();
        assert!(findings.is_empty());
    }

    #[test]
    fn flags_older_siblings() {
        let tmp = TempDir::new().unwrap();
        // launcherDownload/{3.1.0, 3.2.2, 3.3.0} — older two are stale.
        write_file(
            &tmp.path().join("launcherDownload/3.1.0/manifest.json"),
            10 * 1024 * 1024,
        );
        write_file(
            &tmp.path().join("launcherDownload/3.2.2/manifest.json"),
            20 * 1024 * 1024,
        );
        write_file(
            &tmp.path().join("launcherDownload/3.3.0/manifest.json"),
            5 * 1024 * 1024,
        );

        let d = StaleVersionDirDetector;
        let findings = d.run(tmp.path()).unwrap();
        assert_eq!(findings.len(), 1);
        let f = &findings[0];
        // Stale = 3.1.0 (10 MB) + 3.2.2 (20 MB) = 30 MB; 3.3.0 is the live max.
        assert_eq!(f.reclaimable_bytes, Some(30 * 1024 * 1024));
        assert_eq!(f.evidence.len(), 2);
        // Each evidence path ends in the stale version name.
        let paths: Vec<String> = f
            .evidence
            .iter()
            .map(|e| e.path.display().to_string())
            .collect();
        assert!(paths.iter().any(|p| p.contains("3.1.0")));
        assert!(paths.iter().any(|p| p.contains("3.2.2")));
        assert!(!paths.iter().any(|p| p.contains("3.3.0")));
    }

    #[test]
    fn severity_info_when_savings_are_tiny() {
        let tmp = TempDir::new().unwrap();
        write_file(&tmp.path().join("d/1.0.0/a"), 100);
        write_file(&tmp.path().join("d/2.0.0/a"), 100);

        let d = StaleVersionDirDetector;
        let findings = d.run(tmp.path()).unwrap();
        assert_eq!(findings[0].severity, Severity::Info);
    }

    #[test]
    fn severity_warning_when_savings_meaningful() {
        let tmp = TempDir::new().unwrap();
        write_file(
            &tmp.path().join("d/1.0.0/big"),
            150 * 1024 * 1024,
        );
        write_file(&tmp.path().join("d/2.0.0/big"), 10);

        let d = StaleVersionDirDetector;
        let findings = d.run(tmp.path()).unwrap();
        assert_eq!(findings[0].severity, Severity::Warning);
    }

    #[test]
    fn aggregates_across_multiple_parents() {
        // Two unrelated parents, each with stale versions.
        let tmp = TempDir::new().unwrap();
        write_file(&tmp.path().join("launcherDownload/3.1.0/a"), 1024);
        write_file(&tmp.path().join("launcherDownload/3.2.0/a"), 1024);
        write_file(&tmp.path().join("Saved/Resources/Video/3.2.0/a"), 2048);
        write_file(&tmp.path().join("Saved/Resources/Video/3.3.0/a"), 2048);

        let d = StaleVersionDirDetector;
        let findings = d.run(tmp.path()).unwrap();
        assert_eq!(findings.len(), 1);
        let f = &findings[0];
        assert!(f.title.contains("2 parent"));
        assert_eq!(f.evidence.len(), 2);
        // 3.1.0 (1KB) + 3.2.0 video (2KB) = 3KB
        assert_eq!(f.reclaimable_bytes, Some(3 * 1024));
    }
}
