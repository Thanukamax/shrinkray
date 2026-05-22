//! G4 — BC encoder determinism verification.
//!
//! The on-disk anti-cheat-safety claim depends on the BC encoder being
//! deterministic: encode(rgba) called twice must produce byte-identical
//! BC bytes. If image_dds is non-deterministic we need a wrapper; if it is,
//! we can claim file-byte exactness in the paper.
//!
//! These tests probe BC1/BC3/BC5/BC7 across Fast/Slow quality settings.
//! Any non-determinism here surfaces as a failing assert with a byte index
//! diagnostic so we know exactly where the encoder drifted.

use image_dds::{ImageFormat, Mipmaps, Quality, SurfaceRgba8};

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

fn encode(rgba: &[u8], w: u32, h: u32, fmt: ImageFormat, quality: Quality) -> Vec<u8> {
    let surface = SurfaceRgba8 {
        width: w,
        height: h,
        depth: 1,
        layers: 1,
        mipmaps: 1,
        data: rgba,
    };
    surface
        .encode(fmt, quality, Mipmaps::Disabled)
        .expect("image_dds encode ok")
        .data
}

fn assert_byte_equal(a: &[u8], b: &[u8], label: &str) {
    assert_eq!(
        a.len(),
        b.len(),
        "[{label}] byte count differs: {} vs {}",
        a.len(),
        b.len()
    );
    for (i, (x, y)) in a.iter().zip(b.iter()).enumerate() {
        assert_eq!(
            x, y,
            "[{label}] byte {i} differs: {x:#04x} vs {y:#04x}"
        );
    }
}

#[test]
fn bc1_is_byte_deterministic_at_fast_quality() {
    let rgba = rgba_gradient(64, 64);
    let a = encode(&rgba, 64, 64, ImageFormat::BC1RgbaUnorm, Quality::Fast);
    let b = encode(&rgba, 64, 64, ImageFormat::BC1RgbaUnorm, Quality::Fast);
    assert_byte_equal(&a, &b, "BC1 Fast");
}

#[test]
fn bc3_is_byte_deterministic_at_fast_quality() {
    let rgba = rgba_gradient(64, 64);
    let a = encode(&rgba, 64, 64, ImageFormat::BC3RgbaUnorm, Quality::Fast);
    let b = encode(&rgba, 64, 64, ImageFormat::BC3RgbaUnorm, Quality::Fast);
    assert_byte_equal(&a, &b, "BC3 Fast");
}

#[test]
fn bc5_is_byte_deterministic_at_fast_quality() {
    let rgba = rgba_gradient(64, 64);
    let a = encode(&rgba, 64, 64, ImageFormat::BC5RgUnorm, Quality::Fast);
    let b = encode(&rgba, 64, 64, ImageFormat::BC5RgUnorm, Quality::Fast);
    assert_byte_equal(&a, &b, "BC5 Fast");
}

#[test]
fn bc7_is_byte_deterministic_at_fast_quality() {
    let rgba = rgba_gradient(64, 64);
    let a = encode(&rgba, 64, 64, ImageFormat::BC7RgbaUnorm, Quality::Fast);
    let b = encode(&rgba, 64, 64, ImageFormat::BC7RgbaUnorm, Quality::Fast);
    assert_byte_equal(&a, &b, "BC7 Fast");
}

#[test]
fn bc3_is_byte_deterministic_at_slow_quality() {
    // Higher quality settings tend to use more search, which is where
    // non-determinism creeps in. If this fails we know to pin Fast.
    let rgba = rgba_gradient(64, 64);
    let a = encode(&rgba, 64, 64, ImageFormat::BC3RgbaUnorm, Quality::Slow);
    let b = encode(&rgba, 64, 64, ImageFormat::BC3RgbaUnorm, Quality::Slow);
    assert_byte_equal(&a, &b, "BC3 Slow");
}

#[test]
fn bc7_is_byte_deterministic_at_slow_quality() {
    let rgba = rgba_gradient(64, 64);
    let a = encode(&rgba, 64, 64, ImageFormat::BC7RgbaUnorm, Quality::Slow);
    let b = encode(&rgba, 64, 64, ImageFormat::BC7RgbaUnorm, Quality::Slow);
    assert_byte_equal(&a, &b, "BC7 Slow");
}

#[test]
fn bc1_deterministic_on_noise_input() {
    // Noise stresses the encoder's search heuristics — if anything is going
    // to non-determinise, it's here. The synthetic noise sample is the same
    // one the bench harness uses.
    let mut rng: u64 = 0x9e37_79b9_7f4a_7c15;
    let mut rgba = Vec::with_capacity(64 * 64 * 4);
    for _ in 0..(64 * 64) {
        rng ^= rng << 13;
        rng ^= rng >> 7;
        rng ^= rng << 17;
        rgba.push((rng & 0xff) as u8);
        rgba.push(((rng >> 8) & 0xff) as u8);
        rgba.push(((rng >> 16) & 0xff) as u8);
        rgba.push(255);
    }
    let a = encode(&rgba, 64, 64, ImageFormat::BC1RgbaUnorm, Quality::Fast);
    let b = encode(&rgba, 64, 64, ImageFormat::BC1RgbaUnorm, Quality::Fast);
    assert_byte_equal(&a, &b, "BC1 noise");
}
