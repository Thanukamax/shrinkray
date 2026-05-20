//! Detect bundled redistributable installers (VC++, DirectX, UE Prereqs, .NET).
//!
//! Games ship these as a one-time prereq install. After first launch they're
//! pure ballast. Detection is filename-only and conservative — only common
//! known patterns are matched, and we ignore anything outside likely redist
//! folders so we never flag a game's actual VC runtime DLLs.

use super::Detector;
use crate::report::format_bytes;
use crate::types::{Category, Evidence, Finding, Severity};
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

/// Case-insensitive filename patterns. Matches if the file's lowercase name
/// CONTAINS the pattern.
const INSTALLER_PATTERNS: &[(&str, &str)] = &[
    ("ue4prereqsetup", "UE4 Prerequisites installer"),
    ("ueprereqsetup", "UE Prerequisites installer"),
    ("ue5prereqsetup", "UE5 Prerequisites installer"),
    ("vc_redist", "Visual C++ runtime installer"),
    ("vcredist", "Visual C++ runtime installer"),
    ("dxsetup.exe", "DirectX runtime installer"),
    ("dxwebsetup.exe", "DirectX web installer"),
    ("dotnetfx", ".NET Framework installer"),
    ("ndp48", ".NET Framework 4.8 installer"),
    ("ndp472", ".NET Framework 4.7.2 installer"),
];

/// Path-substring hints. We only consider files whose path contains one of
/// these (case-insensitive, OS-normalised to `/`).
const REDIST_PATH_HINTS: &[&str] = &[
    "/extras/redist/",
    "/redist/",
    "/_commonredist/",
    "/commonredist/",
    "/prereqs/",
];

#[derive(Debug, Default)]
pub struct RedistInstallerDetector;

impl Detector for RedistInstallerDetector {
    fn name(&self) -> &'static str {
        "redist_installer"
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
        let lc_name = entry.file_name().to_string_lossy().to_ascii_lowercase();
        let lc_path = path
            .to_string_lossy()
            .replace('\\', "/")
            .to_ascii_lowercase();

        let in_redist_path = REDIST_PATH_HINTS.iter().any(|p| lc_path.contains(p));

        for (pat, reason) in INSTALLER_PATTERNS {
            if lc_name.contains(pat) {
                // Strict mode: require BOTH a known filename AND the file
                // sitting under a redist-ish folder. Keeps us from flagging
                // a game's actual VC runtime DLL or a tool named similarly.
                if !in_redist_path {
                    // Allow standalone .exe under the install root — many
                    // games drop UE4PrereqSetup_x64.exe at the top level.
                    let depth = entry.depth();
                    if !(lc_name.ends_with(".exe") && depth <= 2) {
                        continue;
                    }
                }
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
    let severity = if total_bytes >= 10 * 1024 * 1024 {
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
        "Redistributable installers: {} across {} file(s)",
        format_bytes(total_bytes),
        hits.len()
    );

    let summary = format!(
        "Found {} bundled redistributable installer(s) totalling {}. \
         These are one-time prereq packages (Visual C++ runtime, \
         DirectX, UE Prerequisites, .NET Framework) that run when the \
         game is first installed and then sit dormant. They can be \
         safely deleted; if a system ever needs them, they redownload \
         from Microsoft.",
        hits.len(),
        format_bytes(total_bytes),
    );

    let recommendation =
        "Delete these after confirming the game launches. If you ever \
         move the install folder to a fresh machine you can re-run them \
         from Microsoft's official downloads."
            .to_string();

    Finding {
        detector: "redist_installer".to_string(),
        category: Category::RedistInstaller,
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
    fn flags_ue4_prereq_in_redist_folder() {
        let tmp = TempDir::new().unwrap();
        write_file(
            &tmp.path().join("Engine/Extras/Redist/en-us/UE4PrereqSetup_x64.exe"),
            40 * 1024 * 1024,
        );
        let d = RedistInstallerDetector;
        let findings = d.run(tmp.path()).unwrap();
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].category, Category::RedistInstaller);
        assert_eq!(findings[0].reclaimable_bytes, Some(40 * 1024 * 1024));
        assert_eq!(findings[0].severity, Severity::Warning);
    }

    #[test]
    fn flags_vc_redist_in_commonredist() {
        let tmp = TempDir::new().unwrap();
        write_file(
            &tmp.path().join("_CommonRedist/vcredist/2019/vc_redist.x64.exe"),
            14 * 1024 * 1024,
        );
        let d = RedistInstallerDetector;
        let findings = d.run(tmp.path()).unwrap();
        assert_eq!(findings.len(), 1);
    }

    #[test]
    fn ignores_non_installer_named_files() {
        let tmp = TempDir::new().unwrap();
        // A game DLL that happens to live under Engine/Extras: not flagged.
        write_file(
            &tmp.path().join("Engine/Extras/Redist/en-us/notes.txt"),
            500,
        );
        let d = RedistInstallerDetector;
        assert!(d.run(tmp.path()).unwrap().is_empty());
    }

    #[test]
    fn requires_redist_path_or_root_exe() {
        // vcredist.dll deep inside Engine/Binaries should NOT be flagged —
        // that's the actual runtime, not the installer.
        let tmp = TempDir::new().unwrap();
        write_file(
            &tmp.path()
                .join("Engine/Binaries/ThirdParty/VCRedist/vcredist_runtime.dll"),
            5 * 1024 * 1024,
        );
        let d = RedistInstallerDetector;
        // No `.exe` at root, no redist path hint, no /redist/ either → ignored.
        // Actually the path DOES include "/vcredist/" lowercased... but
        // REDIST_PATH_HINTS doesn't include /vcredist/. Good.
        let findings = d.run(tmp.path()).unwrap();
        // Even with a vcredist match in the filename, since the .dll isn't
        // under a redist hint path AND it's not a root-level .exe, ignored.
        // This relies on the .dll extension dodging the .exe gate.
        assert!(findings.is_empty(), "got {:?}", findings);
    }

    #[test]
    fn flags_top_level_prereq_exe() {
        // Some installers drop UE4PrereqSetup at the install root.
        let tmp = TempDir::new().unwrap();
        write_file(
            &tmp.path().join("UE4PrereqSetup_x64.exe"),
            40 * 1024 * 1024,
        );
        let d = RedistInstallerDetector;
        assert_eq!(d.run(tmp.path()).unwrap().len(), 1);
    }
}
