//! v0.7.4 — End-to-end Δ-Codec round-trip with the real ESRGAN predictor.
//!
//! Thesis under test: "an AI predictor shrinks the residual enough that the
//! bitstream beats a byte-exact full backup, *while* still recovering
//! byte-exact at `quant_step = 1`." Bilinear-predictor tests already cover
//! the lossless property for a transparent baseline; this file is the first
//! time the full pipeline runs with the production ESRGAN predictor.
//!
//! Skip behavior: when the ONNX model is missing (CI without
//! `scripts/fetch-ai-models.sh --esrgan`) we print SKIP and return Ok rather
//! than failing the suite. The model is a 67 MB blob and we don't ship it.

#![cfg(feature = "inference")]

use shrinkray_core::predictors::EsrganX4Predictor;
use shrinkray_delta_codec::{
    box_downsample_2x, decode_texture, encode_texture, BilinearPredictor, PredictorId,
};

/// Build a 512×512 RGBA buffer with a mix of smooth gradients (testing the
/// low-frequency reconstruction path) and sharp checker / stripe regions
/// (testing the residual carries the high-frequency content the predictor
/// can't reproduce). Deterministic — same bytes on every run.
fn build_test_top_512() -> Vec<u8> {
    let (w, h) = (512usize, 512usize);
    let mut out = vec![0u8; w * h * 4];
    for y in 0..h {
        for x in 0..w {
            let i = (y * w + x) * 4;
            let in_left = x < w / 2;
            let in_top = y < h / 2;
            // Four-quadrant content mix:
            //   TL: smooth diagonal gradient (predictor's friend).
            //   TR: 8×8 checkerboard (sharp high-freq).
            //   BL: 4-px vertical stripes (sharp high-freq).
            //   BR: smooth radial gradient.
            let (r, g, b) = match (in_top, in_left) {
                (true, true) => ((x / 2) as u8, (y / 2) as u8, 128u8),
                (true, false) => {
                    let xx = (x - w / 2) / 8;
                    let yy = y / 8;
                    if (xx + yy) % 2 == 0 {
                        (220, 220, 220)
                    } else {
                        (40, 40, 40)
                    }
                }
                (false, true) => {
                    if (x / 4) % 2 == 0 {
                        (200, 60, 60)
                    } else {
                        (60, 60, 200)
                    }
                }
                (false, false) => {
                    let cx = (3 * w / 4) as f32;
                    let cy = (3 * h / 4) as f32;
                    let dx = x as f32 - cx;
                    let dy = y as f32 - cy;
                    let d = (dx * dx + dy * dy).sqrt();
                    let max_d = (w / 4) as f32 * std::f32::consts::SQRT_2;
                    let t = (d / max_d).clamp(0.0, 1.0);
                    let v = (255.0 * (1.0 - t)) as u8;
                    (v, v / 2, 255 - v)
                }
            };
            out[i] = r;
            out[i + 1] = g;
            out[i + 2] = b;
            // Slowly varying alpha so the predictor's bilinear-alpha path
            // produces something close to truth (residual on alpha stays
            // small but is still exercised).
            out[i + 3] = (200 + (x.wrapping_add(y) % 56)) as u8;
        }
    }
    out
}

#[test]
fn esrgan_round_trip_128_to_512_byte_exact() {
    let Some(model_path) = EsrganX4Predictor::locate_default() else {
        eprintln!(
            "SKIP esrgan_round_trip_128_to_512_byte_exact: no ESRGAN model. \
             Run `bash scripts/fetch-ai-models.sh --esrgan` or set SHRINKRAY_AI_MODEL=…"
        );
        return;
    };
    eprintln!(
        "[esrgan_round_trip] loading model from {}",
        model_path.display()
    );

    let mut predictor = match EsrganX4Predictor::new(&model_path) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("SKIP: ESRGAN load failed: {e}");
            return;
        }
    };

    // 512 → 256 → 128. This matches how production derives a low mip from
    // a decoded BCn top when the cooker's intermediate mip isn't available.
    let (top_w, top_h) = (512u32, 512u32);
    let top_512 = build_test_top_512();
    assert_eq!(top_512.len(), (top_w * top_h * 4) as usize);

    let (mid_256, mw, mh) =
        box_downsample_2x(&top_512, top_w, top_h).expect("downsample 512 → 256");
    assert_eq!((mw, mh), (256, 256));
    let (low_128, lw, lh) = box_downsample_2x(&mid_256, mw, mh).expect("downsample 256 → 128");
    assert_eq!((lw, lh), (128, 128));
    assert_eq!(low_128.len(), (lw * lh * 4) as usize);

    // Encode: low mip + residual against ESRGAN's prediction.
    eprintln!("[esrgan_round_trip] encoding {lw}x{lh} → {top_w}x{top_h}");
    let bitstream = encode_texture(
        &mut predictor,
        &top_512,
        top_w,
        top_h,
        low_128.clone(),
        lw,
        lh,
        /* quant_step = */ 1,
        /* record_hash = */ true,
    )
    .expect("encode_texture");

    assert_eq!(bitstream.predictor, PredictorId::RealEsrganX4);
    assert_eq!(bitstream.quant_step, 1);
    assert_eq!((bitstream.top_w, bitstream.top_h), (top_w, top_h));
    assert_eq!((bitstream.low_w, bitstream.low_h), (lw, lh));
    assert!(
        bitstream.original_sha256.is_some(),
        "record_hash=true must populate original_sha256"
    );
    eprintln!(
        "[esrgan_round_trip] bitstream: low_mip {} B, residual_zst {} B, total ≈ {} B",
        bitstream.low_mip_rgba.len(),
        bitstream.residual_zst.len(),
        bitstream.low_mip_rgba.len() + bitstream.residual_zst.len()
    );

    // Decode: byte-exact recovery at quant_step=1.
    let decoded = decode_texture(&mut predictor, &bitstream).expect("decode_texture");
    assert_eq!(decoded.len(), top_512.len(), "decoded length mismatch");
    if decoded != top_512 {
        // Find the first divergence for debugging.
        let mut first_diff = None;
        let mut worst_diff = 0i16;
        for (i, (a, b)) in decoded.iter().zip(top_512.iter()).enumerate() {
            let d = (*a as i16 - *b as i16).abs();
            if d != 0 && first_diff.is_none() {
                first_diff = Some((i, *a, *b));
            }
            worst_diff = worst_diff.max(d);
        }
        panic!(
            "esrgan_round_trip FAIL: decoded != top_512 at quant_step=1. \
             first diff at byte {:?}, worst per-byte diff {worst_diff}",
            first_diff
        );
    }

    // Telemetry: residual size vs bilinear baseline. Useful anchor for the
    // paper bench, not a pass/fail condition — on a synthetic mix of
    // gradient+checker the comparison can go either way.
    let mut baseline = BilinearPredictor;
    let bs_bilinear = encode_texture(
        &mut baseline,
        &top_512,
        top_w,
        top_h,
        low_128,
        lw,
        lh,
        1,
        false,
    )
    .expect("bilinear encode for size compare");
    let esrgan_bytes = bitstream.residual_zst.len() as f64;
    let bilinear_bytes = bs_bilinear.residual_zst.len() as f64;
    let ratio = esrgan_bytes / bilinear_bytes.max(1.0);
    eprintln!(
        "[esrgan_round_trip] residual sizes: esrgan={:.0}B  bilinear={:.0}B  ratio={ratio:.2}× \
         (synthetic content; see bench_real_content.rs for natural-image numbers)",
        esrgan_bytes, bilinear_bytes
    );

    eprintln!("[esrgan_round_trip] PASS — byte-exact recovery at quant_step=1");
}
