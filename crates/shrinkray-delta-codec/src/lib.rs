//! Δ-Codec — combined AI-fast / byte-exact texture compression.
//!
//! The thesis: industry tools force a pick between
//!   (a) ship a small lossy version (Real-ESRGAN family, FitGirl repacks) and
//!   (b) ship a full backup for byte-exact restore (most pak rewriters).
//! Δ-Codec is a single bitstream that satisfies both. Encode keeps a low mip
//! plus a *residual* — the per-pixel delta between the original top mip and
//! whatever the predictor would have produced from the low mip alone. Decode
//! runs the predictor and adds the residual back. With `quant_step == 1` the
//! reconstruction is byte-exact in RGBA space; with `quant_step > 1` we trade
//! quality for residual size.
//!
//! The novelty is the *combination*: anti-cheat-safe restore (hash matches)
//! AND smaller-than-full-backup distribution. Industry literature treats
//! these as mutually exclusive; this crate is the falsifiable experiment.
//!
//! Crate is intentionally predictor-agnostic. The caller passes a closure
//! that produces the high-mip prediction from the low mip. Tests use
//! bilinear upsample as a transparent baseline; production wires a 4×
//! Real-ESRGAN ONNX inference via `shrinkray-core::inference`.

use std::io::Read;

use anyhow::{anyhow, bail, Context, Result};
use serde::{Deserialize, Serialize};

pub mod bc_residual;
pub mod probe;
pub use bc_residual::{
    decode_bc_residual, encode_bc_residual, BcDeltaBitstream, BcResidualFormat,
};
pub use probe::{probe_codec_space, CodecSpace, CodecSpaceRecommendation, HIGH_PASS_THRESHOLD};

/// Bitstream identifier — bumped whenever the on-disk layout changes
/// incompatibly. Decoders refuse to read a bitstream with a different magic.
pub const DELTA_CODEC_MAGIC: u32 = 0x44434443; // "DCDC"
pub const DELTA_CODEC_VERSION: u16 = 1;

/// Compact predictor identifier. Decoder must wire the matching predictor
/// closure or the residual is meaningless.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum PredictorId {
    /// Plain bilinear upsample. The trivial baseline — what mip generation
    /// does already. Residual stays large because no learning was applied.
    Bilinear,
    /// 4× Real-ESRGAN-class upscaler. Residual should be smaller for stylised
    /// natural-image content.
    RealEsrganX4,
    /// Generic 4× ONNX upscaler identified by model checksum. Lets us swap
    /// predictors without bumping the magic.
    Onnx4x { sha256: String },
}

/// Result of encoding one top mip. `low_mip_rgba` is the small mip kept on
/// disk after strip; `residual_zst` is the compressed signed-int16 delta
/// between the original top mip and the predictor's reconstruction of it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeltaBitstream {
    pub magic: u32,
    pub version: u16,
    pub predictor: PredictorId,
    pub quant_step: u8,
    pub low_w: u32,
    pub low_h: u32,
    pub top_w: u32,
    pub top_h: u32,
    pub low_mip_rgba: Vec<u8>,
    pub residual_zst: Vec<u8>,
    /// Optional SHA-256 over the *original* top mip (RGBA bytes pre-encode).
    /// Decoder checks this after reconstruction; gives the anti-cheat-safe
    /// hash receipt the consumer story depends on.
    #[serde(default)]
    pub original_sha256: Option<[u8; 32]>,
}

/// Closure signature for any predictor. Given a low mip in RGBA, produce a
/// high mip in RGBA at the requested target dimensions.
///
/// The crate is intentionally synchronous and CPU-bound to keep the surface
/// area small. Callers that want GPU/async run their predictor outside and
/// pass `encode_with_prediction` directly.
pub trait Predictor {
    fn predict(&mut self, low_rgba: &[u8], low_w: u32, low_h: u32, top_w: u32, top_h: u32)
        -> Result<Vec<u8>>;
    fn id(&self) -> PredictorId;
}

/// Bilinear-upsample baseline predictor. Used by tests and as a transparent
/// fallback when no neural model is available.
pub struct BilinearPredictor;

impl Predictor for BilinearPredictor {
    fn id(&self) -> PredictorId {
        PredictorId::Bilinear
    }
    fn predict(
        &mut self,
        low_rgba: &[u8],
        low_w: u32,
        low_h: u32,
        top_w: u32,
        top_h: u32,
    ) -> Result<Vec<u8>> {
        bilinear_upsample(low_rgba, low_w, low_h, top_w, top_h)
    }
}

/// Quantization parameter. `step=1` is fully lossless; larger steps drop
/// residual precision (and therefore residual entropy / disk size).
///
/// We store the *residual* as signed int16 per channel before quantization.
/// quantization is simple uniform: `q = r / step` rounded to nearest.
pub fn encode_texture<P: Predictor>(
    predictor: &mut P,
    top_mip_rgba: &[u8],
    top_w: u32,
    top_h: u32,
    low_mip_rgba: Vec<u8>,
    low_w: u32,
    low_h: u32,
    quant_step: u8,
    record_hash: bool,
) -> Result<DeltaBitstream> {
    if quant_step == 0 {
        bail!("quant_step must be ≥ 1 (1 = lossless)");
    }
    let expected_top = (top_w as usize)
        .checked_mul(top_h as usize)
        .and_then(|v| v.checked_mul(4))
        .ok_or_else(|| anyhow!("top mip dims overflow"))?;
    if top_mip_rgba.len() != expected_top {
        bail!(
            "top mip rgba length {} != w*h*4 = {} (w={top_w}, h={top_h})",
            top_mip_rgba.len(),
            expected_top
        );
    }
    let expected_low = (low_w as usize)
        .checked_mul(low_h as usize)
        .and_then(|v| v.checked_mul(4))
        .ok_or_else(|| anyhow!("low mip dims overflow"))?;
    if low_mip_rgba.len() != expected_low {
        bail!(
            "low mip rgba length {} != w*h*4 = {} (w={low_w}, h={low_h})",
            low_mip_rgba.len(),
            expected_low
        );
    }

    let predicted = predictor
        .predict(&low_mip_rgba, low_w, low_h, top_w, top_h)
        .context("predictor produced no output")?;
    if predicted.len() != expected_top {
        bail!(
            "predictor returned wrong byte count: got {}, expected {}",
            predicted.len(),
            expected_top
        );
    }

    let mut residual = Vec::with_capacity(expected_top * 2);
    for i in 0..expected_top {
        let r = (top_mip_rgba[i] as i16) - (predicted[i] as i16);
        let q = quantize(r, quant_step);
        residual.extend_from_slice(&q.to_le_bytes());
    }

    let residual_zst = zstd::encode_all(residual.as_slice(), 19)
        .context("zstd encode residual")?;

    let original_sha256 = if record_hash {
        Some(sha256_of(top_mip_rgba))
    } else {
        None
    };

    Ok(DeltaBitstream {
        magic: DELTA_CODEC_MAGIC,
        version: DELTA_CODEC_VERSION,
        predictor: predictor.id(),
        quant_step,
        low_w,
        low_h,
        top_w,
        top_h,
        low_mip_rgba,
        residual_zst,
        original_sha256,
    })
}

/// Decode a Δ-Codec bitstream back to a top-mip RGBA buffer.
///
/// The caller must supply a predictor matching the bitstream's `predictor`
/// id — otherwise the residual lands on the wrong baseline and the
/// reconstruction is garbage.
pub fn decode_texture<P: Predictor>(
    predictor: &mut P,
    bitstream: &DeltaBitstream,
) -> Result<Vec<u8>> {
    if bitstream.magic != DELTA_CODEC_MAGIC {
        bail!(
            "bitstream magic mismatch: got 0x{:08x}, expected 0x{:08x}",
            bitstream.magic,
            DELTA_CODEC_MAGIC
        );
    }
    if bitstream.version != DELTA_CODEC_VERSION {
        bail!(
            "bitstream version mismatch: got {}, expected {}",
            bitstream.version,
            DELTA_CODEC_VERSION
        );
    }
    if predictor.id() != bitstream.predictor {
        bail!(
            "predictor id mismatch: bitstream wants {:?}, caller supplied {:?}",
            bitstream.predictor,
            predictor.id()
        );
    }
    if bitstream.quant_step == 0 {
        bail!("invalid bitstream: quant_step == 0");
    }

    let predicted = predictor
        .predict(
            &bitstream.low_mip_rgba,
            bitstream.low_w,
            bitstream.low_h,
            bitstream.top_w,
            bitstream.top_h,
        )
        .context("predictor failed during decode")?;
    let expected_top = (bitstream.top_w as usize)
        .checked_mul(bitstream.top_h as usize)
        .and_then(|v| v.checked_mul(4))
        .ok_or_else(|| anyhow!("top mip dims overflow"))?;
    if predicted.len() != expected_top {
        bail!(
            "predictor returned wrong byte count during decode: got {}, expected {}",
            predicted.len(),
            expected_top
        );
    }

    let mut residual_buf = Vec::with_capacity(expected_top * 2);
    zstd::Decoder::new(bitstream.residual_zst.as_slice())
        .context("init zstd decoder")?
        .read_to_end(&mut residual_buf)
        .context("zstd decode residual")?;
    if residual_buf.len() != expected_top * 2 {
        bail!(
            "decoded residual size {} != expected {} (2 bytes per pixel-channel)",
            residual_buf.len(),
            expected_top * 2
        );
    }

    let mut out = Vec::with_capacity(expected_top);
    for i in 0..expected_top {
        let q = i16::from_le_bytes([residual_buf[i * 2], residual_buf[i * 2 + 1]]);
        let r = dequantize(q, bitstream.quant_step);
        let reconstructed = (predicted[i] as i16) + r;
        out.push(reconstructed.clamp(0, 255) as u8);
    }

    if let Some(expected_sha) = &bitstream.original_sha256 {
        let got = sha256_of(&out);
        if &got != expected_sha {
            bail!(
                "Δ-Codec reconstruction failed hash check: got {}, expected {}",
                hex(&got),
                hex(expected_sha)
            );
        }
    }
    Ok(out)
}

fn quantize(r: i16, step: u8) -> i16 {
    if step == 1 {
        r
    } else {
        let s = step as i32;
        let half = (s / 2) as i32;
        let q = if r >= 0 {
            (r as i32 + half) / s
        } else {
            (r as i32 - half) / s
        };
        q as i16
    }
}

fn dequantize(q: i16, step: u8) -> i16 {
    if step == 1 {
        q
    } else {
        (q as i32 * step as i32) as i16
    }
}

/// CPU bilinear upsample for RGBA8 buffers. Used by the bilinear baseline
/// predictor *and* serves as a fallback when no neural model is available.
pub fn bilinear_upsample(
    src: &[u8],
    src_w: u32,
    src_h: u32,
    dst_w: u32,
    dst_h: u32,
) -> Result<Vec<u8>> {
    let expected_src = (src_w as usize)
        .checked_mul(src_h as usize)
        .and_then(|v| v.checked_mul(4))
        .ok_or_else(|| anyhow!("src dims overflow"))?;
    if src.len() != expected_src {
        bail!(
            "bilinear_upsample: src rgba length {} != w*h*4 = {} (w={src_w}, h={src_h})",
            src.len(),
            expected_src
        );
    }
    if src_w == 0 || src_h == 0 || dst_w == 0 || dst_h == 0 {
        bail!("bilinear_upsample: zero dimension");
    }

    let mut out = vec![0u8; (dst_w as usize) * (dst_h as usize) * 4];
    let scale_x = (src_w as f32 - 1.0) / (dst_w as f32 - 1.0).max(1.0);
    let scale_y = (src_h as f32 - 1.0) / (dst_h as f32 - 1.0).max(1.0);
    for dy in 0..dst_h {
        let fy = dy as f32 * scale_y;
        let y0 = fy.floor() as u32;
        let y1 = (y0 + 1).min(src_h - 1);
        let wy = fy - y0 as f32;
        for dx in 0..dst_w {
            let fx = dx as f32 * scale_x;
            let x0 = fx.floor() as u32;
            let x1 = (x0 + 1).min(src_w - 1);
            let wx = fx - x0 as f32;
            for c in 0..4 {
                let p00 = src[((y0 * src_w + x0) * 4 + c) as usize] as f32;
                let p10 = src[((y0 * src_w + x1) * 4 + c) as usize] as f32;
                let p01 = src[((y1 * src_w + x0) * 4 + c) as usize] as f32;
                let p11 = src[((y1 * src_w + x1) * 4 + c) as usize] as f32;
                let top = p00 * (1.0 - wx) + p10 * wx;
                let bot = p01 * (1.0 - wx) + p11 * wx;
                let v = top * (1.0 - wy) + bot * wy;
                out[((dy * dst_w + dx) * 4 + c as u32) as usize] =
                    v.round().clamp(0.0, 255.0) as u8;
            }
        }
    }
    Ok(out)
}

/// Box-average downsample by 2 (RGBA8). Used by the experiment harness to
/// derive a low mip from a top mip when the cooker's intermediate mip isn't
/// available (e.g. when measuring against a freshly decoded BCn buffer).
pub fn box_downsample_2x(src: &[u8], src_w: u32, src_h: u32) -> Result<(Vec<u8>, u32, u32)> {
    if src_w < 2 || src_h < 2 {
        bail!("box_downsample_2x: source too small ({src_w}x{src_h})");
    }
    let dst_w = src_w / 2;
    let dst_h = src_h / 2;
    let expected_src = (src_w as usize) * (src_h as usize) * 4;
    if src.len() != expected_src {
        bail!(
            "box_downsample_2x: src len {} != {} (w={src_w}, h={src_h})",
            src.len(),
            expected_src
        );
    }
    let mut out = vec![0u8; (dst_w as usize) * (dst_h as usize) * 4];
    for dy in 0..dst_h {
        for dx in 0..dst_w {
            for c in 0..4 {
                let s00 = src[(((dy * 2) * src_w + dx * 2) * 4 + c) as usize] as u32;
                let s10 = src[(((dy * 2) * src_w + dx * 2 + 1) * 4 + c) as usize] as u32;
                let s01 = src[(((dy * 2 + 1) * src_w + dx * 2) * 4 + c) as usize] as u32;
                let s11 = src[(((dy * 2 + 1) * src_w + dx * 2 + 1) * 4 + c) as usize] as u32;
                let avg = (s00 + s10 + s01 + s11 + 2) / 4;
                out[((dy * dst_w + dx) * 4 + c as u32) as usize] = avg as u8;
            }
        }
    }
    Ok((out, dst_w, dst_h))
}

/// Bitstream measurement struct used by the experiment harness. Lets us
/// tabulate "what did Δ-Codec actually cost" without re-parsing the
/// bitstream layout in callers.
#[derive(Debug, Clone, Serialize)]
pub struct BitstreamSize {
    pub low_mip_bytes: usize,
    pub residual_zst_bytes: usize,
    pub overhead_bytes: usize,
    pub total_bytes: usize,
}

/// G12 — entropy-floor measurement of a Δ-Codec bitstream.
///
/// The residual stream is a sequence of i16-LE values, one per pixel-channel.
/// `entropy_bits_per_byte` is the Shannon entropy of that raw stream (before
/// zstd). `zstd_efficiency` is the ratio `entropy_bits_total / zstd_bits` —
/// 1.0 means zstd hit the information-theoretic floor for this stream;
/// lower means there is residual redundancy zstd missed (and a better
/// entropy coder could exploit).
#[derive(Debug, Clone, Serialize)]
pub struct EntropyReport {
    pub residual_uncompressed_bytes: usize,
    pub residual_zst_bytes: usize,
    pub entropy_bits_per_byte: f64,
    pub entropy_bits_total: f64,
    pub zstd_bits_total: f64,
    /// 1.0 = zstd hit the Shannon floor for this stream. Values < 1.0 mean
    /// residual redundancy is still on the table.
    pub zstd_efficiency: f64,
}

/// Shannon entropy of a byte sequence in bits/byte. 0 = all bytes equal, 8 =
/// uniform distribution.
pub fn shannon_entropy_bits_per_byte(bytes: &[u8]) -> f64 {
    if bytes.is_empty() {
        return 0.0;
    }
    let mut counts = [0u64; 256];
    for &b in bytes {
        counts[b as usize] += 1;
    }
    let n = bytes.len() as f64;
    let mut h = 0.0f64;
    for &c in &counts {
        if c == 0 {
            continue;
        }
        let p = (c as f64) / n;
        h -= p * p.log2();
    }
    h
}

impl DeltaBitstream {
    /// Sum the on-disk cost components. Overhead is everything beyond the
    /// two payload bufs (struct fields, predictor id, quant step, dims, hash).
    pub fn size(&self) -> BitstreamSize {
        let low = self.low_mip_rgba.len();
        let residual = self.residual_zst.len();
        let overhead = {
            // 4 (magic) + 2 (version) + 1 (quant) + 4 dim fields * 4 = 23
            // + predictor id varies; estimate worst-case as 96 bytes (sha256 string + tag)
            // + optional sha256 = 32 if present
            let pred_bytes = match &self.predictor {
                PredictorId::Bilinear => 1,
                PredictorId::RealEsrganX4 => 1,
                PredictorId::Onnx4x { sha256 } => sha256.len() + 8,
            };
            let hash_bytes = if self.original_sha256.is_some() { 32 } else { 0 };
            23 + pred_bytes + hash_bytes
        };
        BitstreamSize {
            low_mip_bytes: low,
            residual_zst_bytes: residual,
            overhead_bytes: overhead,
            total_bytes: low + residual + overhead,
        }
    }
}

impl DeltaBitstream {
    /// G12 — decompress the residual to recover the raw int16 stream, then
    /// measure its Shannon entropy + zstd efficiency. This is the
    /// "how close to the information-theoretic floor is our entropy coder?"
    /// figure for the paper.
    pub fn entropy_report(&self) -> Result<EntropyReport> {
        let mut residual = Vec::new();
        zstd::Decoder::new(self.residual_zst.as_slice())
            .context("init zstd decoder")?
            .read_to_end(&mut residual)
            .context("zstd decode residual")?;
        let entropy = shannon_entropy_bits_per_byte(&residual);
        let entropy_total = entropy * residual.len() as f64;
        let zstd_bits = self.residual_zst.len() as f64 * 8.0;
        let efficiency = if zstd_bits > 0.0 {
            entropy_total / zstd_bits
        } else {
            0.0
        };
        Ok(EntropyReport {
            residual_uncompressed_bytes: residual.len(),
            residual_zst_bytes: self.residual_zst.len(),
            entropy_bits_per_byte: entropy,
            entropy_bits_total: entropy_total,
            zstd_bits_total: zstd_bits,
            zstd_efficiency: efficiency,
        })
    }
}

fn sha256_of(bytes: &[u8]) -> [u8; 32] {
    sha256_of_via_sha2(bytes)
}

pub(crate) fn sha256_of_via_sha2(bytes: &[u8]) -> [u8; 32] {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hasher.finalize().into()
}

fn hex(b: &[u8; 32]) -> String {
    let mut s = String::with_capacity(64);
    for byte in b {
        s.push_str(&format!("{:02x}", byte));
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

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

    fn rgba_solid(w: u32, h: u32, color: [u8; 4]) -> Vec<u8> {
        let mut out = Vec::with_capacity((w * h * 4) as usize);
        for _ in 0..(w * h) {
            out.extend_from_slice(&color);
        }
        out
    }

    #[test]
    fn round_trip_byte_exact_on_solid_color() {
        // Solid color: predictor (bilinear) reconstructs exactly → residual
        // is all zero → zstd compresses to a few bytes. Decode must
        // reproduce the original RGBA byte-for-byte.
        let (w, h) = (64u32, 64u32);
        let top = rgba_solid(w, h, [128, 64, 32, 255]);
        let (low, lw, lh) = box_downsample_2x(&top, w, h).unwrap();
        let mut pred = BilinearPredictor;
        let bitstream = encode_texture(&mut pred, &top, w, h, low, lw, lh, 1, true).unwrap();
        let mut dec = BilinearPredictor;
        let restored = decode_texture(&mut dec, &bitstream).unwrap();
        assert_eq!(restored, top);
    }

    #[test]
    fn round_trip_byte_exact_on_gradient() {
        let (w, h) = (32u32, 32u32);
        let top = rgba_gradient(w, h);
        let (low, lw, lh) = box_downsample_2x(&top, w, h).unwrap();
        let mut pred = BilinearPredictor;
        let bitstream = encode_texture(&mut pred, &top, w, h, low, lw, lh, 1, true).unwrap();
        let mut dec = BilinearPredictor;
        let restored = decode_texture(&mut dec, &bitstream).unwrap();
        assert_eq!(restored, top, "lossless round-trip must reproduce original");
    }

    #[test]
    fn residual_compresses_better_for_predictable_content() {
        // The whole pitch: a "predictable" texture (smooth gradient) yields
        // a small residual; an unpredictable one (pure noise) yields a big
        // one. If this property holds we have a real codec.
        let (w, h) = (64u32, 64u32);
        let smooth = rgba_gradient(w, h);
        let (low_s, lw, lh) = box_downsample_2x(&smooth, w, h).unwrap();

        let mut noise = Vec::with_capacity((w * h * 4) as usize);
        let mut rng: u64 = 0x1234_5678_9abc_def0;
        for _ in 0..(w * h * 4) {
            // xorshift64
            rng ^= rng << 13;
            rng ^= rng >> 7;
            rng ^= rng << 17;
            noise.push((rng & 0xff) as u8);
        }
        // Force alpha = 255 to keep the bilinear baseline honest.
        for i in 0..(w * h) {
            noise[(i * 4 + 3) as usize] = 255;
        }
        let (low_n, _, _) = box_downsample_2x(&noise, w, h).unwrap();

        let mut pred = BilinearPredictor;
        let bs_smooth =
            encode_texture(&mut pred, &smooth, w, h, low_s, lw, lh, 1, false).unwrap();
        let bs_noise =
            encode_texture(&mut pred, &noise, w, h, low_n, lw, lh, 1, false).unwrap();

        assert!(
            bs_smooth.residual_zst.len() < bs_noise.residual_zst.len(),
            "smooth residual ({}) should be smaller than noise residual ({})",
            bs_smooth.residual_zst.len(),
            bs_noise.residual_zst.len()
        );
    }

    #[test]
    fn hash_check_catches_predictor_mismatch() {
        // If decode is called with the wrong predictor id, we should error
        // before producing garbage.
        let (w, h) = (16u32, 16u32);
        let top = rgba_gradient(w, h);
        let (low, lw, lh) = box_downsample_2x(&top, w, h).unwrap();
        let mut pred = BilinearPredictor;
        let mut bs = encode_texture(&mut pred, &top, w, h, low, lw, lh, 1, true).unwrap();
        // Tamper: claim it was an esrgan bitstream.
        bs.predictor = PredictorId::RealEsrganX4;
        let mut decoder = BilinearPredictor;
        let err = decode_texture(&mut decoder, &bs).unwrap_err();
        assert!(
            err.to_string().contains("predictor id mismatch"),
            "want predictor mismatch, got: {err}"
        );
    }

    #[test]
    fn quant_step_2_is_lossy_but_close() {
        // With quant_step=2 the residual is halved per-sample → we lose at
        // most ±1 per channel on round-trip. Assert reconstruction stays
        // within that bound.
        let (w, h) = (32u32, 32u32);
        let top = rgba_gradient(w, h);
        let (low, lw, lh) = box_downsample_2x(&top, w, h).unwrap();
        let mut pred = BilinearPredictor;
        let bs = encode_texture(&mut pred, &top, w, h, low, lw, lh, 2, false).unwrap();
        let mut dec = BilinearPredictor;
        let restored = decode_texture(&mut dec, &bs).unwrap();
        assert_eq!(restored.len(), top.len());
        let max_diff = top
            .iter()
            .zip(restored.iter())
            .map(|(a, b)| ((*a as i16) - (*b as i16)).unsigned_abs())
            .max()
            .unwrap();
        assert!(max_diff <= 1, "quant_step=2 should keep max diff ≤ 1, got {max_diff}");
    }

    #[test]
    fn size_breakdown_is_consistent() {
        let (w, h) = (16u32, 16u32);
        let top = rgba_gradient(w, h);
        let (low, lw, lh) = box_downsample_2x(&top, w, h).unwrap();
        let mut pred = BilinearPredictor;
        let bs = encode_texture(&mut pred, &top, w, h, low, lw, lh, 1, true).unwrap();
        let s = bs.size();
        assert_eq!(s.low_mip_bytes, bs.low_mip_rgba.len());
        assert_eq!(s.residual_zst_bytes, bs.residual_zst.len());
        assert_eq!(
            s.total_bytes,
            s.low_mip_bytes + s.residual_zst_bytes + s.overhead_bytes
        );
    }

    #[test]
    fn rejects_wrong_top_buffer_size() {
        let (w, h) = (8u32, 8u32);
        let bad = vec![0u8; (w * h * 4 - 4) as usize];
        let low = rgba_solid(4, 4, [0; 4]);
        let mut pred = BilinearPredictor;
        let err = encode_texture(&mut pred, &bad, w, h, low, 4, 4, 1, false).unwrap_err();
        assert!(err.to_string().contains("top mip rgba length"), "{err}");
    }

    #[test]
    fn rejects_quant_step_zero() {
        let (w, h) = (4u32, 4u32);
        let top = rgba_solid(w, h, [10; 4]);
        let low = rgba_solid(2, 2, [10; 4]);
        let mut pred = BilinearPredictor;
        let err = encode_texture(&mut pred, &top, w, h, low, 2, 2, 0, false).unwrap_err();
        assert!(err.to_string().contains("quant_step"), "{err}");
    }
}
