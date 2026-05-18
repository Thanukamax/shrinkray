//! Step 1 folder census.
//! Walks the tree once, classifies every file by extension, detects UE L10N
//! language trees, and probes every `.pak` for readability/encryption/signing.
//! All read-only; nothing here writes.

use serde::Serialize;
use std::cmp::Reverse;
use std::collections::{BTreeMap, BinaryHeap};
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

use crate::pak::{self, PakClassification};

const TOP_N: usize = 50;

#[derive(Debug, Serialize, Default, Clone)]
pub struct AssetCategory {
    pub count: usize,
    pub size: u64,
}

#[derive(Debug, Serialize, Clone)]
pub struct FatFile {
    pub path: String,
    pub size: u64,
    pub kind: &'static str,
}

#[derive(Debug, Serialize)]
pub struct UnreadablePak {
    pub path: String,
    pub reason: String,
}

#[derive(Debug, Serialize, Default)]
pub struct PakInventory {
    pub readable: Vec<String>,
    pub signed: Vec<String>,
    pub encrypted: Vec<String>,
    pub unreadable: Vec<UnreadablePak>,
}

#[derive(Debug, Serialize, Default)]
pub struct AnalysisReport {
    pub root: String,
    pub total_files: usize,
    pub total_size: u64,
    pub textures: AssetCategory,
    pub audio: AssetCategory,
    pub paks: AssetCategory,
    pub languages: BTreeMap<String, AssetCategory>,
    pub pak_inventory: PakInventory,
    pub top_files: Vec<FatFile>,
    /// Bytes recoverable by stripping every non-largest detected language.
    /// Defensible baseline: assume the user keeps their primary dub track.
    pub estimated_l10n_savings: u64,
}

pub fn analyze(root: &Path) -> AnalysisReport {
    let mut report = AnalysisReport {
        root: root.to_string_lossy().into_owned(),
        ..Default::default()
    };

    let mut top: BinaryHeap<Reverse<(u64, PathBuf, &'static str)>> =
        BinaryHeap::with_capacity(TOP_N + 1);
    let mut pak_paths: Vec<PathBuf> = Vec::new();

    for entry in WalkDir::new(root).into_iter().filter_map(Result::ok) {
        if !entry.file_type().is_file() {
            continue;
        }
        let abs = entry.path();
        let size = entry.metadata().map(|m| m.len()).unwrap_or(0);
        report.total_files += 1;
        report.total_size += size;

        let ext = abs
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();

        let kind: &'static str = match ext.as_str() {
            "pak" | "utoc" | "ucas" => {
                report.paks.count += 1;
                report.paks.size += size;
                if ext == "pak" {
                    pak_paths.push(abs.to_path_buf());
                }
                "pak"
            }
            "dds" | "png" | "tga" | "bmp" | "jpg" | "jpeg" => {
                report.textures.count += 1;
                report.textures.size += size;
                "texture"
            }
            "wav" | "ogg" | "opus" | "mp3" | "flac" => {
                report.audio.count += 1;
                report.audio.size += size;
                "audio"
            }
            _ => "other",
        };

        if let Some(lang) = detect_language(abs) {
            let cat = report.languages.entry(lang).or_default();
            cat.count += 1;
            cat.size += size;
        }

        top.push(Reverse((size, abs.to_path_buf(), kind)));
        if top.len() > TOP_N {
            top.pop();
        }
    }

    // BinaryHeap<Reverse<T>>::into_sorted_vec is ascending by Reverse, which is
    // descending by T — so this is already largest-first.
    let top_sorted: Vec<_> = top.into_sorted_vec();
    report.top_files = top_sorted
        .into_iter()
        .map(|Reverse((size, p, kind))| FatFile {
            path: rel_path(&p, root),
            size,
            kind,
        })
        .collect();

    for pak_path in &pak_paths {
        let rel = rel_path(pak_path, root);
        match pak::classify_pak(pak_path) {
            PakClassification::Readable => report.pak_inventory.readable.push(rel),
            PakClassification::Signed => report.pak_inventory.signed.push(rel),
            PakClassification::Encrypted => report.pak_inventory.encrypted.push(rel),
            PakClassification::Unreadable { reason } => {
                report.pak_inventory.unreadable.push(UnreadablePak { path: rel, reason })
            }
        }
    }

    report.estimated_l10n_savings = compute_l10n_savings(&report.languages);

    report
}

fn rel_path(p: &Path, root: &Path) -> String {
    p.strip_prefix(root)
        .unwrap_or(p)
        .to_string_lossy()
        .into_owned()
}

/// Returns the detected language code for a path containing `L10N/<lang>/...`
/// or `Localization/<target>/<lang>/...`. Returns None if no L10N segment or no
/// well-formed language code follows.
fn detect_language(path: &Path) -> Option<String> {
    let mut in_l10n = false;
    for comp in path.iter().filter_map(|c| c.to_str()) {
        let lc = comp.to_ascii_lowercase();
        if lc == "l10n" || lc == "localization" {
            in_l10n = true;
            continue;
        }
        if in_l10n && is_language_code(comp) {
            return Some(normalize_lang(comp));
        }
    }
    None
}

/// Matches BCP-47-ish UE language codes. UE shipped paths use lowercase primary
/// subtags (`en`, `fr`, `zh-Hans`, `en-US`), so we require lowercase — that
/// alone rejects the most common false positives like `UI`, `VR`, `XR`.
fn is_language_code(s: &str) -> bool {
    let parts: Vec<&str> = s.split('-').collect();
    let primary_ok = |p: &str| {
        (2..=3).contains(&p.len()) && p.chars().all(|c| c.is_ascii_lowercase())
    };
    let region_ok = |p: &str| {
        (2..=4).contains(&p.len()) && p.chars().all(|c| c.is_ascii_alphabetic())
    };
    match parts.len() {
        1 => primary_ok(parts[0]),
        2 => primary_ok(parts[0]) && region_ok(parts[1]),
        _ => false,
    }
}

/// Normalizes the region/script subtag casing (`en-us` -> `en-US`,
/// `zh-hans` -> `zh-Hans`). Primary subtag is already lowercase per the
/// `is_language_code` precondition.
fn normalize_lang(s: &str) -> String {
    let parts: Vec<&str> = s.split('-').collect();
    match parts.len() {
        1 => parts[0].to_string(),
        2 => {
            let region: String = if parts[1].len() == 4 {
                // Script subtag (Hans, Hant) — title case
                parts[1]
                    .chars()
                    .enumerate()
                    .map(|(i, c)| {
                        if i == 0 {
                            c.to_ascii_uppercase()
                        } else {
                            c.to_ascii_lowercase()
                        }
                    })
                    .collect()
            } else {
                // Region subtag (US, GB, BR) — all upper
                parts[1].to_ascii_uppercase()
            };
            format!("{}-{}", parts[0], region)
        }
        _ => s.to_string(),
    }
}

fn compute_l10n_savings(languages: &BTreeMap<String, AssetCategory>) -> u64 {
    if languages.len() < 2 {
        return 0;
    }
    let total: u64 = languages.values().map(|c| c.size).sum();
    let largest: u64 = languages.values().map(|c| c.size).max().unwrap_or(0);
    total.saturating_sub(largest)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn detect_language_l10n_audio() {
        let p = PathBuf::from("Game/Content/L10N/fr/Audio/voice.uasset");
        assert_eq!(detect_language(&p).as_deref(), Some("fr"));
    }

    #[test]
    fn detect_language_localization_text() {
        let p = PathBuf::from("Game/Content/Localization/Game/en-US/Game.locres");
        assert_eq!(detect_language(&p).as_deref(), Some("en-US"));
    }

    #[test]
    fn detect_language_script_subtag() {
        let p = PathBuf::from("Game/Content/L10N/zh-Hans/ui.uasset");
        assert_eq!(detect_language(&p).as_deref(), Some("zh-Hans"));
    }

    #[test]
    fn detect_language_normalizes_region_case() {
        // UE convention is lowercase primary subtag, region casing varies.
        let p = PathBuf::from("Game/Content/L10N/en-us/file.uasset");
        assert_eq!(detect_language(&p).as_deref(), Some("en-US"));
    }

    #[test]
    fn detect_language_ignores_non_codes() {
        let p = PathBuf::from("Game/Content/L10N/Audio/voice.uasset");
        // "Audio" is 5 chars - not a primary subtag
        assert_eq!(detect_language(&p), None);
    }

    #[test]
    fn detect_language_returns_none_outside_l10n() {
        let p = PathBuf::from("Game/Content/Audio/en/voice.uasset");
        assert_eq!(detect_language(&p), None);
    }

    #[test]
    fn is_language_code_accepts_common_forms() {
        assert!(is_language_code("en"));
        assert!(is_language_code("fr"));
        assert!(is_language_code("ja"));
        assert!(is_language_code("zh-Hans"));
        assert!(is_language_code("en-US"));
        assert!(is_language_code("pt-BR"));
    }

    #[test]
    fn is_language_code_rejects_directory_names() {
        assert!(!is_language_code("Audio"));     // title-cased
        assert!(!is_language_code("UI"));        // uppercase
        assert!(!is_language_code("VR"));        // uppercase
        assert!(!is_language_code("Sources"));   // too long + title-cased
        assert!(!is_language_code("Game"));      // too long
        assert!(!is_language_code("InterchangeAssets")); // way too long
        assert!(!is_language_code("en_US"));     // wrong separator
        assert!(!is_language_code("e"));         // too short
        assert!(!is_language_code("EN"));        // primary subtag must be lowercase
    }

    #[test]
    fn compute_l10n_savings_keeps_largest() {
        let mut m = BTreeMap::new();
        m.insert("en".to_string(), AssetCategory { count: 10, size: 1000 });
        m.insert("fr".to_string(), AssetCategory { count: 10, size: 600 });
        m.insert("de".to_string(), AssetCategory { count: 10, size: 400 });
        assert_eq!(compute_l10n_savings(&m), 1000);
    }

    #[test]
    fn compute_l10n_savings_zero_with_single_lang() {
        let mut m = BTreeMap::new();
        m.insert("en".to_string(), AssetCategory { count: 5, size: 500 });
        assert_eq!(compute_l10n_savings(&m), 0);
    }

    #[test]
    fn compute_l10n_savings_zero_when_empty() {
        let m = BTreeMap::new();
        assert_eq!(compute_l10n_savings(&m), 0);
    }

    #[test]
    fn analyze_walks_real_tree() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();

        // Build a synthetic UE-ish layout.
        let l10n_fr = root.join("MyGame/Content/L10N/fr");
        let l10n_en = root.join("MyGame/Content/L10N/en");
        let assets = root.join("MyGame/Content/Assets");
        std::fs::create_dir_all(&l10n_fr).unwrap();
        std::fs::create_dir_all(&l10n_en).unwrap();
        std::fs::create_dir_all(&assets).unwrap();
        std::fs::write(l10n_fr.join("voice.ogg"), vec![0u8; 2000]).unwrap();
        std::fs::write(l10n_en.join("voice.ogg"), vec![0u8; 1000]).unwrap();
        std::fs::write(assets.join("hero.png"), vec![0u8; 500]).unwrap();

        let r = analyze(root);
        assert_eq!(r.total_files, 3);
        assert_eq!(r.total_size, 3500);
        assert_eq!(r.audio.count, 2);
        assert_eq!(r.audio.size, 3000);
        assert_eq!(r.textures.count, 1);
        assert_eq!(r.textures.size, 500);
        assert_eq!(r.languages.len(), 2);
        assert_eq!(r.languages["fr"].size, 2000);
        assert_eq!(r.languages["en"].size, 1000);
        assert_eq!(r.estimated_l10n_savings, 1000); // strip en, keep fr (largest)
        assert!(!r.top_files.is_empty());
        assert_eq!(r.top_files[0].size, 2000);
    }
}
