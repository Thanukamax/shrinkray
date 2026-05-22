//! gen_synthetic_corpus — produce 30+ 1024×1024 RGBA8 PNGs covering the
//! content classes we expect the Δ-Codec to see in real games. The goal is
//! NOT to be photoreal — it's to span the high-pass-energy axis with stable,
//! reproducible bytes so paper numbers are repeatable from a clean clone.
//!
//! Mix (matches the experiment plan in the harness brief):
//!   - 5  smooth gradients          (radial/linear/diagonal/quad/conic)
//!   - 8  textured gradients/noise  (value noise at various octaves)
//!   - 4  voronoi cells             (4 seed densities)
//!   - 5  normal-map-style          (derivatives of a value-noise heightfield)
//!   - 4  high-freq noise           (pure xorshift PRNG)
//!   - 4  stripes/checker/edges     (hard-edge content — BC-killer worst case)
//!   - 2  constant-color blocks     (the codec floor — must be ≈ 0 residual)
//!
//! Total: 32 samples. All written to `paper/corpus/synthetic/`.
//!
//! Determinism: every sample seeds its own xorshift PRNG from its filename.
//! Regenerating the corpus produces byte-identical PNGs.
//!
//! Run:
//!   cargo run -p shrinkray-core --release --example gen_synthetic_corpus

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use image::{ImageBuffer, Rgba};

const DIM: u32 = 1024;

fn main() -> Result<()> {
    let out_dir = std::env::args()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("paper/corpus/synthetic"));
    std::fs::create_dir_all(&out_dir)
        .with_context(|| format!("create {}", out_dir.display()))?;

    let mut written = 0usize;

    // 1. Smooth gradients (5)
    written += write_sample(&out_dir, "smooth_radial.png", smooth_radial(DIM))?;
    written += write_sample(&out_dir, "smooth_linear_x.png", smooth_linear(DIM, false))?;
    written += write_sample(&out_dir, "smooth_linear_y.png", smooth_linear(DIM, true))?;
    written += write_sample(&out_dir, "smooth_diagonal.png", smooth_diagonal(DIM))?;
    written += write_sample(&out_dir, "smooth_quadratic.png", smooth_quadratic(DIM))?;

    // 2. Textured gradient / value noise (8). Varying octaves -> different HP
    // energies. Octave 1 is barely-textured, octave 6 is rich.
    for octaves in 1..=8u32 {
        let label = format!("value_noise_o{}.png", octaves);
        written += write_sample(&out_dir, &label, value_noise_rgb(DIM, octaves, 0xa5))?;
    }

    // 3. Voronoi cells (4) — increasing seed density gives smaller cells.
    for (i, seeds) in [16u32, 64, 256, 1024].iter().enumerate() {
        let label = format!("voronoi_n{:04}.png", seeds);
        written += write_sample(&out_dir, &label, voronoi(DIM, *seeds, 0xc1 + i as u64))?;
    }

    // 4. Normal-map-style (5) — partial derivatives of a value-noise
    // heightfield, packed R/G/B/A=255. These are the BC5 / NormalMap path.
    for octaves in 2..=6u32 {
        let label = format!("normalmap_o{}.png", octaves);
        written += write_sample(&out_dir, &label, normal_map(DIM, octaves, 0x77))?;
    }

    // 5. Pure high-frequency noise (4) — xorshift PRNG with different seeds.
    for (i, seed) in [
        0x1234_5678_9abc_def0u64,
        0xdead_beef_cafe_babeu64,
        0xface_b00c_0123_4567u64,
        0x9e37_79b9_7f4a_7c15u64,
    ]
    .iter()
    .enumerate()
    {
        let label = format!("highfreq_noise_{}.png", i);
        written += write_sample(&out_dir, &label, xorshift_noise(DIM, *seed))?;
    }

    // 6. Hard-edge content (4) — stripes, checker, sharp-edge text-ish.
    written += write_sample(&out_dir, "stripes_h.png", stripes(DIM, true, 32))?;
    written += write_sample(&out_dir, "stripes_v.png", stripes(DIM, false, 32))?;
    written += write_sample(&out_dir, "checker_64.png", checker(DIM, 64))?;
    written += write_sample(&out_dir, "edges_grid.png", edges_grid(DIM, 32))?;

    // 7. Constant-color (2) — the floor. Residual should be near zero.
    written += write_sample(&out_dir, "constant_grey.png", constant(DIM, [128, 128, 128, 255]))?;
    written += write_sample(&out_dir, "constant_red.png", constant(DIM, [200, 32, 32, 255]))?;

    println!("[gen_synthetic_corpus] wrote {} files to {}", written, out_dir.display());
    Ok(())
}

fn write_sample(dir: &Path, name: &str, rgba: Vec<u8>) -> Result<usize> {
    assert_eq!(rgba.len(), (DIM as usize) * (DIM as usize) * 4);
    let img: ImageBuffer<Rgba<u8>, _> =
        ImageBuffer::from_raw(DIM, DIM, rgba).context("ImageBuffer from raw rgba")?;
    let path = dir.join(name);
    img.save(&path).with_context(|| format!("save {}", path.display()))?;
    Ok(1)
}

// ---------- Smooth gradients ----------

fn smooth_radial(d: u32) -> Vec<u8> {
    let mut out = Vec::with_capacity((d * d * 4) as usize);
    let cx = d as f32 / 2.0;
    let cy = d as f32 / 2.0;
    let max = (cx * cx + cy * cy).sqrt();
    for y in 0..d {
        for x in 0..d {
            let dx = x as f32 - cx;
            let dy = y as f32 - cy;
            let r = (dx * dx + dy * dy).sqrt() / max;
            let v = (255.0 * (1.0 - r)).clamp(0.0, 255.0) as u8;
            out.extend_from_slice(&[v, (v / 2).saturating_add(64), 255 - v, 255]);
        }
    }
    out
}

fn smooth_linear(d: u32, vertical: bool) -> Vec<u8> {
    let mut out = Vec::with_capacity((d * d * 4) as usize);
    for y in 0..d {
        for x in 0..d {
            let v = if vertical {
                ((y * 256 / d) & 0xff) as u8
            } else {
                ((x * 256 / d) & 0xff) as u8
            };
            out.extend_from_slice(&[v, 255 - v, (v / 2).saturating_add(96), 255]);
        }
    }
    out
}

fn smooth_diagonal(d: u32) -> Vec<u8> {
    let mut out = Vec::with_capacity((d * d * 4) as usize);
    for y in 0..d {
        for x in 0..d {
            let v = (((x + y) * 256 / (2 * d)) & 0xff) as u8;
            out.extend_from_slice(&[v, v.wrapping_add(64), 255 - v, 255]);
        }
    }
    out
}

fn smooth_quadratic(d: u32) -> Vec<u8> {
    let mut out = Vec::with_capacity((d * d * 4) as usize);
    let scale = (d as f32) * (d as f32);
    for y in 0..d {
        for x in 0..d {
            let xn = x as f32 / d as f32;
            let yn = y as f32 / d as f32;
            let v = (255.0 * (xn * xn + yn * yn) / 2.0).clamp(0.0, 255.0) as u8;
            let _ = scale; // keep compiler happy
            out.extend_from_slice(&[v, 128, 255 - v, 255]);
        }
    }
    out
}

// ---------- Value noise (cheap Perlin-ish) ----------
//
// Generates a grid of random lattice values at 2^octave resolutions and
// interpolates bilinearly. Sum-of-octaves gives the textured look.

fn value_noise_rgb(d: u32, octaves: u32, seed: u8) -> Vec<u8> {
    let mut out = vec![0u8; (d * d * 4) as usize];
    let mut acc_r = vec![0f32; (d * d) as usize];
    let mut acc_g = vec![0f32; (d * d) as usize];
    let mut acc_b = vec![0f32; (d * d) as usize];
    let mut amp_sum = 0f32;
    for o in 0..octaves {
        let cells = 4u32 << o; // 4, 8, 16, 32...
        let amp = 0.5f32.powi(o as i32);
        amp_sum += amp;
        let mut rng = xorshift_state(0xb000 ^ (seed as u64) ^ (o as u64) * 0x9e37);
        // Three independent lattices for R/G/B so colour isn't grayscale.
        let lr = lattice(cells + 1, &mut rng);
        let lg = lattice(cells + 1, &mut rng);
        let lb = lattice(cells + 1, &mut rng);
        let step = d as f32 / cells as f32;
        for y in 0..d {
            for x in 0..d {
                let i = (y * d + x) as usize;
                let v_r = lattice_sample(&lr, cells, x as f32 / step, y as f32 / step);
                let v_g = lattice_sample(&lg, cells, x as f32 / step, y as f32 / step);
                let v_b = lattice_sample(&lb, cells, x as f32 / step, y as f32 / step);
                acc_r[i] += v_r * amp;
                acc_g[i] += v_g * amp;
                acc_b[i] += v_b * amp;
            }
        }
    }
    for i in 0..(d * d) as usize {
        let r = (acc_r[i] / amp_sum).clamp(0.0, 1.0);
        let g = (acc_g[i] / amp_sum).clamp(0.0, 1.0);
        let b = (acc_b[i] / amp_sum).clamp(0.0, 1.0);
        out[i * 4] = (r * 255.0) as u8;
        out[i * 4 + 1] = (g * 255.0) as u8;
        out[i * 4 + 2] = (b * 255.0) as u8;
        out[i * 4 + 3] = 255;
    }
    out
}

fn lattice(side: u32, rng: &mut u64) -> Vec<f32> {
    let mut v = Vec::with_capacity((side * side) as usize);
    for _ in 0..(side * side) {
        let r = xorshift_next(rng);
        v.push((r & 0xffff) as f32 / 65535.0);
    }
    v
}

fn lattice_sample(lat: &[f32], cells: u32, fx: f32, fy: f32) -> f32 {
    let side = cells + 1;
    let x0 = fx.floor() as u32;
    let y0 = fy.floor() as u32;
    let x1 = (x0 + 1).min(side - 1);
    let y1 = (y0 + 1).min(side - 1);
    let tx = fx - x0 as f32;
    let ty = fy - y0 as f32;
    let p00 = lat[(y0 * side + x0) as usize];
    let p10 = lat[(y0 * side + x1) as usize];
    let p01 = lat[(y1 * side + x0) as usize];
    let p11 = lat[(y1 * side + x1) as usize];
    // smoothstep for nicer interp
    let sx = tx * tx * (3.0 - 2.0 * tx);
    let sy = ty * ty * (3.0 - 2.0 * ty);
    let top = p00 * (1.0 - sx) + p10 * sx;
    let bot = p01 * (1.0 - sx) + p11 * sx;
    top * (1.0 - sy) + bot * sy
}

// ---------- Voronoi cells ----------

fn voronoi(d: u32, n_seeds: u32, seed: u64) -> Vec<u8> {
    let mut rng = xorshift_state(0x5eed ^ seed);
    let mut seeds: Vec<(f32, f32, [u8; 3])> = Vec::with_capacity(n_seeds as usize);
    for _ in 0..n_seeds {
        let x = (xorshift_next(&mut rng) % (d as u64)) as f32;
        let y = (xorshift_next(&mut rng) % (d as u64)) as f32;
        let r = (xorshift_next(&mut rng) & 0xff) as u8;
        let g = (xorshift_next(&mut rng) & 0xff) as u8;
        let b = (xorshift_next(&mut rng) & 0xff) as u8;
        seeds.push((x, y, [r, g, b]));
    }
    let mut out = vec![0u8; (d * d * 4) as usize];
    for y in 0..d {
        for x in 0..d {
            let (xf, yf) = (x as f32, y as f32);
            let mut best = f32::INFINITY;
            let mut color = [0u8, 0, 0];
            for s in &seeds {
                let dx = s.0 - xf;
                let dy = s.1 - yf;
                let d2 = dx * dx + dy * dy;
                if d2 < best {
                    best = d2;
                    color = s.2;
                }
            }
            let i = ((y * d + x) * 4) as usize;
            out[i] = color[0];
            out[i + 1] = color[1];
            out[i + 2] = color[2];
            out[i + 3] = 255;
        }
    }
    out
}

// ---------- Normal-map-style ----------
//
// Sample a value-noise heightfield then encode finite-difference normals
// into RGBA in the conventional UE / DirectX layout: R = dx, G = dy
// (remapped to [-1, 1] then into [0, 255]), B = up (mostly 255), A = 255.

fn normal_map(d: u32, octaves: u32, seed: u8) -> Vec<u8> {
    // Reuse value noise for the heightfield, then map to greyscale.
    let height_rgb = value_noise_rgb(d, octaves, seed);
    let mut height = vec![0f32; (d * d) as usize];
    for i in 0..(d * d) as usize {
        // Use just one channel as height — interesting structure, ignore the
        // others.
        height[i] = height_rgb[i * 4] as f32 / 255.0;
    }
    let mut out = vec![0u8; (d * d * 4) as usize];
    let strength = 4.0f32; // amplifies derivative so the normal map has visible texture
    for y in 0..d {
        for x in 0..d {
            let xm = if x == 0 { 0 } else { x - 1 } as usize;
            let xp = (x + 1).min(d - 1) as usize;
            let ym = if y == 0 { 0 } else { y - 1 } as usize;
            let yp = (y + 1).min(d - 1) as usize;
            let row = (y as usize) * (d as usize);
            let h_l = height[row + xm];
            let h_r = height[row + xp];
            let h_u = height[(ym) * (d as usize) + x as usize];
            let h_d = height[(yp) * (d as usize) + x as usize];
            // Sobel-ish: central diff.
            let dx = (h_r - h_l) * strength;
            let dy = (h_d - h_u) * strength;
            // Pack as standard normal map: R = (dx*0.5+0.5), G = (dy*0.5+0.5),
            // B = sqrt(max(0, 1 - dx^2 - dy^2))*0.5 + 0.5
            let len2 = dx * dx + dy * dy;
            let bz = (1.0 - len2.min(1.0)).max(0.0).sqrt();
            let r = ((dx * 0.5 + 0.5) * 255.0).clamp(0.0, 255.0) as u8;
            let g = ((dy * 0.5 + 0.5) * 255.0).clamp(0.0, 255.0) as u8;
            let b = ((bz * 0.5 + 0.5) * 255.0).clamp(0.0, 255.0) as u8;
            let i = ((y * d + x) * 4) as usize;
            out[i] = r;
            out[i + 1] = g;
            out[i + 2] = b;
            out[i + 3] = 255;
        }
    }
    out
}

// ---------- Pure noise ----------

fn xorshift_noise(d: u32, seed: u64) -> Vec<u8> {
    let mut rng = xorshift_state(seed);
    let mut out = vec![0u8; (d * d * 4) as usize];
    for i in 0..(d * d) as usize {
        let r = xorshift_next(&mut rng);
        out[i * 4] = (r & 0xff) as u8;
        out[i * 4 + 1] = ((r >> 8) & 0xff) as u8;
        out[i * 4 + 2] = ((r >> 16) & 0xff) as u8;
        out[i * 4 + 3] = 255;
    }
    out
}

// ---------- Hard-edge content ----------

fn stripes(d: u32, horizontal: bool, period: u32) -> Vec<u8> {
    let mut out = vec![0u8; (d * d * 4) as usize];
    for y in 0..d {
        for x in 0..d {
            let v = if horizontal {
                if (y / period) % 2 == 0 { 255u8 } else { 0u8 }
            } else if (x / period) % 2 == 0 {
                255u8
            } else {
                0u8
            };
            let i = ((y * d + x) * 4) as usize;
            out[i] = v;
            out[i + 1] = v;
            out[i + 2] = v;
            out[i + 3] = 255;
        }
    }
    out
}

fn checker(d: u32, period: u32) -> Vec<u8> {
    let mut out = vec![0u8; (d * d * 4) as usize];
    for y in 0..d {
        for x in 0..d {
            let on = ((x / period) + (y / period)) % 2 == 0;
            let v = if on { 240u8 } else { 16u8 };
            let i = ((y * d + x) * 4) as usize;
            out[i] = v;
            out[i + 1] = v.saturating_sub(20);
            out[i + 2] = v.saturating_sub(40);
            out[i + 3] = 255;
        }
    }
    out
}

fn edges_grid(d: u32, period: u32) -> Vec<u8> {
    // Sparse hard-line grid on a mid-grey background. The lines are the
    // codec stress test: very small high-frequency content the predictor
    // can't reproduce from the low mip.
    let mut out = vec![128u8; (d * d * 4) as usize];
    for i in 0..(d * d) as usize {
        out[i * 4 + 3] = 255;
    }
    for y in 0..d {
        for x in 0..d {
            if x % period == 0 || y % period == 0 {
                let i = ((y * d + x) * 4) as usize;
                out[i] = 16;
                out[i + 1] = 16;
                out[i + 2] = 16;
            }
        }
    }
    out
}

fn constant(d: u32, color: [u8; 4]) -> Vec<u8> {
    let mut out = Vec::with_capacity((d * d * 4) as usize);
    for _ in 0..(d * d) {
        out.extend_from_slice(&color);
    }
    out
}

// ---------- Tiny PRNG ----------

fn xorshift_state(seed: u64) -> u64 {
    // xorshift needs non-zero state.
    if seed == 0 { 0x9e37_79b9_7f4a_7c15 } else { seed }
}

fn xorshift_next(s: &mut u64) -> u64 {
    let mut x = *s;
    x ^= x << 13;
    x ^= x >> 7;
    x ^= x << 17;
    *s = x;
    x
}
