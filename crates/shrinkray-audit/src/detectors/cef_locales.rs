//! Detect unused CEF (Chromium Embedded Framework) locale bundles.
//!
//! CEF ships per-locale `.pak` files under `Engine/Binaries/ThirdParty/CEF3/
//! Resources/locales/`. A typical install ships 50+ locales (en-US, de, fr,
//! ko, zh-CN, …). Each is small (~150-300 KB) but the total adds up to
//! ~10-20 MB on every UE game that embeds CEF, and ~95% of users only need
//! one or two.
//!
//! Conservative default: keep `en-US` and `en-GB` (plus bare `en`). Anything
//! else is reported as reclaimable. The pak format here is CEF-internal, not
//! UE's, so the upstream pak classifier explicitly skips these — they only
//! show up in this detector.

use super::Detector;
use crate::report::format_bytes;
use crate::types::{Category, Evidence, Finding, Severity};
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

/// Locales kept by default. Case-insensitive.
const KEEPER_LOCALES: &[&str] = &["en", "en-US", "en-GB"];

#[derive(Debug, Default)]
pub struct CefLocalesDetector;

impl Detector for CefLocalesDetector {
    fn name(&self) -> &'static str {
        "cef_locales"
    }

    fn run(&self, root: &Path) -> anyhow::Result<Vec<Finding>> {
        let hits = scan(root);
        let reclaim_hits: Vec<_> = hits.iter().filter(|h| !h.kept).collect();
        if reclaim_hits.is_empty() {
            return Ok(vec![]);
        }
        let total_reclaim: u64 = reclaim_hits.iter().map(|h| h.size_bytes).sum();
        if total_reclaim < 1024 * 1024 {
            return Ok(vec![]);
        }
        Ok(vec![build_finding(hits, total_reclaim)])
    }
}

#[derive(Debug, Clone)]
struct LocaleHit {
    rel_path: PathBuf,
    locale: String,
    size_bytes: u64,
    kept: bool,
}

fn scan(root: &Path) -> Vec<LocaleHit> {
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
        let lc_path = path.to_string_lossy().replace('\\', "/").to_ascii_lowercase();
        if !lc_path.contains("/cef3/") && !lc_path.contains("/cef/") {
            continue;
        }
        let name = entry.file_name().to_string_lossy().into_owned();
        if !name.to_ascii_lowercase().ends_with(".pak") {
            continue;
        }
        // Extract locale token: filename minus ".pak".
        let stem = name.trim_end_matches(|c: char| c != '.').trim_end_matches('.');
        let locale = if stem.is_empty() {
            name.trim_end_matches(".pak").to_string()
        } else {
            stem.to_string()
        };
        let kept = KEEPER_LOCALES
            .iter()
            .any(|k| k.eq_ignore_ascii_case(&locale));
        let size = entry.metadata().map(|m| m.len()).unwrap_or(0);
        let rel = path.strip_prefix(root).unwrap_or(path).to_path_buf();
        out.push(LocaleHit {
            rel_path: rel,
            locale,
            size_bytes: size,
            kept,
        });
    }
    out.sort_by(|a, b| b.size_bytes.cmp(&a.size_bytes));
    out
}

fn build_finding(hits: Vec<LocaleHit>, total_reclaim: u64) -> Finding {
    let drop_count = hits.iter().filter(|h| !h.kept).count();
    let keep_count = hits.iter().filter(|h| h.kept).count();
    let severity = if total_reclaim >= 5 * 1024 * 1024 {
        Severity::Warning
    } else {
        Severity::Info
    };

    let evidence: Vec<Evidence> = hits
        .iter()
        .map(|h| Evidence {
            path: h.rel_path.clone(),
            size_bytes: h.size_bytes,
            note: Some(if h.kept {
                format!("{} (keep)", h.locale)
            } else {
                format!("{}", h.locale)
            }),
        })
        .collect();

    let title = format!(
        "Unused CEF locales: {} across {} file(s)",
        format_bytes(total_reclaim),
        drop_count,
    );

    let summary = format!(
        "Found {} CEF (Chromium Embedded Framework) locale bundle(s) total: \
         {} keeper(s) (English variants), {} reclaimable. CEF is the web view \
         used by UE for in-game browsers and launcher / EOS overlays. Each \
         locale loads only when the user's system language matches it.",
        hits.len(),
        keep_count,
        drop_count,
    );

    let recommendation = "Delete any locale `.pak` outside `en-US` / `en-GB` / \
         `en` if you don't switch the game UI language. The web view falls \
         back to the kept locale at runtime. Easy reclaim with no game-side \
         consequences."
        .to_string();

    Finding {
        detector: "cef_locales".to_string(),
        category: Category::CefLocales,
        severity,
        title,
        summary,
        evidence,
        reclaimable_bytes: Some(total_reclaim),
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
    fn no_finding_when_only_english() {
        let tmp = TempDir::new().unwrap();
        let base = tmp.path().join("Engine/Binaries/ThirdParty/CEF3/Win64/Resources/locales");
        write_file(&base.join("en-US.pak"), 200 * 1024);
        write_file(&base.join("en-GB.pak"), 200 * 1024);
        let d = CefLocalesDetector;
        assert!(d.run(tmp.path()).unwrap().is_empty());
    }

    #[test]
    fn flags_non_english_locales() {
        let tmp = TempDir::new().unwrap();
        let base = tmp.path().join("Engine/Binaries/ThirdParty/CEF3/Win64/Resources/locales");
        write_file(&base.join("en-US.pak"), 300 * 1024);
        write_file(&base.join("de.pak"), 300 * 1024);
        write_file(&base.join("fr.pak"), 300 * 1024);
        write_file(&base.join("ja.pak"), 300 * 1024);
        write_file(&base.join("ko.pak"), 300 * 1024);
        write_file(&base.join("zh-CN.pak"), 300 * 1024);
        let d = CefLocalesDetector;
        let findings = d.run(tmp.path()).unwrap();
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].category, Category::CefLocales);
        // 5 non-en locales × 300 KB = 1.46 MB, comfortably above the 1 MiB threshold.
        assert_eq!(findings[0].reclaimable_bytes, Some(5 * 300 * 1024));
        // Evidence lists all 6 locales (5 to drop + 1 keeper).
        assert_eq!(findings[0].evidence.len(), 6);
    }

    #[test]
    fn ignores_non_cef_pak_files() {
        // A UE pak isn't a CEF locale.
        let tmp = TempDir::new().unwrap();
        write_file(
            &tmp.path().join("Content/Paks/pakchunk0-WindowsNoEditor.pak"),
            5 * 1024 * 1024,
        );
        let d = CefLocalesDetector;
        assert!(d.run(tmp.path()).unwrap().is_empty());
    }

    #[test]
    fn case_insensitive_keeper_matching() {
        let tmp = TempDir::new().unwrap();
        let base = tmp.path().join("Engine/Binaries/ThirdParty/CEF3/Win64/Resources/locales");
        write_file(&base.join("EN-us.pak"), 200 * 1024);
        write_file(&base.join("de.pak"), 600 * 1024);
        write_file(&base.join("ko.pak"), 900 * 1024);
        let d = CefLocalesDetector;
        let findings = d.run(tmp.path()).unwrap();
        assert_eq!(findings.len(), 1);
        // 600K + 900K = 1500 KB reclaimable (above 1 MiB threshold).
        assert_eq!(findings[0].reclaimable_bytes, Some(1500 * 1024));
    }

    #[test]
    fn below_1mb_threshold_ignored() {
        // 5 tiny locales × 100 KB = 500 KB < 1 MB threshold → no finding.
        let tmp = TempDir::new().unwrap();
        let base = tmp.path().join("Engine/Binaries/ThirdParty/CEF3/Win64/Resources/locales");
        write_file(&base.join("en-US.pak"), 100 * 1024);
        for l in ["de", "fr", "ko", "ja", "es"] {
            write_file(&base.join(format!("{}.pak", l)), 100 * 1024);
        }
        let d = CefLocalesDetector;
        assert!(d.run(tmp.path()).unwrap().is_empty());
    }
}
