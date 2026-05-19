//! Detect editor-only / build leftover files that snuck into a cooked
//! release.
//!
//! These should never appear in a shipped game and are safe to delete (with
//! shrinkray's backup, of course). Conservative pattern list — anything
//! contextual (loose `.cpp` files, `.py` for runtime scripting) is left out
//! to avoid false positives.

use super::Detector;
use crate::report::format_bytes;
use crate::types::{Category, Evidence, Finding, Severity};
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

/// File-extension blacklist (must match exactly, case-insensitive).
const SUSPECT_EXTENSIONS: &[&str] = &[
    "pdb",      // Microsoft program database / debug symbols
    "ilk",      // incremental linker output
    "exp",      // linker export file
    "tps",      // UE Third Party Software annotation
    "uproject", // UE project descriptor
    "uplugin",  // UE plugin descriptor (when loose at root)
    "bak",      // editor backup
    "orig",     // diff backup
];

/// Path-substring patterns (case-insensitive, OS-normalized to forward slashes).
const SUSPECT_PATH_SUBSTRINGS: &[&str] = &[
    "/intermediate/",
    "/derivedatacache/",
    "/derived data cache/",
    "/source/",       // engine source folder in shipped builds
    "/build/",        // intermediate build outputs
    "/engine/editor/",
    "/game/editor/",
];

#[derive(Debug, Default)]
pub struct EditorLeftoverDetector;

impl Detector for EditorLeftoverDetector {
    fn name(&self) -> &'static str {
        "editor_leftovers"
    }

    fn run(&self, root: &Path) -> anyhow::Result<Vec<Finding>> {
        let leftovers = scan(root);
        if leftovers.is_empty() {
            return Ok(vec![]);
        }
        Ok(vec![build_finding(root, leftovers)])
    }
}

#[derive(Debug)]
struct Leftover {
    rel_path: PathBuf,
    size_bytes: u64,
    reason: &'static str,
}

fn scan(root: &Path) -> Vec<Leftover> {
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
        let size = entry.metadata().map(|m| m.len()).unwrap_or(0);

        if let Some(reason) = classify(path) {
            let rel_path = path.strip_prefix(root).unwrap_or(path).to_path_buf();
            out.push(Leftover {
                rel_path,
                size_bytes: size,
                reason,
            });
        }
    }
    out.sort_by(|a, b| b.size_bytes.cmp(&a.size_bytes));
    out
}

/// Returns Some(reason) if the file is a leftover; None otherwise.
fn classify(path: &Path) -> Option<&'static str> {
    if let Some(ext) = path.extension().and_then(|s| s.to_str()) {
        let lc = ext.to_ascii_lowercase();
        for sus in SUSPECT_EXTENSIONS {
            if lc == *sus {
                return Some(match *sus {
                    "pdb" => "debug symbols (.pdb)",
                    "ilk" => "linker incremental output (.ilk)",
                    "exp" => "linker export file (.exp)",
                    "tps" => "Third Party Software metadata (.tps)",
                    "uproject" => "UE project descriptor (.uproject)",
                    "uplugin" => "UE plugin descriptor (.uplugin)",
                    "bak" => "editor backup (.bak)",
                    "orig" => "diff backup (.orig)",
                    _ => "suspicious extension",
                });
            }
        }
    }
    // Path-substring patterns — normalise OS separators to `/` lower-cased.
    let display = path.to_string_lossy().replace('\\', "/").to_ascii_lowercase();
    for sub in SUSPECT_PATH_SUBSTRINGS {
        if display.contains(sub) {
            return Some(match *sub {
                "/intermediate/" => "build intermediate output",
                "/derivedatacache/" | "/derived data cache/" => "derived data cache",
                "/source/" => "engine source tree in cooked install",
                "/build/" => "build script output",
                "/engine/editor/" | "/game/editor/" => "editor-only content path",
                _ => "suspicious path",
            });
        }
    }
    None
}

fn build_finding(_root: &Path, leftovers: Vec<Leftover>) -> Finding {
    let total_bytes: u64 = leftovers.iter().map(|l| l.size_bytes).sum();
    let severity = if total_bytes >= 1024 * 1024 {
        Severity::Warning
    } else {
        Severity::Info
    };

    let evidence: Vec<Evidence> = leftovers
        .iter()
        .map(|l| Evidence {
            path: l.rel_path.clone(),
            size_bytes: l.size_bytes,
            note: Some(l.reason.to_string()),
        })
        .collect();

    let title = format!(
        "Editor leftovers: {} across {} file(s)",
        format_bytes(total_bytes),
        leftovers.len()
    );

    let summary = format!(
        "Found {} file(s) that look like editor / build leftovers — these \
         should not exist in a cooked release and are safe to delete after \
         taking a backup. The pattern catalog matches conservative cases \
         only (file extensions like .pdb / .uproject and path components \
         like /Intermediate/ or /Engine/Editor/). Files with ambiguous \
         provenance (loose .cpp, .py) are not flagged.",
        leftovers.len(),
    );

    let recommendation = "Run `shrinkray restore` afterwards if anything \
         breaks — the backup keeps these files reachable. For known-safe \
         deletes you can target this category specifically once the \
         per-category strip op lands in v0.5."
        .to_string();

    Finding {
        detector: "editor_leftovers".to_string(),
        category: Category::EditorLeftovers,
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
        write_file(&tmp.path().join("Content/Paks/pakchunk0.pak"), 1024);
        write_file(&tmp.path().join("Binaries/Win64/game.exe"), 2048);

        let d = EditorLeftoverDetector;
        let findings = d.run(tmp.path()).unwrap();
        assert!(findings.is_empty());
    }

    #[test]
    fn flags_pdb_symbols() {
        let tmp = TempDir::new().unwrap();
        write_file(&tmp.path().join("Binaries/Win64/game.pdb"), 5 * 1024 * 1024);

        let d = EditorLeftoverDetector;
        let findings = d.run(tmp.path()).unwrap();
        assert_eq!(findings.len(), 1);
        let f = &findings[0];
        assert_eq!(f.category, Category::EditorLeftovers);
        assert_eq!(f.reclaimable_bytes, Some(5 * 1024 * 1024));
        assert_eq!(f.severity, Severity::Warning); // > 1 MB
        assert_eq!(f.evidence.len(), 1);
        assert!(f.evidence[0].note.as_ref().unwrap().contains(".pdb"));
    }

    #[test]
    fn flags_intermediate_path() {
        let tmp = TempDir::new().unwrap();
        write_file(
            &tmp.path().join("Game/Intermediate/Build/x.obj"),
            10 * 1024,
        );

        let d = EditorLeftoverDetector;
        let findings = d.run(tmp.path()).unwrap();
        assert_eq!(findings.len(), 1);
        // /Intermediate/ matches before /Build/ in our list ordering, so
        // reason should be "build intermediate output".
        assert_eq!(findings[0].evidence[0].note.as_deref(), Some("build intermediate output"));
    }

    #[test]
    fn flags_editor_path() {
        let tmp = TempDir::new().unwrap();
        write_file(&tmp.path().join("Engine/Editor/EditorUI/icon.png"), 500);

        let d = EditorLeftoverDetector;
        let findings = d.run(tmp.path()).unwrap();
        assert_eq!(findings.len(), 1);
    }

    #[test]
    fn flags_uproject_file() {
        let tmp = TempDir::new().unwrap();
        write_file(&tmp.path().join("MyGame.uproject"), 500);

        let d = EditorLeftoverDetector;
        let findings = d.run(tmp.path()).unwrap();
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].severity, Severity::Info); // < 1 MB
    }

    #[test]
    fn aggregates_multiple_leftover_types() {
        let tmp = TempDir::new().unwrap();
        write_file(&tmp.path().join("Binaries/Win64/game.pdb"), 1024 * 1024);
        write_file(&tmp.path().join("Game.uproject"), 200);
        write_file(&tmp.path().join("Game/Intermediate/x"), 5 * 1024);

        let d = EditorLeftoverDetector;
        let findings = d.run(tmp.path()).unwrap();
        assert_eq!(findings.len(), 1);
        let f = &findings[0];
        assert_eq!(f.evidence.len(), 3);
        // Largest first (the .pdb).
        assert_eq!(f.evidence[0].size_bytes, 1024 * 1024);
    }

    #[test]
    fn case_insensitive_extension_matching() {
        let tmp = TempDir::new().unwrap();
        write_file(&tmp.path().join("a.PDB"), 100);

        let d = EditorLeftoverDetector;
        let findings = d.run(tmp.path()).unwrap();
        assert_eq!(findings.len(), 1);
    }

    #[test]
    fn case_insensitive_path_matching() {
        let tmp = TempDir::new().unwrap();
        // Note capitalisation in the path.
        write_file(&tmp.path().join("Engine/EDITOR/X/y.png"), 100);

        let d = EditorLeftoverDetector;
        let findings = d.run(tmp.path()).unwrap();
        // "/engine/editor/" matches even with /EDITOR/ in actual path.
        assert_eq!(findings.len(), 1);
    }
}
