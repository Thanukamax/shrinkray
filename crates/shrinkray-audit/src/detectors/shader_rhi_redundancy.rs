//! Detect multiple RHI shader caches shipping together.
//!
//! Most cooked games include one shader cache per (graphics API, shader model)
//! combo. A Windows install only consumes one family at runtime — the others
//! are pure ballast. Detection is filename-pattern based; we don't crack the
//! caches open.
//!
//! Conservative: we only flag if at least two families are present AND the
//! non-largest families add up to >= 5 MB. A single-family install is fine.

use super::Detector;
use crate::report::format_bytes;
use crate::types::{Category, Evidence, Finding, Severity};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

/// Substring patterns we look for in shader-related filenames. Order matters
/// only for the "primary" guess: longer / more specific first.
const FAMILY_PATTERNS: &[(&str, &str)] = &[
    ("PCD3D_SM6", "Direct3D SM6"),
    ("PCD3D_SM5", "Direct3D SM5"),
    ("VulkanSM6", "Vulkan SM6"),
    ("VulkanSM5", "Vulkan SM5"),
    ("OpenGLES31", "OpenGL ES 3.1"),
    ("OpenGLES", "OpenGL ES"),
    ("MetalSM5", "Metal SM5"),
    ("MetalSM6", "Metal SM6"),
    ("D3D11", "Direct3D 11"),
];

/// Filename substrings that indicate the file is shader-cache-ish. Combined
/// with a family pattern hit, that's enough to claim it.
const SHADER_FILE_HINTS: &[&str] = &[
    "ShaderCache",
    "ShaderArchive",
    "GlobalShaderCache",
    "upipelinecache",
    "ushaderbytecode",
];

#[derive(Debug, Default)]
pub struct ShaderRhiRedundancyDetector;

impl Detector for ShaderRhiRedundancyDetector {
    fn name(&self) -> &'static str {
        "shader_rhi_redundancy"
    }

    fn run(&self, root: &Path) -> anyhow::Result<Vec<Finding>> {
        let buckets = scan(root);
        if buckets.len() < 2 {
            return Ok(vec![]);
        }
        let total_bytes: u64 = buckets.values().map(|b| b.total_bytes).sum();
        // Reclaimable = total minus the largest family (the presumed primary).
        let largest = buckets.values().map(|b| b.total_bytes).max().unwrap_or(0);
        let reclaimable = total_bytes.saturating_sub(largest);
        if reclaimable < 5 * 1024 * 1024 {
            return Ok(vec![]);
        }
        Ok(vec![build_finding(buckets, reclaimable, largest)])
    }
}

#[derive(Debug, Default)]
struct FamilyBucket {
    label: &'static str,
    files: Vec<(PathBuf, u64)>,
    total_bytes: u64,
}

fn scan(root: &Path) -> BTreeMap<&'static str, FamilyBucket> {
    let mut out: BTreeMap<&'static str, FamilyBucket> = BTreeMap::new();
    for entry in WalkDir::new(root)
        .follow_links(false)
        .into_iter()
        .filter_map(|e| e.ok())
    {
        if !entry.file_type().is_file() {
            continue;
        }
        let name = entry.file_name().to_string_lossy().into_owned();
        let path = entry.path();
        let size = entry.metadata().map(|m| m.len()).unwrap_or(0);
        // A shader-cache-ish filename hint must be present AND a family
        // pattern must match. Both gates keep false positives down.
        let looks_like_shader = SHADER_FILE_HINTS
            .iter()
            .any(|h| name.contains(h));
        if !looks_like_shader {
            continue;
        }
        for (pat, label) in FAMILY_PATTERNS {
            if name.contains(pat) {
                let rel = path.strip_prefix(root).unwrap_or(path).to_path_buf();
                let bucket = out.entry(*pat).or_insert_with(|| FamilyBucket {
                    label,
                    files: Vec::new(),
                    total_bytes: 0,
                });
                bucket.total_bytes = bucket.total_bytes.saturating_add(size);
                bucket.files.push((rel, size));
                break;
            }
        }
    }
    out
}

fn build_finding(
    buckets: BTreeMap<&'static str, FamilyBucket>,
    reclaimable: u64,
    largest: u64,
) -> Finding {
    let severity = if reclaimable >= 200 * 1024 * 1024 {
        Severity::Warning
    } else {
        Severity::Info
    };

    let mut families: Vec<(&'static str, &FamilyBucket)> =
        buckets.iter().map(|(k, v)| (*k, v)).collect();
    families.sort_by(|a, b| b.1.total_bytes.cmp(&a.1.total_bytes));

    let mut evidence: Vec<Evidence> = Vec::new();
    let mut family_summary = Vec::new();
    for (i, (pat, b)) in families.iter().enumerate() {
        let role = if i == 0 { "kept (largest)" } else { "redundant" };
        family_summary.push(format!(
            "{} ({}, {})",
            b.label,
            format_bytes(b.total_bytes),
            role
        ));
        for (path, size) in &b.files {
            evidence.push(Evidence {
                path: path.clone(),
                size_bytes: *size,
                note: Some(format!("{}: {}", b.label, pat)),
            });
        }
    }
    evidence.sort_by(|a, b| b.size_bytes.cmp(&a.size_bytes));

    let title = format!(
        "Shader-cache RHI redundancy: {} extra across {} family/ies",
        format_bytes(reclaimable),
        families.len().saturating_sub(1),
    );

    let summary = format!(
        "Found {} RHI shader-cache families shipping side by side: {}. \
         A given install only runs one family at a time \
         (typically Direct3D SM6 on modern Windows). The non-primary \
         families add up to {} of pure ballast. We assume the \
         largest family is the primary; the rest are reclaimable.",
        families.len(),
        family_summary.join(", "),
        format_bytes(reclaimable),
    );

    let recommendation = format!(
        "Keep the family that matches the target GPU/API (kept here: \
         {}, {}). The other families can be deleted safely — the engine \
         will recompile shaders on first run if it ever needs them. \
         A per-category strip op for this lands in v0.5.",
        families[0].1.label,
        format_bytes(largest),
    );

    Finding {
        detector: "shader_rhi_redundancy".to_string(),
        category: Category::ShaderRhiRedundancy,
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
    fn no_finding_on_single_family() {
        let tmp = TempDir::new().unwrap();
        write_file(
            &tmp.path().join("Engine/GlobalShaderCache-PCD3D_SM6.bin"),
            10 * 1024 * 1024,
        );
        let d = ShaderRhiRedundancyDetector;
        assert!(d.run(tmp.path()).unwrap().is_empty());
    }

    #[test]
    fn flags_sm5_alongside_sm6() {
        let tmp = TempDir::new().unwrap();
        write_file(
            &tmp.path().join("Engine/GlobalShaderCache-PCD3D_SM6.bin"),
            10 * 1024 * 1024,
        );
        write_file(
            &tmp.path().join("Engine/GlobalShaderCache-PCD3D_SM5.bin"),
            8 * 1024 * 1024,
        );
        let d = ShaderRhiRedundancyDetector;
        let findings = d.run(tmp.path()).unwrap();
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].category, Category::ShaderRhiRedundancy);
        assert_eq!(findings[0].reclaimable_bytes, Some(8 * 1024 * 1024));
    }

    #[test]
    fn ignores_below_5mb_threshold() {
        // Two families but the redundant one is tiny — should NOT fire.
        let tmp = TempDir::new().unwrap();
        write_file(
            &tmp.path().join("Engine/GlobalShaderCache-PCD3D_SM6.bin"),
            10 * 1024 * 1024,
        );
        write_file(
            &tmp.path().join("Engine/GlobalShaderCache-VulkanSM5.bin"),
            1 * 1024 * 1024,
        );
        let d = ShaderRhiRedundancyDetector;
        assert!(d.run(tmp.path()).unwrap().is_empty());
    }

    #[test]
    fn ignores_non_shader_files_with_family_in_name() {
        // A random file just named "PCD3D_SM6.txt" should NOT count.
        let tmp = TempDir::new().unwrap();
        write_file(&tmp.path().join("Notes/PCD3D_SM5.txt"), 8 * 1024 * 1024);
        write_file(&tmp.path().join("Notes/PCD3D_SM6.txt"), 8 * 1024 * 1024);
        let d = ShaderRhiRedundancyDetector;
        assert!(d.run(tmp.path()).unwrap().is_empty());
    }

    #[test]
    fn three_families_aggregates_reclaimable() {
        let tmp = TempDir::new().unwrap();
        write_file(
            &tmp.path().join("Engine/GlobalShaderCache-PCD3D_SM6.bin"),
            20 * 1024 * 1024,
        );
        write_file(
            &tmp.path().join("Engine/GlobalShaderCache-PCD3D_SM5.bin"),
            15 * 1024 * 1024,
        );
        write_file(
            &tmp.path().join("Engine/GlobalShaderCache-VulkanSM5.bin"),
            10 * 1024 * 1024,
        );
        let d = ShaderRhiRedundancyDetector;
        let findings = d.run(tmp.path()).unwrap();
        assert_eq!(findings.len(), 1);
        // Total = 45 MB, largest (SM6) = 20 MB, reclaimable = 25 MB.
        assert_eq!(findings[0].reclaimable_bytes, Some(25 * 1024 * 1024));
        // Severity = Info because reclaimable < 200 MB threshold.
        assert_eq!(findings[0].severity, Severity::Info);
    }
}
