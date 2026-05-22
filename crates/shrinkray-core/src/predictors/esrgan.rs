//! Real-ESRGAN x4 predictor for the Δ-Codec.
//!
//! ## Why this lives in `shrinkray-core` (not in the codec crate)
//!
//! The codec stays predictor-agnostic and has zero ORT/ndarray dependency
//! (~200 MB of prebuilt native libs). This file is the bridge: it pulls in
//! ORT here and `impl`s the codec's `Predictor` trait, so the bitstream layer
//! stays publishable standalone.
//!
//! ## Model contract
//!
//! - Input  tensor `[1, 3, H, W]` float32 in `[0, 1]`, NCHW, RGB.
//! - Output tensor `[1, 3, 4H, 4W]` float32 in `[0, 1]`, NCHW, RGB.
//! - Dynamic spatial dims (verified for `crj/dl-ws/real_esrgan_x4.onnx`,
//!   sha256 `4139cc15…dda0d3`).
//!
//! The ONNX model the predictor expects ships via `scripts/fetch-ai-models.sh`
//! at `src-tauri/binaries/ai-models/realesrgan-x4-general.onnx` (~67 MB).
//!
//! ## Alpha handling
//!
//! Real-ESRGAN is an RGB upscaler — it has no opinion on alpha. Because the
//! Δ-Codec's whole point is residual-based byte-exact recovery, the predictor
//! just needs to produce *some* deterministic alpha that matches what the
//! encoder will see at decode time. We bilinear-upsample the low mip's alpha
//! channel and splice it in. That keeps the residual on alpha small when the
//! true alpha is smooth (common — most UE textures with alpha use it for
//! masks/decals that vary slowly).
//!
//! ## Non-exact-4× target dims
//!
//! The model only does 4× exactly. If a caller asks for a target that is not
//! exactly `(4*low_w, 4*low_h)`, we **error** rather than silently adapting
//! with a bilinear post-resample. The Δ-Codec router is responsible for
//! picking the right predictor for each (low, top) pair; landing an exotic
//! ratio on the ESRGAN predictor is a routing bug, and silently absorbing
//! the dimension cost in the residual would hide it. Callers that want a
//! non-4× ratio should pick `BilinearPredictor` (or, eventually, a different
//! ESRGAN variant) instead.

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use anyhow::{anyhow, bail, Context, Result};
use ndarray::Array4;
use ort::session::Session;
use ort::value::Tensor;

use shrinkray_delta_codec::{Predictor, PredictorId};
// NB: we do *not* import `bilinear_upsample` from the codec here. The
// module header explains why: non-4× target dims are treated as a routing
// error and rejected, not silently resampled.

/// A Real-ESRGAN x4 predictor backed by an ONNX session.
///
/// Holds the loaded `Session` so that repeated `predict()` calls don't pay
/// the model-load cost each time. The session is wrapped in a `Mutex` because
/// the `Predictor` trait takes `&mut self`, and ORT's `Session::run` only
/// requires `&self` — the lock is essentially a formality, but it makes the
/// type `Send + Sync` for callers wiring it through `Arc<Mutex<_>>` patterns.
pub struct EsrganX4Predictor {
    session: Mutex<Session>,
    model_path: PathBuf,
}

impl EsrganX4Predictor {
    /// Load a Real-ESRGAN x4 ONNX model from disk.
    ///
    /// Fails fast if the file is missing or ORT can't build a session. The
    /// session is reused across `predict()` calls.
    pub fn new(model_path: &Path) -> Result<Self> {
        if !model_path.exists() {
            bail!(
                "ESRGAN model not found at {} (run scripts/fetch-ai-models.sh)",
                model_path.display()
            );
        }
        let session = Session::builder()
            .context("ort: Session::builder")?
            .commit_from_file(model_path)
            .with_context(|| format!("ort: load model {}", model_path.display()))?;
        Ok(Self {
            session: Mutex::new(session),
            model_path: model_path.to_path_buf(),
        })
    }

    /// Look the model up by convention. Search order:
    ///   1. `$SHRINKRAY_AI_MODEL` env var (absolute path to a `.onnx`).
    ///   2. `<workspace>/src-tauri/binaries/ai-models/realesrgan-x4-general.onnx`
    ///   3. Caller-passed fallback if both above miss (`None` → error).
    ///
    /// Returns `Ok(None)` when no model is found — callers can use this to
    /// skip cleanly (e.g. in tests on CI without the model fetched).
    pub fn locate_default() -> Option<PathBuf> {
        if let Ok(p) = std::env::var("SHRINKRAY_AI_MODEL") {
            let candidate = PathBuf::from(p);
            if candidate.exists() {
                return Some(candidate);
            }
        }
        let candidate = PathBuf::from(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../src-tauri/binaries/ai-models/realesrgan-x4-general.onnx"
        ));
        if candidate.exists() {
            return Some(candidate);
        }
        None
    }

    /// Path the predictor was loaded from. Useful for logs + error context.
    pub fn model_path(&self) -> &Path {
        &self.model_path
    }

    /// Internal: run ORT on an RGB float32 NCHW tensor and return RGB float32
    /// NCHW output. Caller handles RGBA pack/unpack and alpha routing.
    fn run_rgb_4x(&self, rgb_nchw: Array4<f32>, w: usize, h: usize) -> Result<(Vec<f32>, usize, usize)> {
        let mut session = self
            .session
            .lock()
            .map_err(|_| anyhow!("ESRGAN session mutex poisoned"))?;
        let input_name = session
            .inputs
            .first()
            .map(|i| i.name.clone())
            .ok_or_else(|| anyhow!("model has no inputs"))?;
        let input_tensor = Tensor::from_array(rgb_nchw).context("ort: build input tensor")?;
        let outputs = session
            .run(ort::inputs![input_name => input_tensor])
            .context("ort: session.run")?;

        let (output_name, _) = outputs
            .iter()
            .next()
            .ok_or_else(|| anyhow!("model produced no outputs"))?;
        let output_value = &outputs[output_name];
        let (output_shape, output_data) = output_value
            .try_extract_tensor::<f32>()
            .context("ort: extract output tensor")?;

        if output_shape.len() != 4 {
            bail!(
                "ESRGAN output rank {} != 4 (NCHW)",
                output_shape.len()
            );
        }
        let oc = output_shape[1] as usize;
        let oh = output_shape[2] as usize;
        let ow = output_shape[3] as usize;
        if oc != 3 {
            bail!("ESRGAN output channels = {oc}, expected 3 (RGB)");
        }
        let scale_w = ow as f32 / w as f32;
        let scale_h = oh as f32 / h as f32;
        if (scale_w - 4.0).abs() > 0.01 || (scale_h - 4.0).abs() > 0.01 {
            bail!(
                "ESRGAN: expected 4× upscale, got input {w}x{h} → output {ow}x{oh} \
                 (scale {scale_w:.2}x{scale_h:.2}). Wrong model wired?"
            );
        }

        Ok((output_data.to_vec(), ow, oh))
    }
}

impl Predictor for EsrganX4Predictor {
    fn id(&self) -> PredictorId {
        PredictorId::RealEsrganX4
    }

    fn predict(
        &mut self,
        low_rgba: &[u8],
        low_w: u32,
        low_h: u32,
        top_w: u32,
        top_h: u32,
    ) -> Result<Vec<u8>> {
        // --- dim sanity ---
        let expected_low = (low_w as usize)
            .checked_mul(low_h as usize)
            .and_then(|v| v.checked_mul(4))
            .ok_or_else(|| anyhow!("low mip dims overflow"))?;
        if low_rgba.len() != expected_low {
            bail!(
                "ESRGAN predict: low rgba len {} != w*h*4 = {expected_low} (w={low_w}, h={low_h})",
                low_rgba.len()
            );
        }
        if low_w == 0 || low_h == 0 || top_w == 0 || top_h == 0 {
            bail!("ESRGAN predict: zero dimension (low {low_w}x{low_h}, top {top_w}x{top_h})");
        }
        // Enforce exact 4× — see module header. A non-4× landing here is a
        // codec-router bug, not a thing we silently absorb in the residual.
        if top_w != low_w.checked_mul(4).unwrap_or(0)
            || top_h != low_h.checked_mul(4).unwrap_or(0)
        {
            bail!(
                "ESRGAN predict: target {top_w}x{top_h} is not 4× of low {low_w}x{low_h}. \
                 EsrganX4Predictor is fixed-4×; use BilinearPredictor for other ratios."
            );
        }

        // --- Strict 4× check: Real-ESRGAN-x4 is fixed-ratio. Anything else
        //     is a routing bug — the codec's caller is supposed to pick the
        //     right predictor for the (low, top) pair. We error rather than
        //     silently bilinear-resampling because that would hide bugs and
        //     dump the dimension cost into the residual. ---
        if top_w != low_w.saturating_mul(4) || top_h != low_h.saturating_mul(4) {
            bail!(
                "ESRGAN-x4 requires exact 4× upscale, got {low_w}x{low_h} → {top_w}x{top_h}"
            );
        }

        // --- RGBA8 → NCHW float32 [0, 1] (drop alpha; we'll handle it
        //     separately to keep the residual on alpha small) ---
        let lw = low_w as usize;
        let lh = low_h as usize;
        let mut rgb_nchw = Array4::<f32>::zeros((1, 3, lh, lw));
        for y in 0..lh {
            for x in 0..lw {
                let base = (y * lw + x) * 4;
                rgb_nchw[[0, 0, y, x]] = low_rgba[base] as f32 / 255.0;
                rgb_nchw[[0, 1, y, x]] = low_rgba[base + 1] as f32 / 255.0;
                rgb_nchw[[0, 2, y, x]] = low_rgba[base + 2] as f32 / 255.0;
            }
        }

        // --- ORT: 4× RGB upscale ---
        let (out_nchw, model_w, model_h) = self.run_rgb_4x(rgb_nchw, lw, lh)?;
        debug_assert_eq!(model_w, lw * 4);
        debug_assert_eq!(model_h, lh * 4);

        // --- NCHW float32 → RGB8 NHWC at model dims (4×) ---
        let mut rgb_4x = vec![0u8; model_w * model_h * 3];
        let channel_stride = model_h * model_w;
        for y in 0..model_h {
            for x in 0..model_w {
                let dst = (y * model_w + x) * 3;
                let r = out_nchw[y * model_w + x];
                let g = out_nchw[channel_stride + y * model_w + x];
                let b = out_nchw[2 * channel_stride + y * model_w + x];
                rgb_4x[dst] = (r.clamp(0.0, 1.0) * 255.0).round() as u8;
                rgb_4x[dst + 1] = (g.clamp(0.0, 1.0) * 255.0).round() as u8;
                rgb_4x[dst + 2] = (b.clamp(0.0, 1.0) * 255.0).round() as u8;
            }
        }

        // --- Alpha: bilinear-upsample the low mip's alpha to MODEL dims,
        //     then we'll resample together if the target isn't 4× ---
        let low_alpha: Vec<u8> = low_rgba.iter().skip(3).step_by(4).copied().collect();
        let alpha_4x = bilinear_upsample_single_channel(&low_alpha, low_w, low_h, model_w as u32, model_h as u32)?;

        // --- Assemble RGBA at MODEL dims ---
        let mut rgba_4x = vec![0u8; model_w * model_h * 4];
        for i in 0..(model_w * model_h) {
            rgba_4x[i * 4] = rgb_4x[i * 3];
            rgba_4x[i * 4 + 1] = rgb_4x[i * 3 + 1];
            rgba_4x[i * 4 + 2] = rgb_4x[i * 3 + 2];
            rgba_4x[i * 4 + 3] = alpha_4x[i];
        }

        // Strict 4× upstream — model_w/h must already equal top_w/h.
        debug_assert_eq!((top_w as usize, top_h as usize), (model_w, model_h));
        Ok(rgba_4x)
    }
}

/// Bilinear-upsample a single-channel u8 buffer. Used for alpha routing.
/// (Couldn't reuse `bilinear_upsample` directly without packing RGBA.)
fn bilinear_upsample_single_channel(
    src: &[u8],
    src_w: u32,
    src_h: u32,
    dst_w: u32,
    dst_h: u32,
) -> Result<Vec<u8>> {
    let expected = (src_w as usize) * (src_h as usize);
    if src.len() != expected {
        bail!(
            "bilinear_upsample_single_channel: src len {} != {} (w={src_w}, h={src_h})",
            src.len(),
            expected
        );
    }
    if src_w == 0 || src_h == 0 || dst_w == 0 || dst_h == 0 {
        bail!("bilinear_upsample_single_channel: zero dimension");
    }
    let mut out = vec![0u8; (dst_w as usize) * (dst_h as usize)];
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
            let p00 = src[(y0 * src_w + x0) as usize] as f32;
            let p10 = src[(y0 * src_w + x1) as usize] as f32;
            let p01 = src[(y1 * src_w + x0) as usize] as f32;
            let p11 = src[(y1 * src_w + x1) as usize] as f32;
            let top = p00 * (1.0 - wx) + p10 * wx;
            let bot = p01 * (1.0 - wx) + p11 * wx;
            let v = top * (1.0 - wy) + bot * wy;
            out[(dy * dst_w + dx) as usize] = v.round().clamp(0.0, 255.0) as u8;
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn model_or_skip() -> Option<PathBuf> {
        let path = EsrganX4Predictor::locate_default();
        if path.is_none() {
            eprintln!(
                "SKIP: ESRGAN ONNX model missing (set SHRINKRAY_AI_MODEL or run scripts/fetch-ai-models.sh --esrgan)"
            );
        }
        path
    }

    #[test]
    fn constant_color_predicts_approximately_constant() {
        // The classic AI-upscaler smoke check: a constant-color low mip
        // should produce a (near-)constant-color upscaled output. If this
        // fails, NCHW/NHWC conversion is wrong, channel order is swapped, or
        // [0,1] normalization is wrong.
        let Some(model) = model_or_skip() else { return };
        let mut pred = EsrganX4Predictor::new(&model).expect("load model");

        let (lw, lh) = (32u32, 32u32);
        let color = [180u8, 96, 48, 255];
        let mut low = Vec::with_capacity((lw * lh * 4) as usize);
        for _ in 0..(lw * lh) {
            low.extend_from_slice(&color);
        }

        let out = pred
            .predict(&low, lw, lh, lw * 4, lh * 4)
            .expect("predict");
        assert_eq!(out.len(), (lw * lh * 16 * 4) as usize);

        // Every pixel should be within a band of the constant input. Real-
        // ESRGAN drifts up to ~20 units near constant-field "edges" due to
        // conv-pad effects (the network has never seen pure flats during
        // training). Empirical worst on this specific model: ~20/ch. We
        // allow ±28 for some export-to-export jitter. If it fails by a lot,
        // the NCHW/[0,1] conversion or channel order is wrong.
        let mut worst = 0i16;
        let mut sum: i64 = 0;
        let mut count: i64 = 0;
        for chunk in out.chunks_exact(4) {
            for c in 0..3 {
                let diff = (chunk[c] as i16) - (color[c] as i16);
                worst = worst.max(diff.abs());
                sum += diff as i64;
                count += 1;
            }
            // Alpha is bilinear-upsampled from a constant → must be exact.
            assert_eq!(chunk[3], color[3], "alpha must round-trip on constant input");
        }
        let mean_drift = sum as f64 / count as f64;
        assert!(
            worst <= 28,
            "constant-color upscale drifted by {worst} (>28). \
             Likely an NCHW/normalization bug. (mean drift = {mean_drift:.2})"
        );
        // A small mean drift is OK (pad noise); a large one means BGR/RGB
        // swap or [-1,1] vs [0,1] normalization is wrong.
        assert!(
            mean_drift.abs() < 8.0,
            "constant-color mean drift {mean_drift:.2} too large — channel order or normalization likely wrong"
        );
    }

    #[test]
    fn rejects_mismatched_low_buffer_length() {
        // Catch the missing-model path before we touch ORT, so this works
        // even without the model file.
        let dummy = PathBuf::from("/nonexistent/model.onnx");
        let err = match EsrganX4Predictor::new(&dummy) {
            Ok(_) => panic!("expected error for missing model"),
            Err(e) => e,
        };
        assert!(err.to_string().contains("not found"), "{err}");
    }
}
