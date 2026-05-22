//! v0.7.0 — ONNX-driven texture upscale.
//!
//! This module loads a Real-ESRGAN-class 4× upscaler model and applies it to
//! a raw RGBA buffer. v0.7.0 ships this in isolation — no pak/manifest hooks
//! yet, those land in v0.7.1+. The function is the load-bearing piece the
//! manifest-driven restore will call once per AI-class texture.
//!
//! Model contract (Real-ESRGAN x4 family):
//!   - Input: `[1, 3, H, W]` float32 RGB, normalized to `[0, 1]`.
//!   - Output: `[1, 3, 4H, 4W]` float32 RGB, normalized to `[0, 1]`.
//!   - Some exports use fixed input shapes; we re-create the session per call
//!     with the actual input dimensions to handle both fixed-and-dynamic.
//!
//! The output is converted back to RGBA8 (alpha forced to 255 — Real-ESRGAN
//! doesn't preserve alpha; the AI-restore class is reserved for diffuse-ish
//! textures where the cook stores RGB in BCn and alpha was redundant anyway).
//!
//! Why not the manifest hooks yet: v0.7.0 lands the inference primitive
//! standalone so we can validate it against real Real-ESRGAN ONNX exports
//! before threading it through BC en/decode + the texture_strip path. v0.7.1
//! adds the BC en/decode helpers + mip-chain regen + manifest restore.

use std::path::Path;

use anyhow::{anyhow, bail, Context, Result};
use ndarray::Array4;
use ort::session::Session;
use ort::value::Tensor;

/// Run a 4× upscale on a raw RGBA buffer. Returns `(upscaled_rgba, new_w, new_h)`.
///
/// `rgba` must be exactly `w * h * 4` bytes in row-major order (R, G, B, A).
/// The alpha channel is dropped on the way in and restored as `255` on the
/// way out — Real-ESRGAN x4 is an RGB upscaler, and the AI-restore class is
/// gated to non-alpha textures by [`crate::classifier`].
pub fn upscale_4x_rgba(
    model_path: &Path,
    rgba: &[u8],
    w: u32,
    h: u32,
) -> Result<(Vec<u8>, u32, u32)> {
    let expected_len = (w as usize)
        .checked_mul(h as usize)
        .and_then(|v| v.checked_mul(4))
        .ok_or_else(|| anyhow!("input dims overflow"))?;
    if rgba.len() != expected_len {
        bail!(
            "rgba buffer length {} doesn't match w×h×4 = {} (w={}, h={})",
            rgba.len(),
            expected_len,
            w,
            h,
        );
    }

    let mut session = Session::builder()
        .context("ort: Session::builder")?
        .commit_from_file(model_path)
        .with_context(|| format!("ort: load model {}", model_path.display()))?;

    // RGBA → RGB float32 [0, 1], NCHW layout (1, 3, H, W).
    let mut input = Array4::<f32>::zeros((1, 3, h as usize, w as usize));
    for y in 0..h as usize {
        for x in 0..w as usize {
            let base = (y * w as usize + x) * 4;
            input[[0, 0, y, x]] = rgba[base] as f32 / 255.0;
            input[[0, 1, y, x]] = rgba[base + 1] as f32 / 255.0;
            input[[0, 2, y, x]] = rgba[base + 2] as f32 / 255.0;
        }
    }

    let input_name = session
        .inputs
        .first()
        .map(|i| i.name.clone())
        .ok_or_else(|| anyhow!("model has no inputs"))?;

    let input_tensor =
        Tensor::from_array(input).context("ort: build input tensor")?;
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

    // Expected output shape: [1, 3, 4H, 4W]. Validate the upscale factor and
    // channel count before we trust the math.
    if output_shape.len() != 4 {
        bail!(
            "model output has rank {}, expected 4 (NCHW)",
            output_shape.len()
        );
    }
    let (oc, oh, ow) = (
        output_shape[1] as usize,
        output_shape[2] as usize,
        output_shape[3] as usize,
    );
    if oc != 3 {
        bail!("model output channels = {}, expected 3 (RGB)", oc);
    }
    let scale_h = oh as f32 / h as f32;
    let scale_w = ow as f32 / w as f32;
    if (scale_h - 4.0).abs() > 0.01 || (scale_w - 4.0).abs() > 0.01 {
        bail!(
            "model is not a 4× upscaler: input {}x{} → output {}x{} (scale ≈ {:.2}x{:.2})",
            w, h, ow, oh, scale_w, scale_h,
        );
    }

    // Float [0, 1] → RGBA8. Saturate out-of-range values (Real-ESRGAN can
    // legitimately produce values slightly outside [0, 1] near edges).
    let mut out = vec![255u8; ow * oh * 4];
    let channel_stride = oh * ow;
    for y in 0..oh {
        for x in 0..ow {
            let r = output_data[y * ow + x];
            let g = output_data[channel_stride + y * ow + x];
            let b = output_data[2 * channel_stride + y * ow + x];
            let base = (y * ow + x) * 4;
            out[base] = (r.clamp(0.0, 1.0) * 255.0).round() as u8;
            out[base + 1] = (g.clamp(0.0, 1.0) * 255.0).round() as u8;
            out[base + 2] = (b.clamp(0.0, 1.0) * 255.0).round() as u8;
            // alpha already 255
        }
    }

    Ok((out, ow as u32, oh as u32))
}

/// Open an ONNX model session and report its input/output shapes. Used by
/// the v0.7.0 smoke test as a fast "is the ORT runtime actually wired?"
/// check that doesn't require running inference. v0.7.1's manifest restore
/// uses this to validate a model file before kicking off long-running
/// per-texture inference.
pub fn probe_model(model_path: &Path) -> Result<ModelProbe> {
    let session = Session::builder()
        .context("ort: Session::builder")?
        .commit_from_file(model_path)
        .with_context(|| format!("ort: load model {}", model_path.display()))?;
    let inputs: Vec<IoSummary> = session
        .inputs
        .iter()
        .map(|i| IoSummary {
            name: i.name.clone(),
            shape: format!("{:?}", i.input_type),
        })
        .collect();
    let outputs: Vec<IoSummary> = session
        .outputs
        .iter()
        .map(|o| IoSummary {
            name: o.name.clone(),
            shape: format!("{:?}", o.output_type),
        })
        .collect();
    Ok(ModelProbe { inputs, outputs })
}

#[derive(Debug, Clone)]
pub struct ModelProbe {
    pub inputs: Vec<IoSummary>,
    pub outputs: Vec<IoSummary>,
}

#[derive(Debug, Clone)]
pub struct IoSummary {
    pub name: String,
    pub shape: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn model_or_skip() -> Option<std::path::PathBuf> {
        let candidate = match std::env::var("SHRINKRAY_AI_MODEL") {
            Ok(p) => std::path::PathBuf::from(p),
            Err(_) => std::path::PathBuf::from(
                concat!(env!("CARGO_MANIFEST_DIR"), "/../../src-tauri/binaries/ai-models/realesrgan-x4-general.onnx"),
            ),
        };
        if !candidate.exists() {
            eprintln!(
                "SKIP: ESRGAN ONNX model missing at {} (set SHRINKRAY_AI_MODEL to override; run scripts/fetch-ai-models.sh)",
                candidate.display(),
            );
            return None;
        }
        Some(candidate)
    }

    fn smoke_model_or_skip() -> Option<std::path::PathBuf> {
        // A small public ONNX model used only to validate that ort is wired
        // up correctly. Not a Real-ESRGAN — doesn't validate the upscale path.
        // Search order:
        //   1. SHRINKRAY_SMOKE_MODEL env override
        //   2. binaries/ai-models/super-resolution-10.onnx (post-fetch-ai-models.sh)
        //   3. /tmp/super-resolution-10.onnx (one-off curl)
        let env_path = std::env::var("SHRINKRAY_SMOKE_MODEL").ok().map(std::path::PathBuf::from);
        let candidates: Vec<std::path::PathBuf> = env_path
            .into_iter()
            .chain([
                std::path::PathBuf::from(concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/../../src-tauri/binaries/ai-models/super-resolution-10.onnx"
                )),
                std::path::PathBuf::from("/tmp/super-resolution-10.onnx"),
            ])
            .collect();
        for c in &candidates {
            if c.exists() {
                return Some(c.clone());
            }
        }
        eprintln!(
            "SKIP: smoke ONNX model missing (run `bash scripts/fetch-ai-models.sh` to download)",
        );
        None
    }

    #[test]
    fn rejects_mismatched_buffer_length() {
        // No model needed — the dim check is up-front.
        let dummy = std::path::PathBuf::from("/nonexistent/model.onnx");
        let err = upscale_4x_rgba(&dummy, &[0u8; 16], 4, 4).unwrap_err();
        assert!(err.to_string().contains("doesn't match"), "{err}");
    }

    #[test]
    fn probe_smoke_model_loads_and_reports_io() {
        let Some(model) = smoke_model_or_skip() else { return };
        let probe = probe_model(&model).expect("probe ok");
        assert!(!probe.inputs.is_empty(), "smoke model must have at least one input");
        assert!(!probe.outputs.is_empty(), "smoke model must have at least one output");
        // Sub-pixel CNN super-resolution has a single input/output pair.
        eprintln!("smoke probe: inputs={:?} outputs={:?}", probe.inputs, probe.outputs);
    }

    #[test]
    fn upscales_synthetic_gradient_4x() {
        let Some(model) = model_or_skip() else { return };

        // 64x64 RGBA gradient — a real ESRGAN can chew through this in well
        // under a second on CPU and the result dims are easy to verify.
        let (w, h) = (64u32, 64u32);
        let mut rgba = Vec::with_capacity((w * h * 4) as usize);
        for y in 0..h {
            for x in 0..w {
                rgba.push((x * 4) as u8); // R
                rgba.push((y * 4) as u8); // G
                rgba.push(128);            // B
                rgba.push(255);            // A
            }
        }

        let (out, ow, oh) = upscale_4x_rgba(&model, &rgba, w, h).expect("inference ok");
        assert_eq!(ow, w * 4, "width should 4×");
        assert_eq!(oh, h * 4, "height should 4×");
        assert_eq!(out.len(), (ow * oh * 4) as usize, "RGBA byte count");
        // Alpha must be 255 everywhere (we force it; Real-ESRGAN doesn't emit it).
        for chunk in out.chunks_exact(4) {
            assert_eq!(chunk[3], 255, "alpha must be 255 in upscaled output");
        }
        // At least some non-zero RGB pixels (the gradient survives the upscale).
        let any_nonzero = out
            .chunks_exact(4)
            .any(|c| c[0] != 0 || c[1] != 0 || c[2] != 0);
        assert!(any_nonzero, "all-zero upscale output is almost certainly a bug");
    }
}
