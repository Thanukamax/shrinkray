//! Audio detect + re-encode (Phase 1).
//! Pipeline: decode (symphonia: WAV / OGG-Vorbis / MP3 / FLAC) → resample
//! if needed → encode to Opus at chosen bitrate → write back.

use anyhow::Result;
use std::path::Path;

#[allow(dead_code)]
pub struct Bitrate(pub u32);

#[allow(dead_code)]
pub fn reencode(_src: &Path, _dst: &Path, _target: Bitrate) -> Result<u64> {
    anyhow::bail!("audio re-encode lands in Phase 1")
}
