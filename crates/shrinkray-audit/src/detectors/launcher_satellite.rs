//! Detect per-language .NET satellite assembly directories at the launcher
//! root — small individually (~5-20 MB each) but signal of "ship everything,
//! prune nothing" discipline.
//!
//! Pattern: a launcher exe ships in a directory alongside per-language
//! sibling folders (`cs/`, `de/`, `es/`, `fr/`, `it/`, `ja/`, `ko/`, `pl/`,
//! `pt-BR/`, `ru/`, `tr/`, `zh-Hans/`, `zh-Hant/`), each containing
//! `<App>.resources.dll` — the standard .NET satellite layout.
//!
//! We can't know which language the user actually uses without app config,
//! so the recommendation is just "trim to the language(s) you launch in" —
//! we surface the total reducible budget.

use super::Detector;
use crate::report::format_bytes;
use crate::types::{Category, Evidence, Finding, Severity};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

const MIN_SATELLITE_SIBLINGS: usize = 3;

#[derive(Debug, Default)]
pub struct LauncherSatelliteDetector;

impl Detector for LauncherSatelliteDetector {
    fn name(&self) -> &'static str {
        "launcher_satellite"
    }

    fn run(&self, root: &Path) -> anyhow::Result<Vec<Finding>> {
        let groups = find_satellite_groups(root);
        if groups.is_empty() {
            return Ok(vec![]);
        }
        Ok(vec![build_finding(root, groups)])
    }
}

#[derive(Debug)]
struct SatelliteGroup {
    satellites: Vec<Satellite>,
}

#[derive(Debug)]
struct Satellite {
    rel_path: PathBuf,
    lang_code: String,
    size_bytes: u64,
}

fn find_satellite_groups(root: &Path) -> Vec<SatelliteGroup> {
    let mut by_parent: BTreeMap<PathBuf, Vec<Satellite>> = BTreeMap::new();

    for entry in WalkDir::new(root)
        .follow_links(false)
        .min_depth(1)
        .into_iter()
        .filter_map(|e| e.ok())
    {
        if !entry.file_type().is_dir() {
            continue;
        }
        let dir = entry.path();
        let Some(name) = dir.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        if !is_language_code(name) {
            continue;
        }
        if !contains_resources_dll(dir) {
            continue;
        }
        let size = dir_size(dir);
        let Some(parent) = dir.parent() else { continue };
        let rel_path = dir.strip_prefix(root).unwrap_or(dir).to_path_buf();
        by_parent
            .entry(parent.to_path_buf())
            .or_default()
            .push(Satellite {
                rel_path,
                lang_code: name.to_string(),
                size_bytes: size,
            });
    }

    by_parent
        .into_values()
        .filter_map(|satellites| {
            if satellites.len() < MIN_SATELLITE_SIBLINGS {
                None
            } else {
                Some(SatelliteGroup { satellites })
            }
        })
        .collect()
}

/// Match BCP-47 launcher satellite directory names:
/// - 2-3 lowercase ASCII letters (`en`, `de`, `ko`, `zho`)
/// - 2-3 lowercase ASCII letters + `-` + 2-4 alphanumeric (`pt-BR`, `zh-Hans`)
///
/// Conservative — we keep this tight so non-language dirs like `cs/` (would
/// match!) only count when also containing `.resources.dll` files, gated by
/// the caller via [`contains_resources_dll`].
fn is_language_code(name: &str) -> bool {
    let bytes = name.as_bytes();
    let len = bytes.len();
    if !(2..=8).contains(&len) {
        return false;
    }
    let all_letters = |range: &[u8]| range.iter().all(|b| b.is_ascii_alphabetic());

    if let Some(hyphen_pos) = name.find('-') {
        let (prefix, rest) = (&bytes[..hyphen_pos], &bytes[hyphen_pos + 1..]);
        // prefix must be 2-3 lowercase letters
        if !(2..=3).contains(&prefix.len()) || !prefix.iter().all(|b| b.is_ascii_lowercase()) {
            return false;
        }
        // suffix must be 2-4 alnum (covers Hans, Hant, BR, CN, etc.)
        if !(2..=4).contains(&rest.len()) || !rest.iter().all(|b| b.is_ascii_alphanumeric()) {
            return false;
        }
        true
    } else {
        (2..=3).contains(&len) && all_letters(bytes) && bytes.iter().all(|b| b.is_ascii_lowercase())
    }
}

/// True if `dir` contains at least one `*.dll` direct child. We don't insist
/// on the `.resources.dll` suffix specifically — UE launcher localisation
/// sometimes ships plain `.dll` resource bundles.
fn contains_resources_dll(dir: &Path) -> bool {
    let Ok(read_dir) = fs::read_dir(dir) else {
        return false;
    };
    for ent in read_dir.flatten() {
        let p = ent.path();
        if p.is_file() && p.extension().and_then(|s| s.to_str()) == Some("dll") {
            return true;
        }
    }
    false
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

fn build_finding(root: &Path, groups: Vec<SatelliteGroup>) -> Finding {
    let total_bytes: u64 = groups
        .iter()
        .flat_map(|g| g.satellites.iter().map(|s| s.size_bytes))
        .sum();
    let total_count: usize = groups.iter().map(|g| g.satellites.len()).sum();

    let mut evidence: Vec<Evidence> = Vec::new();
    for g in &groups {
        for s in &g.satellites {
            evidence.push(Evidence {
                path: s.rel_path.clone(),
                size_bytes: s.size_bytes,
                note: Some(format!("`{}` satellite", s.lang_code)),
            });
        }
    }
    // Sort largest-first for evidence display.
    evidence.sort_by(|a, b| b.size_bytes.cmp(&a.size_bytes));

    // Conservative reclaimable: assume user keeps one language. So
    // reclaimable = total - (largest satellite per group).
    let reclaimable: u64 = groups
        .iter()
        .map(|g| {
            let group_total: u64 = g.satellites.iter().map(|s| s.size_bytes).sum();
            let largest = g.satellites.iter().map(|s| s.size_bytes).max().unwrap_or(0);
            group_total.saturating_sub(largest)
        })
        .sum();

    let severity = if reclaimable >= 10 * 1024 * 1024 {
        Severity::Warning
    } else {
        Severity::Info
    };

    let _ = root;
    let title = format!(
        "Launcher language satellites: {} across {} satellite(s) in {} location(s)",
        format_bytes(total_bytes),
        total_count,
        groups.len()
    );

    let summary = format!(
        "Detected {} per-language .NET satellite assembly director(ies) \
         alongside what looks like a launcher binary. Each carries localised \
         resources for one language — but the launcher only ever uses one \
         locale at a time. Conservative reclaimable estimate assumes you keep \
         the largest satellite per location and delete the rest: {}.",
        total_count,
        format_bytes(reclaimable),
    );

    let recommendation = "Decide which language(s) you launch the app in and \
         delete the other satellite directories. Safe to do manually; \
         shrinkray will get a per-language strip op in v0.5+."
        .to_string();

    Finding {
        detector: "launcher_satellite".to_string(),
        category: Category::LauncherSatellite,
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
    use std::io::Write;
    use tempfile::TempDir;

    fn write_dll(path: &Path, bytes: u64) {
        if let Some(p) = path.parent() {
            fs::create_dir_all(p).unwrap();
        }
        fs::File::create(path)
            .unwrap()
            .write_all(&vec![0u8; bytes as usize])
            .unwrap();
    }

    #[test]
    fn lang_code_matches_two_letter() {
        assert!(is_language_code("en"));
        assert!(is_language_code("de"));
        assert!(is_language_code("ko"));
        assert!(is_language_code("ja"));
    }

    #[test]
    fn lang_code_matches_bcp47_hyphenated() {
        assert!(is_language_code("pt-BR"));
        assert!(is_language_code("zh-Hans"));
        assert!(is_language_code("zh-Hant"));
        assert!(is_language_code("zh-CN"));
    }

    #[test]
    fn lang_code_rejects_obvious_non_languages() {
        assert!(!is_language_code("Binaries"));
        assert!(!is_language_code("Content"));
        assert!(!is_language_code("EN")); // uppercase prefix not lowercase
        assert!(!is_language_code(""));
        assert!(!is_language_code("a"));
        assert!(!is_language_code("toolong-suffix"));
    }

    #[test]
    fn no_finding_when_no_satellites() {
        let tmp = TempDir::new().unwrap();
        write_dll(&tmp.path().join("launcher.exe"), 1024);
        let d = LauncherSatelliteDetector;
        let findings = d.run(tmp.path()).unwrap();
        assert!(findings.is_empty());
    }

    #[test]
    fn no_finding_below_threshold() {
        // Only 2 satellites — under MIN_SATELLITE_SIBLINGS = 3.
        let tmp = TempDir::new().unwrap();
        write_dll(&tmp.path().join("launcher/en/x.dll"), 1024);
        write_dll(&tmp.path().join("launcher/de/x.dll"), 1024);
        let d = LauncherSatelliteDetector;
        let findings = d.run(tmp.path()).unwrap();
        assert!(findings.is_empty());
    }

    #[test]
    fn rejects_language_dirs_without_dlls() {
        // 3 dirs named like languages but containing no .dll → not satellites.
        let tmp = TempDir::new().unwrap();
        for code in ["en", "de", "fr"] {
            write_dll(&tmp.path().join(format!("launcher/{}/notes.txt", code)), 100);
            // Rename to .txt so contains_resources_dll returns false.
        }
        let d = LauncherSatelliteDetector;
        let findings = d.run(tmp.path()).unwrap();
        assert!(findings.is_empty());
    }

    #[test]
    fn flags_satellite_collection() {
        let tmp = TempDir::new().unwrap();
        // Launcher with 5 language satellites, sizes vary.
        write_dll(&tmp.path().join("launcher/en/Launcher.resources.dll"), 5 * 1024 * 1024);
        write_dll(&tmp.path().join("launcher/de/Launcher.resources.dll"), 4 * 1024 * 1024);
        write_dll(&tmp.path().join("launcher/fr/Launcher.resources.dll"), 4 * 1024 * 1024);
        write_dll(&tmp.path().join("launcher/ko/Launcher.resources.dll"), 3 * 1024 * 1024);
        write_dll(&tmp.path().join("launcher/zh-Hans/Launcher.resources.dll"), 3 * 1024 * 1024);

        let d = LauncherSatelliteDetector;
        let findings = d.run(tmp.path()).unwrap();
        assert_eq!(findings.len(), 1);
        let f = &findings[0];
        assert_eq!(f.category, Category::LauncherSatellite);
        // Reclaimable = total - largest = 19MB - 5MB = 14MB
        assert_eq!(f.reclaimable_bytes, Some(14 * 1024 * 1024));
        assert_eq!(f.severity, Severity::Warning);
        assert_eq!(f.evidence.len(), 5);
    }

    #[test]
    fn aggregates_across_multiple_apps() {
        let tmp = TempDir::new().unwrap();
        // Two separate apps each with 3 satellite dirs.
        for code in ["en", "de", "fr"] {
            write_dll(
                &tmp.path().join(format!("app1/{}/x.dll", code)),
                100,
            );
            write_dll(
                &tmp.path().join(format!("app2/{}/y.dll", code)),
                200,
            );
        }
        let d = LauncherSatelliteDetector;
        let findings = d.run(tmp.path()).unwrap();
        assert_eq!(findings.len(), 1);
        let f = &findings[0];
        assert!(f.title.contains("2 location"));
        assert_eq!(f.evidence.len(), 6);
    }
}
