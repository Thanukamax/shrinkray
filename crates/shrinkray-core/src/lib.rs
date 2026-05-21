//! shrinkray-core — tier-agnostic optimization logic.
//!
//! All modules here are the destructive-write capable subsystems:
//! analyze (read-only census), pak (repak wrapper), backup (differential
//! backup + restore), strip (L10N + pak trimming), recompress (loose-file
//! PNG/WAV/FLAC re-encode), and the audio/texture helpers used by both.
//!
//! Read-only auditing lives in the sibling `shrinkray-audit` crate.

pub mod ai_restore;
pub mod analyze;
pub mod audio;
pub mod backup;
pub mod classifier;
pub mod pak;
pub mod recompress;
pub mod strip;
pub mod texture;
