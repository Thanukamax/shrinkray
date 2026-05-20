//! Detect mod-manager / mod-installer leftover files.
//!
//! Vortex, Mod Organizer 2, and Nexus Mod Manager all leave breadcrumbs:
//! `.disabled` (toggle-off), `.modtemp` (extraction scratch),
//! `.nxm_backup` / `.vortex_backup` / `.mohidden`. Manual modders also
//! leave `.original` when patching a file.
//!
//! `.bak` is deliberately NOT in this list — that's the editor-leftover
//! detector's territory (build-artifact backups). Keeping the two
//! detectors orthogonal avoids double-flagging.

use super::Detector;
use crate::report::format_bytes;
use crate::types::{Category, Evidence, Finding, Severity};
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

/// Suffix patterns. Match a file when its lowercase name ENDS WITH the
/// pattern (including the leading `.`).
const SUFFIX_PATTERNS: &[(&str, &str)] = &[
    (".disabled", "mod manager disable toggle"),
    (".modtemp", "mod manager extraction scratch"),
    (".nxm_backup", "Nexus Mod Manager backup"),
    (".vortex_backup", "Vortex backup"),
    (".mohidden", "Mod Organizer hidden marker"),
    (".original", "manual mod patch backup"),
];

#[derive(Debug, Default)]
pub struct ModManagerArtifactsDetector;

impl Detector for ModManagerArtifactsDetector {
    fn name(&self) -> &'static str {
        "mod_manager_artifacts"
    }

    fn run(&self, root: &Path) -> anyhow::Result<Vec<Finding>> {
        let hits = scan(root);
        if hits.is_empty() {
            return Ok(vec![]);
        }
        Ok(vec![build_finding(hits)])
    }
}

#[derive(Debug)]
struct Hit {
    rel_path: PathBuf,
    size_bytes: u64,
    reason: &'static str,
}

fn scan(root: &Path) -> Vec<Hit> {
    let mut out = Vec::new();
    for entry in WalkDir::new(root)
        .follow_links(false)
        .into_iter()
        .filter_map(|e| e.ok())
    {
        if !entry.file_type().is_file() {
            continue;
        }
        let path = entry.path();
        let lc = entry.file_name().to_string_lossy().to_ascii_lowercase();
        for (sfx, reason) in SUFFIX_PATTERNS {
            if lc.ends_with(sfx) {
                let size = entry.metadata().map(|m| m.len()).unwrap_or(0);
                let rel = path.strip_prefix(root).unwrap_or(path).to_path_buf();
                out.push(Hit {
                    rel_path: rel,
                    size_bytes: size,
                    reason,
                });
                break;
            }
        }
    }
    out.sort_by(|a, b| b.size_bytes.cmp(&a.size_bytes));
    out
}

fn build_finding(hits: Vec<Hit>) -> Finding {
    let total_bytes: u64 = hits.iter().map(|h| h.size_bytes).sum();
    let severity = if total_bytes >= 50 * 1024 * 1024 {
        Severity::Warning
    } else {
        Severity::Info
    };

    let evidence: Vec<Evidence> = hits
        .iter()
        .map(|h| Evidence {
            path: h.rel_path.clone(),
            size_bytes: h.size_bytes,
            note: Some(h.reason.to_string()),
        })
        .collect();

    let title = format!(
        "Mod-manager leftovers: {} across {} file(s)",
        format_bytes(total_bytes),
        hits.len()
    );

    let summary =
        "These files were left behind by Vortex, Mod Organizer, Nexus Mod \
         Manager, or manual mod installs. `.disabled` files are toggled-off \
         mod content the manager keeps around as a hidden state; `.modtemp` \
         is extraction scratch the manager forgot to clean up; `.original` \
         is the pre-patch copy of a file. None of these load at runtime."
            .to_string();

    let recommendation =
        "Safe to delete. If you uninstall the mod manager first, it can't \
         re-toggle these automatically — make sure you don't need the \
         disabled mods back before deleting."
            .to_string();

    Finding {
        detector: "mod_manager_artifacts".to_string(),
        category: Category::ModManagerArtifacts,
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
    fn no_finding_on_clean_install() {
        let tmp = TempDir::new().unwrap();
        write_file(&tmp.path().join("Content/Paks/x.pak"), 1024);
        let d = ModManagerArtifactsDetector;
        assert!(d.run(tmp.path()).unwrap().is_empty());
    }

    #[test]
    fn flags_disabled_file() {
        let tmp = TempDir::new().unwrap();
        write_file(
            &tmp.path().join("Content/Paks/MyMod_P.pak.disabled"),
            120 * 1024 * 1024,
        );
        let d = ModManagerArtifactsDetector;
        let findings = d.run(tmp.path()).unwrap();
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].category, Category::ModManagerArtifacts);
        assert_eq!(findings[0].reclaimable_bytes, Some(120 * 1024 * 1024));
        assert_eq!(findings[0].severity, Severity::Warning);
    }

    #[test]
    fn flags_nexus_backup() {
        let tmp = TempDir::new().unwrap();
        write_file(
            &tmp.path()
                .join("Content/Paks/MyMod_P.pak.nxm_backup"),
            5 * 1024 * 1024,
        );
        let d = ModManagerArtifactsDetector;
        let findings = d.run(tmp.path()).unwrap();
        assert_eq!(findings.len(), 1);
        assert!(findings[0].evidence[0]
            .note
            .as_ref()
            .unwrap()
            .contains("Nexus"));
    }

    #[test]
    fn does_not_flag_bak_files() {
        // .bak belongs to editor_leftovers, not us.
        let tmp = TempDir::new().unwrap();
        write_file(&tmp.path().join("Game/notes.bak"), 100);
        let d = ModManagerArtifactsDetector;
        assert!(d.run(tmp.path()).unwrap().is_empty());
    }

    #[test]
    fn aggregates_mixed_patterns() {
        let tmp = TempDir::new().unwrap();
        write_file(
            &tmp.path().join("Content/Paks/A_P.pak.disabled"),
            20 * 1024 * 1024,
        );
        write_file(
            &tmp.path().join("Content/Paks/B_P.pak.original"),
            10 * 1024 * 1024,
        );
        write_file(
            &tmp.path().join("Content/Paks/Half-installed.modtemp"),
            1 * 1024 * 1024,
        );
        let d = ModManagerArtifactsDetector;
        let findings = d.run(tmp.path()).unwrap();
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].evidence.len(), 3);
        assert_eq!(findings[0].reclaimable_bytes, Some(31 * 1024 * 1024));
        assert_eq!(findings[0].severity, Severity::Info); // < 50 MB
    }
}
