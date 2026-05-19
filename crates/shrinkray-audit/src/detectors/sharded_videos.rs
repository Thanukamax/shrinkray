//! Detect fragmented pak collections — many small pak files where one
//! consolidated archive would be tighter.
//!
//! Pattern observed in Wuthering Waves: `Saved/Resources/Video/Paks/`
//! contains 124 subdirectories each holding a single ~100-500 MB pak file.
//! Sharding aids parallel patch downloads but costs filesystem overhead
//! (124 .sig files, 124 inodes, 124× pak header/footer alignment padding)
//! and prevents cross-archive compression dedup.
//!
//! Heuristic: a directory is "fragmented" when its immediate children
//! include 20+ subdirectories that each contain exactly one `.pak` file
//! AND the average pak size is below 600 MB. The 20-subdir threshold
//! avoids flagging normal pak layouts (Content/Paks/ typically has a
//! couple dozen paks but as direct children, not sharded subdirectories).

use super::Detector;
use crate::report::format_bytes;
use crate::types::{Category, Evidence, Finding, Severity};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

const MIN_SHARD_COUNT: usize = 20;
const MAX_MEAN_SHARD_BYTES: u64 = 600 * 1024 * 1024;
const RECLAIMABLE_FRACTION: f64 = 0.05;

#[derive(Debug, Default)]
pub struct ShardedVideosDetector;

impl Detector for ShardedVideosDetector {
    fn name(&self) -> &'static str {
        "sharded_videos"
    }

    fn run(&self, root: &Path) -> anyhow::Result<Vec<Finding>> {
        let collections = find_sharded_collections(root);
        if collections.is_empty() {
            return Ok(vec![]);
        }
        Ok(vec![build_finding(root, collections)])
    }
}

#[derive(Debug)]
struct ShardCollection {
    parent: PathBuf,
    shard_count: usize,
    total_bytes: u64,
}

fn find_sharded_collections(root: &Path) -> Vec<ShardCollection> {
    // For each directory in the tree, count how many direct subdirectories
    // contain at least one .pak file, and sum the pak bytes across them.
    let mut by_parent: BTreeMap<PathBuf, (usize, u64)> = BTreeMap::new();

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
        let pak_bytes = direct_pak_bytes(dir);
        if pak_bytes == 0 {
            continue;
        }
        // This dir is a shard candidate. Count it under its parent.
        let Some(parent) = dir.parent() else { continue };
        let entry = by_parent.entry(parent.to_path_buf()).or_insert((0, 0));
        entry.0 += 1;
        entry.1 = entry.1.saturating_add(pak_bytes);
    }

    by_parent
        .into_iter()
        .filter_map(|(parent, (count, total_bytes))| {
            if count < MIN_SHARD_COUNT {
                return None;
            }
            let mean = total_bytes / (count as u64).max(1);
            if mean > MAX_MEAN_SHARD_BYTES {
                return None;
            }
            Some(ShardCollection {
                parent,
                shard_count: count,
                total_bytes,
            })
        })
        .collect()
}

/// Bytes of `.pak` files in this directory (non-recursive — direct children
/// only).
fn direct_pak_bytes(dir: &Path) -> u64 {
    let Ok(read_dir) = fs::read_dir(dir) else {
        return 0;
    };
    let mut total: u64 = 0;
    for ent in read_dir.flatten() {
        let path = ent.path();
        if !path.is_file() {
            continue;
        }
        if path.extension().and_then(|s| s.to_str()) != Some("pak") {
            continue;
        }
        if let Ok(md) = ent.metadata() {
            total = total.saturating_add(md.len());
        }
    }
    total
}

fn build_finding(root: &Path, collections: Vec<ShardCollection>) -> Finding {
    let total_bytes: u64 = collections.iter().map(|c| c.total_bytes).sum();
    let total_shards: usize = collections.iter().map(|c| c.shard_count).sum();
    let reclaimable = (total_bytes as f64 * RECLAIMABLE_FRACTION) as u64;

    let evidence: Vec<Evidence> = collections
        .iter()
        .map(|c| Evidence {
            path: c.parent.strip_prefix(root).unwrap_or(&c.parent).to_path_buf(),
            size_bytes: c.total_bytes,
            note: Some(format!(
                "{} shard dirs, mean {}/shard",
                c.shard_count,
                format_bytes(c.total_bytes / c.shard_count as u64)
            )),
        })
        .collect();

    let title = format!(
        "Fragmented pak collection: {} across {} shards in {} location(s)",
        format_bytes(total_bytes),
        total_shards,
        collections.len()
    );

    let summary = format!(
        "Found {} location(s) where pak files are sharded across many \
         subdirectories (≥{} shards per parent, mean shard size under {}). \
         Each shard carries its own .sig file, pak header/footer, and \
         alignment padding. Consolidating into one archive per location \
         removes that overhead and can compress better across the corpus. \
         Estimated savings (conservative): {} of {} total.",
        collections.len(),
        MIN_SHARD_COUNT,
        format_bytes(MAX_MEAN_SHARD_BYTES),
        format_bytes(reclaimable),
        format_bytes(total_bytes),
    );

    let recommendation = "Re-pack the sharded collections into one .pak per \
         location during the next cook pass. Requires publisher cooperation \
         — third-party tools shouldn't repackage shipped paks because matching \
         AES keys + integrity hashes break."
        .to_string();

    Finding {
        detector: "sharded_videos".to_string(),
        category: Category::ShardedVideos,
        severity: Severity::Warning,
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

    fn write_pak(path: &Path, size_bytes: u64) {
        if let Some(p) = path.parent() {
            fs::create_dir_all(p).unwrap();
        }
        // Sparse allocation so multi-GB test cases don't fill the disk.
        // Detector reads metadata().len(), which honours the logical size.
        fs::File::create(path).unwrap().set_len(size_bytes).unwrap();
    }

    #[test]
    fn no_finding_when_no_sharding() {
        let tmp = TempDir::new().unwrap();
        write_pak(&tmp.path().join("Content/Paks/pakchunk0.pak"), 1024);
        write_pak(&tmp.path().join("Content/Paks/pakchunk1.pak"), 1024);

        let d = ShardedVideosDetector;
        let findings = d.run(tmp.path()).unwrap();
        assert!(findings.is_empty());
    }

    #[test]
    fn no_finding_below_threshold() {
        // 19 shard dirs — under MIN_SHARD_COUNT = 20.
        let tmp = TempDir::new().unwrap();
        for i in 0..19 {
            write_pak(
                &tmp.path().join(format!("Video/Paks/{}/v.pak", i)),
                100,
            );
        }
        let d = ShardedVideosDetector;
        let findings = d.run(tmp.path()).unwrap();
        assert!(findings.is_empty());
    }

    #[test]
    fn flags_at_threshold_with_small_shards() {
        // 25 shard dirs, each with one 100-byte pak → fragmented.
        let tmp = TempDir::new().unwrap();
        for i in 0..25 {
            write_pak(
                &tmp.path().join(format!("Video/Paks/{}/v.pak", i)),
                100,
            );
        }
        let d = ShardedVideosDetector;
        let findings = d.run(tmp.path()).unwrap();
        assert_eq!(findings.len(), 1);
        let f = &findings[0];
        assert_eq!(f.category, Category::ShardedVideos);
        assert_eq!(f.severity, Severity::Warning);
        assert!(f.title.contains("25 shards"));
        // Total = 25 * 100 = 2500. Reclaimable = 5% = 125.
        assert_eq!(f.reclaimable_bytes, Some(125));
    }

    #[test]
    fn ignores_when_shards_are_huge() {
        // 25 shards but each is 1 GB → mean exceeds MAX_MEAN_SHARD_BYTES (600 MB)
        // → these are real big paks, not "fragmented small chunks".
        let tmp = TempDir::new().unwrap();
        for i in 0..25 {
            write_pak(
                &tmp.path().join(format!("Big/Paks/{}/v.pak", i)),
                1024 * 1024 * 1024,
            );
        }
        let d = ShardedVideosDetector;
        let findings = d.run(tmp.path()).unwrap();
        assert!(
            findings.is_empty(),
            "huge per-shard paks aren't fragmentation"
        );
    }

    #[test]
    fn aggregates_multiple_sharded_locations() {
        let tmp = TempDir::new().unwrap();
        for i in 0..25 {
            write_pak(&tmp.path().join(format!("A/{}/v.pak", i)), 100);
            write_pak(&tmp.path().join(format!("B/{}/v.pak", i)), 200);
        }
        let d = ShardedVideosDetector;
        let findings = d.run(tmp.path()).unwrap();
        assert_eq!(findings.len(), 1, "one combined finding");
        let f = &findings[0];
        assert_eq!(f.evidence.len(), 2, "one evidence per sharded location");
        // 25*100 + 25*200 = 7500. Reclaimable 5% = 375.
        assert_eq!(f.reclaimable_bytes, Some(375));
    }
}
