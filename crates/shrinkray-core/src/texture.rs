//! Texture detect + recompression (Phase 1).
//! Pipeline: decode (image / dds) → analyze (current format + dims) →
//! re-encode (intel_tex_2 for BC1/3/7) → write back.
//! Mipmap strip optional based on target tier.

use anyhow::Result;
use std::path::Path;

#[allow(dead_code)]
pub enum Quality {
    Lossless,
    High,
    Balanced,
    Aggressive,
}

#[allow(dead_code)]
pub fn recompress(_src: &Path, _dst: &Path, _quality: Quality) -> Result<u64> {
    anyhow::bail!("texture recompression lands in Phase 1")
}
