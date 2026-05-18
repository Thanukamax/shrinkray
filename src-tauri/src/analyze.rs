//! Phase 0 folder census.
//! Walks the tree once, classifies every file by extension, sums sizes per
//! category. The savings estimate uses a fixed per-category multiplier based on
//! published recompression ratios — refined in Phase 1 with per-asset
//! introspection.

use serde::Serialize;
use std::path::Path;
use walkdir::WalkDir;

#[derive(Debug, Serialize, Default)]
pub struct AssetCategory {
    pub count: usize,
    pub size: u64,
}

#[derive(Debug, Serialize, Default)]
pub struct AnalysisReport {
    pub total_files: usize,
    pub total_size: u64,
    pub textures: AssetCategory,
    pub audio: AssetCategory,
    pub paks: AssetCategory,
    pub estimated_savings: u64,
}

/// Multipliers reflect realistic-but-conservative savings:
/// textures BC1/3 → BC7/ASTC + mipmap strip → ~40% off.
/// Audio WAV/Vorbis → Opus → ~50% off.
/// Pak repack zlib → zstd → ~10% off.
const TEXTURE_RATIO: f64 = 0.40;
const AUDIO_RATIO:   f64 = 0.50;
const PAK_RATIO:     f64 = 0.10;

pub fn analyze(path: &Path) -> AnalysisReport {
    let mut report = AnalysisReport::default();

    for entry in WalkDir::new(path).into_iter().filter_map(Result::ok) {
        if !entry.file_type().is_file() {
            continue;
        }
        let size = entry.metadata().map(|m| m.len()).unwrap_or(0);
        report.total_files += 1;
        report.total_size += size;

        let ext = entry
            .path()
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();

        match ext.as_str() {
            "pak" | "utoc" | "ucas" => {
                report.paks.count += 1;
                report.paks.size += size;
            }
            "dds" | "png" | "tga" | "bmp" | "jpg" | "jpeg" => {
                report.textures.count += 1;
                report.textures.size += size;
            }
            "wav" | "ogg" | "opus" | "mp3" | "flac" => {
                report.audio.count += 1;
                report.audio.size += size;
            }
            _ => {}
        }
    }

    report.estimated_savings = (report.textures.size as f64 * TEXTURE_RATIO
        + report.audio.size   as f64 * AUDIO_RATIO
        + report.paks.size    as f64 * PAK_RATIO) as u64;

    report
}
