//! Detectors: each implements [`Detector`] and emits zero or more
//! [`Finding`]s for the audit report.
//!
//! Adding a new detector:
//! 1. Create `detectors/<name>.rs` with a struct implementing `Detector`.
//! 2. Re-export it here.
//! 3. Register it in the `default_detectors()` list in `lib.rs`.

pub mod patch_overlay;
pub mod stale_versions;

use crate::types::Finding;
use std::path::Path;

/// Read-only detector. Every detector takes the install root and returns
/// findings; it must not mutate disk in any way.
///
/// Detectors should be cheap: walk the tree once, classify, return.
/// Anything expensive (full pak entry enumeration, hashing) belongs in a
/// separate explicit op, not the audit pipeline.
pub trait Detector: Send + Sync {
    /// Stable identifier — appears in `Finding.detector` and the audit
    /// metadata's detector list. Use `snake_case`.
    fn name(&self) -> &'static str;

    fn run(&self, root: &Path) -> anyhow::Result<Vec<Finding>>;
}
