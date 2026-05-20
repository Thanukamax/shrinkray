//! Core audit types: severity, category, finding, evidence, report.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;

/// How serious a finding is.
///
/// `Info`     — observation, no action required (e.g. encryption is normal).
/// `Warning`  — addressable bloat present.
/// `Critical` — major structural waste or design flaw blocking optimization.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Info,
    Warning,
    Critical,
}

impl Severity {
    pub fn label(self) -> &'static str {
        match self {
            Severity::Info => "info",
            Severity::Warning => "warning",
            Severity::Critical => "critical",
        }
    }
}

/// Semantic bucket a finding falls into. Used to group the report and to
/// roll up reclaimable bytes per category.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Category {
    /// `_P.pak` overlay accumulation creating zombie content in base paks.
    PatchOverlay,
    /// Old version directories left after patches (`Lang_*/3.3.9/` while
    /// current is `3.3.11`, `Video/3.2.0/` stubs, `launcherDownload/3.1.0/`).
    StaleVersionDir,
    /// Many small video paks where one consolidated archive would be tighter.
    ShardedVideos,
    /// Single pak chunks well above the ~1-2 GB target — bad for patch cost.
    LargeChunk,
    /// AES encryption status — locks third-party content surgery when present.
    Encryption,
    /// Editor-only content in cooked builds (.pdb, /Engine/Editor/, .uproject…).
    EditorLeftovers,
    /// Per-language .NET satellite assemblies for the launcher.
    LauncherSatellite,
    /// Overall chunking strategy assessment (composite of size + count).
    ChunkingQuality,
    /// Multiple RHI shader caches ship together (PCD3D_SM5 + PCD3D_SM6 etc).
    ShaderRhiRedundancy,
    /// Bundled redistributable installers (UE4PrereqSetup, vc_redist, etc).
    RedistInstaller,
    /// Multi-platform binary tree (Win64 + Linux + Mac in one install).
    PlatformSiblings,
    /// Content-hash duplicates of large loose files.
    DuplicateContent,
    /// Mod manager / installer backup files (.bak, .disabled, etc).
    ModManagerArtifacts,
    /// CEF (Chromium Embedded Framework) per-locale .pak bundles for
    /// languages the user doesn't need.
    CefLocales,
}

impl Category {
    pub fn label(self) -> &'static str {
        match self {
            Category::PatchOverlay => "patch_overlay",
            Category::StaleVersionDir => "stale_version_dir",
            Category::ShardedVideos => "sharded_videos",
            Category::LargeChunk => "large_chunk",
            Category::Encryption => "encryption",
            Category::EditorLeftovers => "editor_leftovers",
            Category::LauncherSatellite => "launcher_satellite",
            Category::ChunkingQuality => "chunking_quality",
            Category::ShaderRhiRedundancy => "shader_rhi_redundancy",
            Category::RedistInstaller => "redist_installer",
            Category::PlatformSiblings => "platform_siblings",
            Category::DuplicateContent => "duplicate_content",
            Category::ModManagerArtifacts => "mod_manager_artifacts",
            Category::CefLocales => "cef_locales",
        }
    }
}

/// One concrete file (or directory) cited as evidence in a finding.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Evidence {
    /// Path relative to the audited root.
    pub path: PathBuf,
    pub size_bytes: u64,
    /// Optional per-item annotation (e.g. "shadows base chunk 70").
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

/// One finding from one detector.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Finding {
    pub detector: String,
    pub category: Category,
    pub severity: Severity,
    pub title: String,
    pub summary: String,
    pub evidence: Vec<Evidence>,
    /// Lower-bound estimate of bytes reclaimable if the finding is addressed.
    /// `None` when we can describe the issue but not put a number on it
    /// (e.g. encryption blocks measurement, chunking quality is structural).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reclaimable_bytes: Option<u64>,
    pub recommendation: String,
}

/// Computed summary of the whole audit.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Aggregate {
    pub total_findings: usize,
    pub findings_by_severity: BTreeMap<Severity, usize>,
    pub reclaimable_by_category: BTreeMap<Category, u64>,
    pub total_reclaimable_bytes: u64,
    /// Reclaimable bytes as a percentage of total install size (0.0-100.0).
    pub total_reclaimable_pct: f64,
    /// Composite 0-100 score. Lower is cleaner.
    pub bloat_score: u8,
}

/// Audit metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditMeta {
    pub schema_version: u32,
    /// shrinkray-audit crate version at the time of the run.
    pub tool_version: String,
    /// RFC3339 timestamp.
    pub generated_at: String,
    /// Detector names that participated in this run.
    pub detectors: Vec<String>,
}

/// Top-level audit report.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditReport {
    pub root: PathBuf,
    pub total_size_bytes: u64,
    pub findings: Vec<Finding>,
    pub aggregate: Aggregate,
    pub meta: AuditMeta,
}

impl AuditReport {
    /// Build an `AuditReport` from the per-detector findings plus the measured
    /// total install size. Computes `Aggregate` (per-category roll-up, severity
    /// counts, bloat score).
    pub fn assemble(
        root: PathBuf,
        total_size_bytes: u64,
        findings: Vec<Finding>,
        detectors: Vec<String>,
    ) -> Self {
        let aggregate = aggregate_findings(total_size_bytes, &findings);
        let meta = AuditMeta {
            schema_version: 1,
            tool_version: env!("CARGO_PKG_VERSION").to_string(),
            generated_at: now_rfc3339(),
            detectors,
        };
        AuditReport {
            root,
            total_size_bytes,
            findings,
            aggregate,
            meta,
        }
    }
}

fn aggregate_findings(total_size_bytes: u64, findings: &[Finding]) -> Aggregate {
    let mut findings_by_severity: BTreeMap<Severity, usize> = BTreeMap::new();
    let mut reclaimable_by_category: BTreeMap<Category, u64> = BTreeMap::new();
    let mut total_reclaimable_bytes: u64 = 0;

    for f in findings {
        *findings_by_severity.entry(f.severity).or_insert(0) += 1;
        if let Some(bytes) = f.reclaimable_bytes {
            *reclaimable_by_category.entry(f.category).or_insert(0) += bytes;
            total_reclaimable_bytes = total_reclaimable_bytes.saturating_add(bytes);
        }
    }

    let total_reclaimable_pct = if total_size_bytes > 0 {
        (total_reclaimable_bytes as f64 / total_size_bytes as f64) * 100.0
    } else {
        0.0
    };

    let bloat_score = compute_bloat_score(total_reclaimable_pct, findings);

    Aggregate {
        total_findings: findings.len(),
        findings_by_severity,
        reclaimable_by_category,
        total_reclaimable_bytes,
        total_reclaimable_pct,
        bloat_score,
    }
}

/// Bloat score, 0-100.
///
/// The number a user sees first. Designed so:
///
/// - 0-20  = clean. Well-shipped game, nothing to do.
/// - 20-50 = mid-tier. Some addressable bloat (typical AAA at 5-15% reclaimable).
/// - 50-80 = visible structural problems (sharded videos, large chunks, stale dirs).
/// - 80-100 = severely bloated install or design failure (WuWa territory).
///
/// Formula: 2× reclaimable % (cap 60), plus +10 per critical finding (cap +30),
/// plus +10 when any non-informational pak-access finding is present
/// (encryption, IoStore, signing, or unknown format — each locks
/// future content-level optimization on the affected paks).
fn compute_bloat_score(reclaimable_pct: f64, findings: &[Finding]) -> u8 {
    let base = (reclaimable_pct * 2.0).min(60.0);

    let critical_count = findings
        .iter()
        .filter(|f| f.severity == Severity::Critical)
        .count();
    let critical_bonus = (critical_count as f64 * 10.0).min(30.0);

    let encryption_bonus = if findings
        .iter()
        .any(|f| f.category == Category::Encryption && f.severity != Severity::Info)
    {
        10.0
    } else {
        0.0
    };

    let raw = base + critical_bonus + encryption_bonus;
    raw.clamp(0.0, 100.0) as u8
}

fn now_rfc3339() -> String {
    // Lightweight: no chrono dep yet. Uses SystemTime + manual format.
    use std::time::{SystemTime, UNIX_EPOCH};
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);

    // Compute UTC Y-M-D h:m:s from epoch seconds. Public-domain algorithm
    // (Howard Hinnant's date library, days_from_civil inverse).
    let (y, mo, d, hh, mm, ss) = epoch_to_ymdhms(secs);
    format!("{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z", y, mo, d, hh, mm, ss)
}

/// Convert UNIX epoch seconds (UTC) to (year, month [1-12], day [1-31],
/// hour, minute, second). Days-from-civil algorithm by Howard Hinnant.
fn epoch_to_ymdhms(secs: u64) -> (i64, u32, u32, u32, u32, u32) {
    let days = (secs / 86400) as i64;
    let time_of_day = (secs % 86400) as u32;
    let hh = time_of_day / 3600;
    let mm = (time_of_day % 3600) / 60;
    let ss = time_of_day % 60;

    let z = days + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = (z - era * 146097) as u64; // [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365; // [0, 399]
    let y = (yoe as i64) + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32; // [1, 31]
    let mo = if mp < 10 { mp + 3 } else { mp - 9 } as u32; // [1, 12]
    let y = if mo <= 2 { y + 1 } else { y };
    (y, mo, d, hh, mm, ss)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dummy_finding(category: Category, severity: Severity, reclaimable: Option<u64>) -> Finding {
        Finding {
            detector: "test".into(),
            category,
            severity,
            title: "t".into(),
            summary: "s".into(),
            evidence: vec![],
            reclaimable_bytes: reclaimable,
            recommendation: "r".into(),
        }
    }

    #[test]
    fn aggregate_empty_is_zero_score() {
        let r = AuditReport::assemble(PathBuf::from("/x"), 1_000_000_000, vec![], vec![]);
        assert_eq!(r.aggregate.bloat_score, 0);
        assert_eq!(r.aggregate.total_reclaimable_bytes, 0);
        assert_eq!(r.aggregate.total_findings, 0);
    }

    #[test]
    fn aggregate_rolls_up_per_category() {
        let findings = vec![
            dummy_finding(Category::PatchOverlay, Severity::Warning, Some(9_000_000_000)),
            dummy_finding(Category::PatchOverlay, Severity::Warning, Some(1_000_000_000)),
            dummy_finding(Category::StaleVersionDir, Severity::Info, Some(500_000_000)),
            dummy_finding(Category::ChunkingQuality, Severity::Critical, None),
        ];
        let r = AuditReport::assemble(
            PathBuf::from("/x"),
            125_000_000_000, // 125 GB
            findings,
            vec!["test".into()],
        );
        assert_eq!(
            r.aggregate.reclaimable_by_category[&Category::PatchOverlay],
            10_000_000_000
        );
        assert_eq!(r.aggregate.total_reclaimable_bytes, 10_500_000_000);
        assert!(r.aggregate.total_reclaimable_pct > 8.0);
        assert!(r.aggregate.total_reclaimable_pct < 9.0);
        assert_eq!(r.aggregate.findings_by_severity[&Severity::Critical], 1);
        assert_eq!(r.aggregate.findings_by_severity[&Severity::Warning], 2);
        assert_eq!(r.aggregate.findings_by_severity[&Severity::Info], 1);
    }

    #[test]
    fn bloat_score_clean_install() {
        // 0.5% reclaimable, no critical findings → near zero score
        let findings = vec![dummy_finding(
            Category::StaleVersionDir,
            Severity::Info,
            Some(500_000_000),
        )];
        let r = AuditReport::assemble(
            PathBuf::from("/x"),
            100_000_000_000,
            findings,
            vec!["test".into()],
        );
        assert!(r.aggregate.bloat_score < 5, "got {}", r.aggregate.bloat_score);
    }

    #[test]
    fn bloat_score_wuwa_class() {
        // ~18% reclaimable + 2 critical findings (large chunk + chunking quality)
        // + warning-severity encryption finding → should be 70+
        let findings = vec![
            dummy_finding(
                Category::PatchOverlay,
                Severity::Warning,
                Some(9_000_000_000),
            ),
            dummy_finding(
                Category::StaleVersionDir,
                Severity::Warning,
                Some(2_000_000_000),
            ),
            dummy_finding(
                Category::ShardedVideos,
                Severity::Warning,
                Some(3_000_000_000),
            ),
            dummy_finding(Category::LargeChunk, Severity::Critical, Some(8_000_000_000)),
            dummy_finding(Category::ChunkingQuality, Severity::Critical, None),
            dummy_finding(Category::Encryption, Severity::Warning, None),
        ];
        let r = AuditReport::assemble(
            PathBuf::from("/x"),
            125_000_000_000,
            findings,
            vec!["test".into()],
        );
        assert!(
            r.aggregate.bloat_score >= 50,
            "expected high bloat score, got {}",
            r.aggregate.bloat_score
        );
    }

    #[test]
    fn json_roundtrip() {
        let r = AuditReport::assemble(
            PathBuf::from("/x"),
            1_000_000_000,
            vec![dummy_finding(
                Category::PatchOverlay,
                Severity::Warning,
                Some(100_000_000),
            )],
            vec!["test".into()],
        );
        let json = serde_json::to_string(&r).expect("serialize");
        let back: AuditReport = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back.total_size_bytes, 1_000_000_000);
        assert_eq!(back.findings.len(), 1);
    }

    #[test]
    fn rfc3339_is_well_formed() {
        let s = now_rfc3339();
        // YYYY-MM-DDTHH:MM:SSZ → 20 chars exactly
        assert_eq!(s.len(), 20);
        assert!(s.ends_with('Z'));
        assert!(s.chars().nth(10) == Some('T'));
    }

    #[test]
    fn epoch_to_ymdhms_known_date() {
        // 2024-01-01 00:00:00 UTC = 1704067200 epoch seconds
        let (y, mo, d, h, m, s) = epoch_to_ymdhms(1_704_067_200);
        assert_eq!((y, mo, d, h, m, s), (2024, 1, 1, 0, 0, 0));
    }
}
