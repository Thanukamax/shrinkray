//! G1 — BC-byte-residual variant of Δ-Codec.
//!
//! The original Δ-Codec computes its residual in RGBA pixel space. That's
//! conceptually clean but the cooked game pak ultimately stores BC-encoded
//! bytes, and BC's lossy quantization introduces noise that the pixel-space
//! residual has to carry. The block-space variant computes the residual
//! *after* both the original and prediction are BC-encoded.
//!
//! Hypothesis: BC quantization is the same operator applied to both sides
//! → it absorbs most of the prediction error into the block search →
//! residual on the BC byte stream is sparser and lower-entropy than the
//! pixel-space residual.
//!
//! The stronger property: with a deterministic BC encoder (verified by
//! G4 tests in tests/g4_bc_determinism.rs), the decoder reproduces BC
//! bytes *byte-for-byte*. That's the file-byte anti-cheat claim, not just
//! RGBA-byte.
//!
//! This module ships the prototype encoder/decoder so the bench can
//! compare apples-to-apples against the pixel-space path.

use std::io::Read;

use anyhow::{anyhow, bail, Context, Result};
use image_dds::{ImageFormat, Mipmaps, Quality, Surface, SurfaceRgba8};
use serde::{Deserialize, Serialize};

use crate::{PredictorId, Predictor};

/// BC format identifier — mirrors `shrinkray_core::bcn::BcFormat` but we
/// keep it private so the codec crate stays decoupled from shrinkray-core.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum BcResidualFormat {
    Bc1,
    Bc3,
    Bc5,
    Bc7,
}

impl BcResidualFormat {
    fn to_image_dds(self) -> ImageFormat {
        match self {
            BcResidualFormat::Bc1 => ImageFormat::BC1RgbaUnorm,
            BcResidualFormat::Bc3 => ImageFormat::BC3RgbaUnorm,
            BcResidualFormat::Bc5 => ImageFormat::BC5RgUnorm,
            BcResidualFormat::Bc7 => ImageFormat::BC7RgbaUnorm,
        }
    }
}

/// Block-space Δ-Codec bitstream. Mirrors [`crate::DeltaBitstream`] but the
/// residual is over BC-encoded bytes, not RGBA pixels.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BcDeltaBitstream {
    pub magic: u32,
    pub version: u16,
    pub predictor: PredictorId,
    pub bc_format: BcResidualFormat,
    pub low_w: u32,
    pub low_h: u32,
    pub top_w: u32,
    pub top_h: u32,
    pub low_mip_rgba: Vec<u8>,
    /// Signed per-byte delta between `BC(original)` and `BC(predicted)`,
    /// zstd-compressed. With BC determinism (G4), the decoder recomputes
    /// `BC(predicted)` exactly and adds this delta to recover `BC(original)`
    /// byte-for-byte.
    pub bc_residual_zst: Vec<u8>,
    pub original_bc_sha256: Option<[u8; 32]>,
}

pub const BC_DELTA_MAGIC: u32 = 0x42434443; // "BCDC"
pub const BC_DELTA_VERSION: u16 = 1;

/// Encode the BC-byte-residual variant.
///
/// `top_mip_rgba` is the original at top dims; the encoder BC-encodes it,
/// runs the predictor on `low_mip_rgba`, BC-encodes the prediction, computes
/// the signed-byte delta, and zstds it. Decoder recovers `BC(original)`
/// byte-exact when the predictor + BC encoder are both deterministic.
pub fn encode_bc_residual<P: Predictor>(
    predictor: &mut P,
    bc_format: BcResidualFormat,
    top_mip_rgba: &[u8],
    top_w: u32,
    top_h: u32,
    low_mip_rgba: Vec<u8>,
    low_w: u32,
    low_h: u32,
    record_hash: bool,
) -> Result<BcDeltaBitstream> {
    let expected_top = (top_w as usize) * (top_h as usize) * 4;
    if top_mip_rgba.len() != expected_top {
        bail!(
            "encode_bc_residual: top mip rgba len {} != w*h*4 = {}",
            top_mip_rgba.len(),
            expected_top
        );
    }
    let expected_low = (low_w as usize) * (low_h as usize) * 4;
    if low_mip_rgba.len() != expected_low {
        bail!(
            "encode_bc_residual: low mip rgba len {} != w*h*4 = {}",
            low_mip_rgba.len(),
            expected_low
        );
    }

    let original_bc = bc_encode(bc_format, top_mip_rgba, top_w, top_h)?;
    let predicted = predictor.predict(&low_mip_rgba, low_w, low_h, top_w, top_h)?;
    if predicted.len() != expected_top {
        bail!(
            "encode_bc_residual: predictor wrong byte count: got {}, expected {}",
            predicted.len(),
            expected_top
        );
    }
    let predicted_bc = bc_encode(bc_format, &predicted, top_w, top_h)?;
    if original_bc.len() != predicted_bc.len() {
        bail!(
            "encode_bc_residual: BC byte counts disagree: {} vs {}",
            original_bc.len(),
            predicted_bc.len()
        );
    }

    // Signed delta per BC byte. Each value lives in `[-255, 255]`.
    let mut residual = Vec::with_capacity(original_bc.len() * 2);
    for i in 0..original_bc.len() {
        let d = (original_bc[i] as i16) - (predicted_bc[i] as i16);
        residual.extend_from_slice(&d.to_le_bytes());
    }
    let bc_residual_zst =
        zstd::encode_all(residual.as_slice(), 19).context("zstd encode bc residual")?;

    let original_bc_sha256 = if record_hash {
        Some(super::sha256_of_via_sha2(&original_bc))
    } else {
        None
    };

    Ok(BcDeltaBitstream {
        magic: BC_DELTA_MAGIC,
        version: BC_DELTA_VERSION,
        predictor: predictor.id(),
        bc_format,
        low_w,
        low_h,
        top_w,
        top_h,
        low_mip_rgba,
        bc_residual_zst,
        original_bc_sha256,
    })
}

/// Decode a BC-byte-residual bitstream back to the original BC bytes.
///
/// With deterministic predictor + deterministic BC encoder, the result is
/// byte-for-byte identical to whatever the original BC bytes were on disk.
pub fn decode_bc_residual<P: Predictor>(
    predictor: &mut P,
    bitstream: &BcDeltaBitstream,
) -> Result<Vec<u8>> {
    if bitstream.magic != BC_DELTA_MAGIC {
        bail!(
            "bc delta magic mismatch: got 0x{:08x}, expected 0x{:08x}",
            bitstream.magic,
            BC_DELTA_MAGIC
        );
    }
    if bitstream.version != BC_DELTA_VERSION {
        bail!(
            "bc delta version mismatch: got {}, expected {}",
            bitstream.version,
            BC_DELTA_VERSION
        );
    }
    if predictor.id() != bitstream.predictor {
        bail!(
            "predictor id mismatch: bitstream wants {:?}, caller supplied {:?}",
            bitstream.predictor,
            predictor.id()
        );
    }

    let predicted = predictor.predict(
        &bitstream.low_mip_rgba,
        bitstream.low_w,
        bitstream.low_h,
        bitstream.top_w,
        bitstream.top_h,
    )?;
    let predicted_bc = bc_encode(bitstream.bc_format, &predicted, bitstream.top_w, bitstream.top_h)?;

    let mut residual_buf = Vec::with_capacity(predicted_bc.len() * 2);
    zstd::Decoder::new(bitstream.bc_residual_zst.as_slice())
        .context("init zstd decoder")?
        .read_to_end(&mut residual_buf)
        .context("zstd decode bc residual")?;
    if residual_buf.len() != predicted_bc.len() * 2 {
        bail!(
            "bc residual size {} != expected {} (2 bytes per BC byte)",
            residual_buf.len(),
            predicted_bc.len() * 2
        );
    }

    let mut out = Vec::with_capacity(predicted_bc.len());
    for i in 0..predicted_bc.len() {
        let d = i16::from_le_bytes([residual_buf[i * 2], residual_buf[i * 2 + 1]]);
        let v = (predicted_bc[i] as i16) + d;
        out.push(v.clamp(0, 255) as u8);
    }

    if let Some(expected) = &bitstream.original_bc_sha256 {
        let got = super::sha256_of_via_sha2(&out);
        if &got != expected {
            return Err(anyhow!(
                "BC-Δ-Codec reconstruction failed BC-byte hash check"
            ));
        }
    }
    Ok(out)
}

/// Cost breakdown for a BC-byte-residual bitstream.
#[derive(Debug, Clone, Serialize)]
pub struct BcDeltaSize {
    pub low_mip_bytes: usize,
    pub residual_zst_bytes: usize,
    pub overhead_bytes: usize,
    pub total_bytes: usize,
}

impl BcDeltaBitstream {
    pub fn size(&self) -> BcDeltaSize {
        let low = self.low_mip_rgba.len();
        let residual = self.bc_residual_zst.len();
        let pred_bytes = match &self.predictor {
            PredictorId::Bilinear => 1,
            PredictorId::RealEsrganX4 => 1,
            PredictorId::Onnx4x { sha256 } => sha256.len() + 8,
        };
        let hash_bytes = if self.original_bc_sha256.is_some() {
            32
        } else {
            0
        };
        // 4 (magic) + 2 (version) + 1 (bc fmt tag) + 4*4 (dims) = 23, plus
        // predictor id + hash bytes.
        let overhead = 23 + pred_bytes + hash_bytes;
        BcDeltaSize {
            low_mip_bytes: low,
            residual_zst_bytes: residual,
            overhead_bytes: overhead,
            total_bytes: low + residual + overhead,
        }
    }
}

fn bc_encode(format: BcResidualFormat, rgba: &[u8], w: u32, h: u32) -> Result<Vec<u8>> {
    let surface = SurfaceRgba8 {
        width: w,
        height: h,
        depth: 1,
        layers: 1,
        mipmaps: 1,
        data: rgba,
    };
    Ok(surface
        .encode(format.to_image_dds(), Quality::Fast, Mipmaps::Disabled)
        .map_err(|e| anyhow!("image_dds encode failed: {e}"))?
        .data)
}

/// Decode a BC byte stream back to RGBA. Used by the bench harness to
/// compute the per-channel error of a BC-byte-residual reconstruction
/// vs. the original RGBA (so we can compare apples-to-apples with the
/// pixel-space Δ-Codec's max_err figure).
pub fn bc_decode_to_rgba8(
    format: BcResidualFormat,
    width: u32,
    height: u32,
    bc_bytes: &[u8],
) -> Result<Vec<u8>> {
    let surface = Surface {
        width,
        height,
        depth: 1,
        layers: 1,
        mipmaps: 1,
        image_format: format.to_image_dds(),
        data: bc_bytes,
    };
    Ok(surface
        .decode_rgba8()
        .map_err(|e| anyhow!("image_dds decode failed: {e}"))?
        .data)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{box_downsample_2x, BilinearPredictor};

    fn rgba_gradient(w: u32, h: u32) -> Vec<u8> {
        let mut out = Vec::with_capacity((w * h * 4) as usize);
        for y in 0..h {
            for x in 0..w {
                out.push(((x * 257 / w) & 0xff) as u8);
                out.push(((y * 257 / h) & 0xff) as u8);
                out.push((((x + y) * 257 / (w + h)) & 0xff) as u8);
                out.push(255);
            }
        }
        out
    }

    #[test]
    fn bc1_round_trip_byte_exact() {
        let (w, h) = (64u32, 64u32);
        let rgba = rgba_gradient(w, h);
        let (low, lw, lh) = box_downsample_2x(&rgba, w, h).unwrap();
        let mut pred = BilinearPredictor;
        let bs = encode_bc_residual(
            &mut pred,
            BcResidualFormat::Bc1,
            &rgba,
            w,
            h,
            low,
            lw,
            lh,
            true,
        )
        .unwrap();
        let mut dec = BilinearPredictor;
        let bc_out = decode_bc_residual(&mut dec, &bs).unwrap();

        let expected = bc_encode(BcResidualFormat::Bc1, &rgba, w, h).unwrap();
        assert_eq!(bc_out, expected, "BC1 byte-exact recovery");
    }

    #[test]
    fn bc3_round_trip_byte_exact() {
        let (w, h) = (32u32, 32u32);
        let rgba = rgba_gradient(w, h);
        let (low, lw, lh) = box_downsample_2x(&rgba, w, h).unwrap();
        let mut pred = BilinearPredictor;
        let bs = encode_bc_residual(
            &mut pred,
            BcResidualFormat::Bc3,
            &rgba,
            w,
            h,
            low,
            lw,
            lh,
            true,
        )
        .unwrap();
        let mut dec = BilinearPredictor;
        let bc_out = decode_bc_residual(&mut dec, &bs).unwrap();

        let expected = bc_encode(BcResidualFormat::Bc3, &rgba, w, h).unwrap();
        assert_eq!(bc_out, expected, "BC3 byte-exact recovery");
    }

    #[test]
    fn bc7_round_trip_byte_exact() {
        let (w, h) = (32u32, 32u32);
        let rgba = rgba_gradient(w, h);
        let (low, lw, lh) = box_downsample_2x(&rgba, w, h).unwrap();
        let mut pred = BilinearPredictor;
        let bs = encode_bc_residual(
            &mut pred,
            BcResidualFormat::Bc7,
            &rgba,
            w,
            h,
            low,
            lw,
            lh,
            true,
        )
        .unwrap();
        let mut dec = BilinearPredictor;
        let bc_out = decode_bc_residual(&mut dec, &bs).unwrap();
        let expected = bc_encode(BcResidualFormat::Bc7, &rgba, w, h).unwrap();
        assert_eq!(bc_out, expected, "BC7 byte-exact recovery");
    }
}
