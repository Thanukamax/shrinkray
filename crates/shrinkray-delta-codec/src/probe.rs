//! G13 — adaptive codec-space selector.
//!
//! Pixel-space Δ-Codec wins on predictable content (smooth/textured) because
//! the bilinear predictor reduces residual entropy strongly. BC-byte-space
//! Δ-Codec wins on noisy content because BC quantization absorbs prediction
//! error structurally. An adaptive scheme that picks per-texture pushes
//! every sample into the "wins" column.
//!
//! The probe is intentionally cheap (O(pixels) high-pass + variance) so
//! it can run on every texture in a pak without becoming the bottleneck.
//! The threshold is tuned against the bench samples: smooth (high-pass
//! energy ~0), textured (low-to-mid), noise (high).

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CodecSpace {
    /// Residual computed on RGBA pixels — best when the predictor can
    /// reproduce most of the high mip from the low mip (smooth/textured).
    Pixel,
    /// Residual computed on BC bytes — best when prediction error is large
    /// in pixel space but BC quantization absorbs it (noisy / hard content).
    Bc3Byte,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CodecSpaceRecommendation {
    pub recommended: CodecSpace,
    /// High-pass energy per pixel (mean |center − neighbour-avg|). Scale: 0
    /// (constant) to ~96 (random uniform). The threshold lives in
    /// [`HIGH_PASS_THRESHOLD`].
    pub high_pass_energy: f64,
    /// Variance across patch means, normalised. Higher = more global
    /// structure / less translation-invariance. Used as a tiebreaker.
    pub patch_variance: f64,
    /// One-line reason the probe gave that answer. Surfaces in the UI
    /// alongside the choice so a user can sanity-check the routing.
    pub basis: String,
}

/// Threshold tuned empirically against the bench samples (smooth_gradient ~
/// 0.5 bpp HP, textured_gradient ~ 6.5 bpp HP, high_freq_noise ~ 85 bpp HP).
/// Pixel-space stays competitive up to ~25; BC-byte takes over above.
pub const HIGH_PASS_THRESHOLD: f64 = 25.0;

/// Cheap probe: high-pass energy + patch variance from a single RGBA mip.
///
/// Linear pass over pixels — O(w·h). Allocates nothing larger than two
/// scalars + a few thousand patch means.
pub fn probe_codec_space(rgba: &[u8], w: u32, h: u32) -> CodecSpaceRecommendation {
    let energy = high_pass_energy(rgba, w, h);
    let pv = patch_variance(rgba, w, h, 8);
    let (recommended, basis) = if energy < HIGH_PASS_THRESHOLD {
        (
            CodecSpace::Pixel,
            format!(
                "low high-pass energy ({:.2} < {:.0}) → bilinear predictor will reduce residual well",
                energy, HIGH_PASS_THRESHOLD
            ),
        )
    } else {
        (
            CodecSpace::Bc3Byte,
            format!(
                "high high-pass energy ({:.2} ≥ {:.0}) → BC quantization will absorb prediction error",
                energy, HIGH_PASS_THRESHOLD
            ),
        )
    };
    CodecSpaceRecommendation {
        recommended,
        high_pass_energy: energy,
        patch_variance: pv,
        basis,
    }
}

/// Mean |center − avg(4 neighbours)| per pixel-channel, weighted across all 3
/// colour channels (alpha skipped — UE BC* texture alpha is rarely the
/// information-bearing channel for the AI-restore class).
fn high_pass_energy(rgba: &[u8], w: u32, h: u32) -> f64 {
    if w < 3 || h < 3 {
        return 0.0;
    }
    let mut sum = 0u64;
    let mut count = 0u64;
    let stride = (w * 4) as usize;
    for y in 1..(h - 1) {
        for x in 1..(w - 1) {
            let i = (y as usize) * stride + (x as usize) * 4;
            for c in 0..3 {
                let center = rgba[i + c] as i32;
                let up = rgba[i - stride + c] as i32;
                let down = rgba[i + stride + c] as i32;
                let left = rgba[i + c - 4] as i32;
                let right = rgba[i + c + 4] as i32;
                let avg = (up + down + left + right) / 4;
                sum += (center - avg).unsigned_abs() as u64;
                count += 1;
            }
        }
    }
    if count == 0 {
        0.0
    } else {
        (sum as f64) / (count as f64)
    }
}

/// Variance of mean luma across non-overlapping `patch×patch` tiles. Higher
/// = more global structure, lower = locally translation-invariant.
fn patch_variance(rgba: &[u8], w: u32, h: u32, patch: u32) -> f64 {
    if w < patch || h < patch {
        return 0.0;
    }
    let mut means = Vec::new();
    let tiles_x = w / patch;
    let tiles_y = h / patch;
    for ty in 0..tiles_y {
        for tx in 0..tiles_x {
            let mut acc = 0u64;
            for py in 0..patch {
                for px in 0..patch {
                    let i = ((ty * patch + py) as usize) * (w as usize) * 4
                        + ((tx * patch + px) as usize) * 4;
                    let r = rgba[i] as u64;
                    let g = rgba[i + 1] as u64;
                    let b = rgba[i + 2] as u64;
                    acc += (r * 299 + g * 587 + b * 114) / 1000;
                }
            }
            means.push(acc as f64 / (patch * patch) as f64);
        }
    }
    if means.is_empty() {
        return 0.0;
    }
    let m = means.iter().sum::<f64>() / means.len() as f64;
    means.iter().map(|x| (x - m).powi(2)).sum::<f64>() / means.len() as f64
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rgba_smooth(w: u32, h: u32) -> Vec<u8> {
        let mut out = Vec::with_capacity((w * h * 4) as usize);
        for y in 0..h {
            for x in 0..w {
                let g = ((x as u32 * 256 / w) & 0xff) as u8;
                out.push(g);
                out.push(g / 2 + 64);
                out.push(255 - g);
                out.push(255);
            }
        }
        out
    }

    fn rgba_textured(w: u32, h: u32) -> Vec<u8> {
        let mut out = Vec::with_capacity((w * h * 4) as usize);
        for y in 0..h {
            for x in 0..w {
                let g = ((x as u32 * 256 / w) & 0xff) as u8;
                let bump = (((x ^ y) & 0x0f) as u8) * 4;
                out.push(g.saturating_add(bump));
                out.push(g.saturating_sub(bump / 2));
                out.push(128u8.saturating_add(bump));
                out.push(255);
            }
        }
        out
    }

    fn rgba_noise(w: u32, h: u32) -> Vec<u8> {
        let mut rng: u64 = 0x9e37_79b9_7f4a_7c15;
        let mut out = Vec::with_capacity((w * h * 4) as usize);
        for _ in 0..(w * h) {
            rng ^= rng << 13;
            rng ^= rng >> 7;
            rng ^= rng << 17;
            out.push((rng & 0xff) as u8);
            out.push(((rng >> 8) & 0xff) as u8);
            out.push(((rng >> 16) & 0xff) as u8);
            out.push(255);
        }
        out
    }

    #[test]
    fn smooth_picks_pixel_space() {
        let r = probe_codec_space(&rgba_smooth(64, 64), 64, 64);
        assert_eq!(r.recommended, CodecSpace::Pixel, "{}", r.basis);
        assert!(
            r.high_pass_energy < HIGH_PASS_THRESHOLD,
            "smooth HP={}",
            r.high_pass_energy
        );
    }

    #[test]
    fn noise_picks_bc_space() {
        let r = probe_codec_space(&rgba_noise(64, 64), 64, 64);
        assert_eq!(r.recommended, CodecSpace::Bc3Byte, "{}", r.basis);
        assert!(
            r.high_pass_energy >= HIGH_PASS_THRESHOLD,
            "noise HP={}",
            r.high_pass_energy
        );
    }

    #[test]
    fn textured_lands_in_pixel_space() {
        // The textured sample we use in the bench has a low-amplitude xor
        // bump on top of a smooth gradient. Probe should still pick pixel.
        let r = probe_codec_space(&rgba_textured(64, 64), 64, 64);
        assert_eq!(r.recommended, CodecSpace::Pixel, "{}", r.basis);
    }

    #[test]
    fn solid_color_picks_pixel_space() {
        // Degenerate case: pure solid color. Zero high-pass energy. Pixel.
        let solid = vec![128u8; 64 * 64 * 4];
        let r = probe_codec_space(&solid, 64, 64);
        assert_eq!(r.recommended, CodecSpace::Pixel);
        assert_eq!(r.high_pass_energy, 0.0);
    }

    #[test]
    fn rejects_too_small_input_gracefully() {
        // < 3 pixels in either dim returns 0 high-pass energy + Pixel pick.
        let tiny = vec![0u8; 4 * 4];  // 1x1 RGBA
        let r = probe_codec_space(&tiny, 1, 1);
        assert_eq!(r.high_pass_energy, 0.0);
        assert_eq!(r.recommended, CodecSpace::Pixel);
    }
}
