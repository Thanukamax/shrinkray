//! Detect byte-identical copies of large files across the install tree.
//!
//! Two-stage detection so we don't hash every file in a 500 GB install:
//!   1. Walk and collect (path, size) for files >= 4 MB.
//!   2. Group by exact size — only size-collisions become hash candidates.
//!   3. SHA-256 each candidate, group by hash.
//!   4. Emit one finding per hash-group of size >= 2.
//!
//! Reclaimable = sum of sizes minus one copy per group.

use super::Detector;
use crate::report::format_bytes;
use crate::types::{Category, Evidence, Finding, Severity};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::fs::File;
use std::io::{BufReader, Read};
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

/// Files smaller than this are ignored. Hashing tiny files is a waste and
/// the per-group reclaimable would be too small to matter.
const MIN_SIZE_BYTES: u64 = 4 * 1024 * 1024;

#[derive(Debug, Default)]
pub struct DuplicateContentDetector;

impl Detector for DuplicateContentDetector {
    fn name(&self) -> &'static str {
        "duplicate_content"
    }

    fn run(&self, root: &Path) -> anyhow::Result<Vec<Finding>> {
        let by_size = group_by_size(root);
        let groups = hash_candidates(by_size);
        if groups.is_empty() {
            return Ok(vec![]);
        }
        Ok(vec![build_finding(root, groups)])
    }
}

#[derive(Debug)]
struct DupGroup {
    /// Hex SHA-256 (lowercased) — used as the group key in the report.
    hash: String,
    /// Files that share this hash.
    paths: Vec<PathBuf>,
    /// Per-file size (all the same; held for convenience).
    size_bytes: u64,
}

fn group_by_size(root: &Path) -> HashMap<u64, Vec<PathBuf>> {
    let mut out: HashMap<u64, Vec<PathBuf>> = HashMap::new();
    for entry in WalkDir::new(root)
        .follow_links(false)
        .into_iter()
        .filter_map(|e| e.ok())
    {
        if !entry.file_type().is_file() {
            continue;
        }
        let size = entry.metadata().map(|m| m.len()).unwrap_or(0);
        if size < MIN_SIZE_BYTES {
            continue;
        }
        out.entry(size).or_default().push(entry.path().to_path_buf());
    }
    out
}

fn hash_candidates(by_size: HashMap<u64, Vec<PathBuf>>) -> Vec<DupGroup> {
    let mut out = Vec::new();
    for (size, paths) in by_size {
        if paths.len() < 2 {
            continue;
        }
        let mut by_hash: HashMap<String, Vec<PathBuf>> = HashMap::new();
        for p in paths {
            if let Ok(h) = sha256_of(&p) {
                by_hash.entry(h).or_default().push(p);
            }
        }
        for (hash, ps) in by_hash {
            if ps.len() >= 2 {
                out.push(DupGroup {
                    hash,
                    paths: ps,
                    size_bytes: size,
                });
            }
        }
    }
    out.sort_by(|a, b| {
        let a_rec = a.size_bytes * (a.paths.len() as u64 - 1);
        let b_rec = b.size_bytes * (b.paths.len() as u64 - 1);
        b_rec.cmp(&a_rec)
    });
    out
}

fn sha256_of(path: &Path) -> anyhow::Result<String> {
    let f = File::open(path)?;
    let mut r = BufReader::with_capacity(1024 * 1024, f);
    let mut hasher = Sha256::new();
    let mut buf = [0u8; 65536];
    loop {
        let n = r.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

fn build_finding(root: &Path, groups: Vec<DupGroup>) -> Finding {
    let total_reclaimable: u64 = groups
        .iter()
        .map(|g| g.size_bytes * (g.paths.len() as u64 - 1))
        .sum();
    let dup_count: usize = groups.iter().map(|g| g.paths.len() - 1).sum();

    let severity = if total_reclaimable >= 100 * 1024 * 1024 {
        Severity::Warning
    } else {
        Severity::Info
    };

    let mut evidence: Vec<Evidence> = Vec::new();
    for g in &groups {
        let mut sorted_paths = g.paths.clone();
        sorted_paths.sort();
        let group_reclaim = g.size_bytes * (g.paths.len() as u64 - 1);
        let short_hash = &g.hash[..16.min(g.hash.len())];
        for (i, p) in sorted_paths.iter().enumerate() {
            let rel = p.strip_prefix(root).unwrap_or(p).to_path_buf();
            let role = if i == 0 { "keep" } else { "duplicate" };
            evidence.push(Evidence {
                path: rel,
                size_bytes: g.size_bytes,
                note: Some(format!(
                    "sha256:{}… ({}) — group reclaim {}",
                    short_hash,
                    role,
                    format_bytes(group_reclaim),
                )),
            });
        }
    }

    let title = format!(
        "Duplicate content: {} across {} group(s), {} extra cop(y/ies)",
        format_bytes(total_reclaimable),
        groups.len(),
        dup_count,
    );

    let summary = format!(
        "Found {} group(s) of byte-identical files ({} total duplicate cop(y/ies)). \
         Each group is hashed with SHA-256 after size pre-filtering, so this is \
         a true content match — not just same-named or same-sized files. \
         Total reclaimable if one copy is kept per group: {}.",
        groups.len(),
        dup_count,
        format_bytes(total_reclaimable),
    );

    let recommendation =
        "For each group, delete all but one copy (or symlink the rest to the \
         survivor on Linux). Common sources: redundant pak backups, mod \
         manager copies of the original asset, multi-target cooked outputs."
            .to_string();

    Finding {
        detector: "duplicate_content".to_string(),
        category: Category::DuplicateContent,
        severity,
        title,
        summary,
        evidence,
        reclaimable_bytes: Some(total_reclaimable),
        recommendation,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::io::Write;
    use tempfile::TempDir;

    fn write_bytes(path: &Path, content: &[u8]) {
        if let Some(p) = path.parent() {
            fs::create_dir_all(p).unwrap();
        }
        fs::File::create(path).unwrap().write_all(content).unwrap();
    }

    #[test]
    fn no_finding_on_unique_files() {
        let tmp = TempDir::new().unwrap();
        write_bytes(&tmp.path().join("a.pak"), &vec![0xAA; 5 * 1024 * 1024]);
        write_bytes(&tmp.path().join("b.pak"), &vec![0xBB; 5 * 1024 * 1024]);
        let d = DuplicateContentDetector;
        assert!(d.run(tmp.path()).unwrap().is_empty());
    }

    #[test]
    fn flags_identical_pair() {
        let tmp = TempDir::new().unwrap();
        let payload = vec![0x42; 5 * 1024 * 1024];
        write_bytes(&tmp.path().join("dir1/a.pak"), &payload);
        write_bytes(&tmp.path().join("dir2/a.pak"), &payload);
        let d = DuplicateContentDetector;
        let findings = d.run(tmp.path()).unwrap();
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].category, Category::DuplicateContent);
        // One extra copy at 5 MB = reclaimable 5 MB.
        assert_eq!(findings[0].reclaimable_bytes, Some(5 * 1024 * 1024));
    }

    #[test]
    fn ignores_below_4mb_threshold() {
        let tmp = TempDir::new().unwrap();
        let payload = vec![0x42; 2 * 1024 * 1024]; // 2 MB < 4 MB
        write_bytes(&tmp.path().join("a.pak"), &payload);
        write_bytes(&tmp.path().join("b.pak"), &payload);
        let d = DuplicateContentDetector;
        assert!(d.run(tmp.path()).unwrap().is_empty());
    }

    #[test]
    fn same_size_different_content_not_flagged() {
        let tmp = TempDir::new().unwrap();
        write_bytes(&tmp.path().join("a.pak"), &vec![0xAA; 5 * 1024 * 1024]);
        write_bytes(&tmp.path().join("b.pak"), &vec![0xBB; 5 * 1024 * 1024]);
        // Sizes collide; SHA differs → no finding.
        let d = DuplicateContentDetector;
        assert!(d.run(tmp.path()).unwrap().is_empty());
    }

    #[test]
    fn three_way_dup_reclaims_two_copies() {
        let tmp = TempDir::new().unwrap();
        let payload = vec![0x42; 5 * 1024 * 1024];
        write_bytes(&tmp.path().join("dir1/a.pak"), &payload);
        write_bytes(&tmp.path().join("dir2/a.pak"), &payload);
        write_bytes(&tmp.path().join("dir3/a.pak"), &payload);
        let d = DuplicateContentDetector;
        let findings = d.run(tmp.path()).unwrap();
        assert_eq!(findings.len(), 1);
        // 3 copies @ 5 MB each, keep one → reclaimable 10 MB.
        assert_eq!(findings[0].reclaimable_bytes, Some(10 * 1024 * 1024));
        // Evidence lists all 3 paths.
        assert_eq!(findings[0].evidence.len(), 3);
    }
}
