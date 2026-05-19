//! shrinkray-audit — read-only bloat audit.
//!
//! Walks a UE game install and surfaces structural inefficiencies that
//! don't require pak content access:
//!
//! - `_P.pak` patch overlay accumulation (zombie content in base paks)
//! - Stale version directories (`Lang_*/X.Y.Z/`, `Saved/Resources/X.Y.Z/`)
//! - Sharded video pak fragmentation
//! - Monolithic large chunks (poor patch-cost characteristics)
//! - Pak encryption status (locks third-party content surgery)
//! - Editor leftovers in cooked builds (.pdb, /Engine/Editor/, etc.)
//! - Launcher per-language satellite assemblies
//!
//! The audit never writes. Output is an [`AuditReport`] serializable to
//! JSON (via serde) or Markdown (via [`render_markdown`]).
//!
//! Detector modules land in the next commits — this scaffold ships the
//! type + report system that all detectors will produce.

pub mod report;
pub mod types;

pub use report::{format_bytes, render_markdown};
pub use types::{Aggregate, AuditMeta, AuditReport, Category, Evidence, Finding, Severity};
