//! ESRGAN-x4 smoke check.
//!
//! Loads the Real-ESRGAN-x4 predictor, runs it on a 128×128 constant-color
//! RGBA buffer, and asserts the 512×512 output mean per RGB channel is
//! within ±5 of the input constant. This catches NCHW/NHWC packing bugs
//! and channel-order swaps *before* the round-trip codec test runs, where
//! a bad predictor would otherwise produce a confusing residual-size
//! explosion instead of an obvious "the output isn't even close to the
//! input" failure.
//!
//! Skips cleanly (prints a SKIP line + exits 0) if the model isn't on
//! disk, so this can be wired into CI behind `cargo run` without a hard
//! requirement on the 67 MB download.
//!
//! Run with:
//!   cargo run -p shrinkray-core --release --example esrgan_smoke --features inference

#[cfg(not(feature = "inference"))]
fn main() {
    eprintln!("esrgan_smoke: build with --features inference");
    std::process::exit(0);
}

#[cfg(feature = "inference")]
fn main() -> anyhow::Result<()> {
    use shrinkray_core::predictors::EsrganX4Predictor;
    use shrinkray_delta_codec::Predictor;

    let Some(model_path) = EsrganX4Predictor::locate_default() else {
        eprintln!(
            "SKIP: ESRGAN ONNX model missing (set SHRINKRAY_AI_MODEL or run \
             scripts/fetch-ai-models.sh --esrgan)"
        );
        return Ok(());
    };
    eprintln!("[esrgan_smoke] loading model from {}", model_path.display());

    let mut predictor = EsrganX4Predictor::new(&model_path)?;

    let (low_w, low_h) = (128u32, 128u32);
    let (top_w, top_h) = (low_w * 4, low_h * 4);
    let color = [180u8, 96, 48, 255];

    let mut low = Vec::with_capacity((low_w * low_h * 4) as usize);
    for _ in 0..(low_w * low_h) {
        low.extend_from_slice(&color);
    }
    eprintln!(
        "[esrgan_smoke] predicting {low_w}x{low_h} → {top_w}x{top_h} (constant RGBA {color:?})"
    );

    let out = predictor.predict(&low, low_w, low_h, top_w, top_h)?;
    let expected_len = (top_w * top_h * 4) as usize;
    if out.len() != expected_len {
        anyhow::bail!(
            "esrgan_smoke: output byte count {} != expected {}",
            out.len(),
            expected_len
        );
    }

    // Per-channel mean. u32 accumulator is fine — 2048*2048 pixels max here.
    let pixels = (top_w * top_h) as u64;
    let mut sums = [0u64; 4];
    for chunk in out.chunks_exact(4) {
        sums[0] += chunk[0] as u64;
        sums[1] += chunk[1] as u64;
        sums[2] += chunk[2] as u64;
        sums[3] += chunk[3] as u64;
    }
    let means = [
        (sums[0] as f64) / (pixels as f64),
        (sums[1] as f64) / (pixels as f64),
        (sums[2] as f64) / (pixels as f64),
        (sums[3] as f64) / (pixels as f64),
    ];
    eprintln!(
        "[esrgan_smoke] input RGBA={color:?}, output means R={:.2} G={:.2} B={:.2} A={:.2}",
        means[0], means[1], means[2], means[3]
    );

    // Tolerance: ±5 per channel for RGB on a constant input. Real-ESRGAN
    // can drift a small amount near boundaries because of conv-pad, but
    // the *mean* over 512² pixels should be very close to the input.
    let tol = 5.0;
    for c in 0..3 {
        let drift = (means[c] - color[c] as f64).abs();
        if drift > tol {
            anyhow::bail!(
                "esrgan_smoke FAIL: channel {} mean {:.2} drifted {:.2} from input {} (>{tol})",
                ["R", "G", "B"][c],
                means[c],
                drift,
                color[c]
            );
        }
    }
    // Alpha is deterministic-bilinear from a constant — must be exact.
    if (means[3] - color[3] as f64).abs() > 0.5 {
        anyhow::bail!(
            "esrgan_smoke FAIL: alpha mean {:.2} != constant input {} (alpha must be exact)",
            means[3],
            color[3]
        );
    }

    eprintln!("[esrgan_smoke] PASS — channel means within ±{tol} of input");
    Ok(())
}
