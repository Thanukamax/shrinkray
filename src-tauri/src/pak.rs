//! UE pak file operations (Phase 1).
//! Will wrap the `repak` crate for read/write + chosen compression.
//! V0: stubs that return an explicit "not yet implemented" error so the
//! frontend can surface intent without crashing.

use anyhow::Result;
use std::path::Path;

#[allow(dead_code)]
pub fn unpack(_pak: &Path, _dest: &Path) -> Result<()> {
    anyhow::bail!("pak unpacker lands in Phase 1")
}

#[allow(dead_code)]
pub fn repack(_src: &Path, _pak: &Path) -> Result<()> {
    anyhow::bail!("pak repacker lands in Phase 1")
}
