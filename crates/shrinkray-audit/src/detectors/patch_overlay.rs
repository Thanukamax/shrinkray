//! Detect `_P.pak` patch overlay accumulation.
//!
//! In UE's pak system a `_P.pak` file with the same chunk number as a base
//! pak shadows matching entries in the base — meaning bytes inside the base
//! are now zombies (still on disk, never loaded). Live-service titles
//! accumulate these overlays across every shipped patch.
//!
//! Filesystem signal only: we cannot enumerate which specific entries are
//! shadowed without decrypting + reading both indexes, and many live-service
//! paks are AES-encrypted. So we report on the *quantity* of overlay and
//! estimate the lower-bound zombie content as 50% of overlay size — a
//! conservative floor; the actual figure is between 50% and 100% of the
//! overlay total.

use super::Detector;
use crate::types::{Category, Evidence, Finding, Severity};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

#[derive(Debug, Default)]
pub struct PatchOverlayDetector;

impl Detector for PatchOverlayDetector {
    fn name(&self) -> &'static str {
        "patch_overlay"
    }

    fn run(&self, root: &Path) -> anyhow::Result<Vec<Finding>> {
        let inventory = walk_pak_inventory(root);
        let chunks = group_by_chunk(inventory);

        // Keep only chunks that have BOTH a base and at least one patch.
        let overlays: Vec<(String, ChunkGroup)> = chunks
            .into_iter()
            .filter(|(_, g)| g.base.is_some() && !g.patches.is_empty())
            .collect();

        if overlays.is_empty() {
            return Ok(vec![]);
        }

        Ok(vec![build_finding(root, overlays)])
    }
}

#[derive(Debug)]
struct PakEntry {
    path: PathBuf,
    rel_path: PathBuf,
    chunk_id: String,
    is_patch: bool,
    size_bytes: u64,
}

#[derive(Debug, Default)]
struct ChunkGroup {
    base: Option<PakEntry>,
    patches: Vec<PakEntry>,
}

fn walk_pak_inventory(root: &Path) -> Vec<PakEntry> {
    let mut out = Vec::new();
    for entry in WalkDir::new(root)
        .follow_links(false)
        .into_iter()
        .filter_map(|e| e.ok())
    {
        if !entry.file_type().is_file() {
            continue;
        }
        let fname = match entry.file_name().to_str() {
            Some(s) => s,
            None => continue,
        };
        let Some((chunk_id, is_patch)) = parse_pak_filename(fname) else {
            continue;
        };
        let size_bytes = entry.metadata().map(|m| m.len()).unwrap_or(0);
        let path = entry.path().to_path_buf();
        let rel_path = path.strip_prefix(root).unwrap_or(&path).to_path_buf();
        out.push(PakEntry {
            path,
            rel_path,
            chunk_id,
            is_patch,
            size_bytes,
        });
    }
    out
}

/// Parse a pak filename. Returns (chunk_id, is_patch) when the file matches
/// the `pakchunk{ID}-{platform}[_P].pak` convention, else None.
///
/// Examples:
/// - `pakchunk70-WindowsNoEditor.pak` → ("70", false)
/// - `pakchunk70-WindowsNoEditor_P.pak` → ("70", true)
/// - `pakchunk0optional-WindowsNoEditor.pak` → ("0optional", false)
/// - `Video_105_1-WindowsNoEditor.pak` → None (not a chunk pak)
/// - `random.pak` → None
fn parse_pak_filename(fname: &str) -> Option<(String, bool)> {
    let stem = fname.strip_suffix(".pak")?;
    let stem = stem.strip_prefix("pakchunk")?;
    let (chunk_part, rest) = stem.split_once('-')?;
    if chunk_part.is_empty() {
        return None;
    }
    let is_patch = rest.ends_with("_P");
    Some((chunk_part.to_string(), is_patch))
}

fn group_by_chunk(entries: Vec<PakEntry>) -> BTreeMap<String, ChunkGroup> {
    let mut groups: BTreeMap<String, ChunkGroup> = BTreeMap::new();
    for e in entries {
        let g = groups.entry(e.chunk_id.clone()).or_default();
        if e.is_patch {
            g.patches.push(e);
        } else {
            // If we somehow see two base paks for the same chunk (e.g. one in
            // Content/Paks and one elsewhere) keep the first; treat the later
            // one as if it were also a patch overlay candidate.
            if g.base.is_none() {
                g.base = Some(e);
            } else {
                g.patches.push(e);
            }
        }
    }
    groups
}

fn build_finding(root: &Path, overlays: Vec<(String, ChunkGroup)>) -> Finding {
    let mut total_patch_bytes: u64 = 0;
    let mut total_base_with_patch_bytes: u64 = 0;
    let mut max_overlay_ratio: f64 = 0.0;
    let mut evidence: Vec<Evidence> = Vec::new();
    let chunk_count = overlays.len();

    for (chunk_id, group) in &overlays {
        let base = group.base.as_ref().expect("filtered to have base");
        let patch_total: u64 = group.patches.iter().map(|p| p.size_bytes).sum();
        let ratio = if base.size_bytes > 0 {
            patch_total as f64 / base.size_bytes as f64
        } else {
            0.0
        };
        if ratio > max_overlay_ratio {
            max_overlay_ratio = ratio;
        }
        total_patch_bytes += patch_total;
        total_base_with_patch_bytes += base.size_bytes;

        evidence.push(Evidence {
            path: base.rel_path.clone(),
            size_bytes: base.size_bytes,
            note: Some(format!(
                "chunk {} base · {:.0}% overlay",
                chunk_id,
                ratio * 100.0
            )),
        });
        for p in &group.patches {
            evidence.push(Evidence {
                path: p.rel_path.clone(),
                size_bytes: p.size_bytes,
                note: Some(format!("chunk {} overlay", chunk_id)),
            });
        }
    }

    // Conservative lower bound: 50% of overlay total is zombie content in the
    // base paks. Actual range is 50-100% but we can't measure exactly without
    // decrypting + diffing pak indexes.
    let reclaimable = total_patch_bytes / 2;

    let severity = if max_overlay_ratio >= 0.4 {
        Severity::Critical
    } else {
        Severity::Warning
    };

    let title = format!(
        "Patch overlay accumulation: {} of overlay across {} chunks",
        super::super::report::format_bytes(total_patch_bytes),
        chunk_count
    );

    let summary = format!(
        "{} `_P.pak` overlay file(s) shadow matching entries in {} base pak(s) totalling \
         {}. UE's pak system loads `_P` overlays at higher priority than the base, so \
         shadowed entries in the base never run — but the bytes stay on disk. \
         Estimated zombie content in base paks: {} to {} (lower bound to upper bound). \
         The largest overlay ratio observed is {:.0}% of its base chunk's bytes.",
        overlays.iter().map(|(_, g)| g.patches.len()).sum::<usize>(),
        chunk_count,
        super::super::report::format_bytes(total_base_with_patch_bytes),
        super::super::report::format_bytes(reclaimable),
        super::super::report::format_bytes(total_patch_bytes),
        max_overlay_ratio * 100.0,
    );

    let recommendation = if matches!(severity, Severity::Critical) {
        "A consolidated re-cook of affected base paks would eliminate this. \
         For published games this requires a one-time forced re-download for \
         existing players; the long-term storage win usually justifies it. \
         Third-party tools cannot fix this — only the publisher's cook pipeline \
         can rebuild the base paks safely."
            .to_string()
    } else {
        "Track this metric over patch versions. Once overlay accumulation crosses \
         ~30% of base chunk bytes, plan a base re-cook with the next major patch."
            .to_string()
    };

    let _ = root; // not used in this finding, future detectors may take env config
    Finding {
        detector: "patch_overlay".to_string(),
        category: Category::PatchOverlay,
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

    fn write_pak(dir: &Path, name: &str, size_bytes: u64) {
        fs::create_dir_all(dir).unwrap();
        let p = dir.join(name);
        let mut f = fs::File::create(&p).unwrap();
        // Write `size_bytes` zero bytes — content doesn't matter for the
        // detector, only metadata.len().
        f.write_all(&vec![0u8; size_bytes as usize]).unwrap();
    }

    #[test]
    fn parses_base_pak_name() {
        assert_eq!(
            parse_pak_filename("pakchunk70-WindowsNoEditor.pak"),
            Some(("70".to_string(), false))
        );
    }

    #[test]
    fn parses_patch_pak_name() {
        assert_eq!(
            parse_pak_filename("pakchunk70-WindowsNoEditor_P.pak"),
            Some(("70".to_string(), true))
        );
    }

    #[test]
    fn parses_optional_chunk() {
        assert_eq!(
            parse_pak_filename("pakchunk0optional-WindowsNoEditor.pak"),
            Some(("0optional".to_string(), false))
        );
    }

    #[test]
    fn rejects_non_pakchunk_files() {
        assert_eq!(parse_pak_filename("Video_105_1-WindowsNoEditor.pak"), None);
        assert_eq!(parse_pak_filename("random.pak"), None);
        assert_eq!(parse_pak_filename("pakchunk0.pak"), None); // no `-platform` segment
        assert_eq!(parse_pak_filename("pakchunk-WindowsNoEditor.pak"), None); // empty id
    }

    #[test]
    fn no_finding_when_no_overlays() {
        let tmp = TempDir::new().unwrap();
        let paks = tmp.path().join("Content/Paks");
        write_pak(&paks, "pakchunk0-WindowsNoEditor.pak", 4096);
        write_pak(&paks, "pakchunk1-WindowsNoEditor.pak", 4096);

        let d = PatchOverlayDetector;
        let findings = d.run(tmp.path()).unwrap();
        assert!(findings.is_empty(), "no overlays → no finding");
    }

    #[test]
    fn detects_single_overlay_pair() {
        let tmp = TempDir::new().unwrap();
        let paks = tmp.path().join("Content/Paks");
        let overlays = tmp.path().join("Saved/Resources/3.3.0/Resource/3.3.11");
        write_pak(&paks, "pakchunk7-WindowsNoEditor.pak", 8 * 1024); // base
        write_pak(&overlays, "pakchunk7-WindowsNoEditor_P.pak", 2 * 1024); // 25% overlay

        let d = PatchOverlayDetector;
        let findings = d.run(tmp.path()).unwrap();
        assert_eq!(findings.len(), 1);
        let f = &findings[0];
        assert_eq!(f.category, Category::PatchOverlay);
        assert_eq!(f.severity, Severity::Warning); // 25% < 40% critical threshold
        assert_eq!(f.reclaimable_bytes, Some(1024)); // 50% of 2KB overlay
        assert_eq!(f.evidence.len(), 2);
    }

    #[test]
    fn flags_critical_when_overlay_ratio_high() {
        let tmp = TempDir::new().unwrap();
        let paks = tmp.path().join("Content/Paks");
        let overlays = tmp.path().join("Saved/Resources/3.3.0/Resource/3.3.11");
        write_pak(&paks, "pakchunk0-WindowsNoEditor.pak", 5 * 1024); // base 5KB
        write_pak(&overlays, "pakchunk0-WindowsNoEditor_P.pak", 3 * 1024); // 60% overlay

        let d = PatchOverlayDetector;
        let findings = d.run(tmp.path()).unwrap();
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].severity, Severity::Critical);
    }

    #[test]
    fn aggregates_multiple_chunks_into_one_finding() {
        let tmp = TempDir::new().unwrap();
        let paks = tmp.path().join("Content/Paks");
        let overlays = tmp.path().join("Saved/Resources/3.3.0/Resource/3.3.11");
        write_pak(&paks, "pakchunk0-WindowsNoEditor.pak", 5 * 1024);
        write_pak(&paks, "pakchunk7-WindowsNoEditor.pak", 8 * 1024);
        write_pak(&paks, "pakchunk53-WindowsNoEditor.pak", 6 * 1024);
        // Two of three chunks have overlays:
        write_pak(&overlays, "pakchunk0-WindowsNoEditor_P.pak", 3 * 1024);
        write_pak(&overlays, "pakchunk7-WindowsNoEditor_P.pak", 1 * 1024);

        let d = PatchOverlayDetector;
        let findings = d.run(tmp.path()).unwrap();
        assert_eq!(findings.len(), 1, "all overlays → single combined finding");
        let f = &findings[0];
        assert!(f.title.contains("2 chunks"), "title was: {}", f.title);
        assert_eq!(f.reclaimable_bytes, Some(2 * 1024)); // 50% of (3KB + 1KB) total overlay
        assert_eq!(f.severity, Severity::Critical, "chunk 0 hits 60% ratio");
    }

    #[test]
    fn skips_orphan_patches_without_base() {
        // Patch present but no base — typical when a base chunk gets fully
        // removed by a patch (unusual but possible). We don't flag it; another
        // detector could pick it up as "orphan patch", but it's not zombie
        // content because there's no base to be dead-weight in.
        let tmp = TempDir::new().unwrap();
        let overlays = tmp.path().join("Saved/Resources/3.3.0/Resource/3.3.11");
        write_pak(&overlays, "pakchunk99-WindowsNoEditor_P.pak", 1024);

        let d = PatchOverlayDetector;
        let findings = d.run(tmp.path()).unwrap();
        assert!(findings.is_empty());
    }

    #[test]
    fn handles_multiple_patch_files_per_chunk() {
        // Some setups accumulate multiple _P.pak generations for one base.
        let tmp = TempDir::new().unwrap();
        let paks = tmp.path().join("Content/Paks");
        let overlay_a = tmp.path().join("Saved/Resources/3.3.0/Resource/3.3.10");
        let overlay_b = tmp.path().join("Saved/Resources/3.3.0/Resource/3.3.11");
        write_pak(&paks, "pakchunk5-WindowsNoEditor.pak", 10 * 1024);
        write_pak(&overlay_a, "pakchunk5-WindowsNoEditor_P.pak", 2 * 1024);
        write_pak(&overlay_b, "pakchunk5-WindowsNoEditor_P.pak", 3 * 1024);

        let d = PatchOverlayDetector;
        let findings = d.run(tmp.path()).unwrap();
        assert_eq!(findings.len(), 1);
        let f = &findings[0];
        // Total overlay = 2KB + 3KB = 5KB, reclaimable = 50% = 2560 bytes
        assert_eq!(f.reclaimable_bytes, Some(2560));
        // Evidence: 1 base + 2 overlays = 3 items
        assert_eq!(f.evidence.len(), 3);
    }
}
