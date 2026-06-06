//! Cross-asset dedup prize sizer — how much redundancy lives *between* textures
//! that a per-texture codec (bilinear/ESRGAN/BC-residual) structurally cannot
//! see. The bcn_gate showed squeezing a single texture's residual caps at
//! ~+15% on BC7; this measures the orthogonal opportunity.
//!
//! Three model-free signals (all conservative — a real codec would do better):
//!   1. EXACT dups   — identical pixel buffers (content hash). Free to dedup.
//!   2. NEAR dups    — perceptual (average-)hash within a small Hamming radius.
//!                     Candidates for reference + delta coding.
//!   3. INTRA-MATERIAL cross-map redundancy — a material's Color/Normal/Rough/
//!      Metal maps share the same spatial structure. zstd(joint) vs
//!      sum(zstd(independent)) sizes the cross-map prediction prize directly.
//!
//! Caveat: measured in PIXEL space (the opportunity ceiling). On-disk BC hides
//! some of it — but cross-asset coding operates ABOVE the BC layer, which is
//! exactly why it sidesteps the non-linearity that capped bcn_gate.
//!
//! Run:
//!   cargo run -p shrinkray-core --release --example crossasset_gate -- \
//!     --corpus paper/corpus/pbr-cc0 --max-dim 512

use anyhow::{Context, Result};
use image::ImageReader;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::path::PathBuf;
use walkdir::WalkDir;

struct Tex {
    material: String,
    class: String,
    rgba: Vec<u8>,
    ahash: u64,
}

fn content_key(bytes: &[u8]) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    bytes.hash(&mut h);
    h.finish()
}

/// 8×8 average-hash over luminance → 64-bit perceptual fingerprint.
fn ahash(rgba: &[u8], w: u32, h: u32) -> u64 {
    let (gw, gh) = (8u32, 8u32);
    let mut cells = [0f64; 64];
    for cy in 0..gh {
        for cx in 0..gw {
            let x0 = cx * w / gw;
            let x1 = (((cx + 1) * w / gw).max(x0 + 1)).min(w);
            let y0 = cy * h / gh;
            let y1 = (((cy + 1) * h / gh).max(y0 + 1)).min(h);
            let (mut s, mut n) = (0f64, 0f64);
            for y in y0..y1 {
                for x in x0..x1 {
                    let i = ((y * w + x) * 4) as usize;
                    s += 0.299 * rgba[i] as f64 + 0.587 * rgba[i + 1] as f64 + 0.114 * rgba[i + 2] as f64;
                    n += 1.0;
                }
            }
            cells[(cy * gw + cx) as usize] = if n > 0.0 { s / n } else { 0.0 };
        }
    }
    let mean = cells.iter().sum::<f64>() / 64.0;
    let mut bits = 0u64;
    for (i, c) in cells.iter().enumerate() {
        if *c >= mean {
            bits |= 1 << i;
        }
    }
    bits
}

fn zc(data: &[u8]) -> usize {
    zstd::bulk::compress(data, 19).map(|v| v.len()).unwrap_or(data.len())
}

fn split_name(name: &str) -> (String, String) {
    let stem = name.strip_suffix(".png").unwrap_or(name);
    match stem.rsplit_once('_') {
        Some((mat, class)) => (mat.to_string(), class.to_string()),
        None => (stem.to_string(), "?".to_string()),
    }
}

fn mib(b: usize) -> f64 {
    b as f64 / (1024.0 * 1024.0)
}

fn main() -> Result<()> {
    let mut corpus = PathBuf::from("paper/corpus/pbr-cc0");
    let mut max_dim: u32 = 512;
    let mut near_radius: u32 = 6;
    let mut a = std::env::args().skip(1);
    while let Some(f) = a.next() {
        match f.as_str() {
            "--corpus" => corpus = PathBuf::from(a.next().context("--corpus path")?),
            "--max-dim" => max_dim = a.next().context("--max-dim N")?.parse()?,
            "--near-radius" => near_radius = a.next().context("--near-radius N")?.parse()?,
            other => eprintln!("[crossasset_gate] ignoring {other}"),
        }
    }

    let mut texs: Vec<Tex> = Vec::new();
    for e in WalkDir::new(&corpus).into_iter().filter_map(|e| e.ok()) {
        let p = e.path();
        if !p.is_file() {
            continue;
        }
        if !matches!(
            p.extension().and_then(|s| s.to_str()).unwrap_or(""),
            "png" | "jpg" | "jpeg" | "webp"
        ) {
            continue;
        }
        let img = ImageReader::open(p)?
            .with_guessed_format()?
            .decode()
            .with_context(|| format!("decode {}", p.display()))?
            .to_rgba8();
        let (mut w, mut h) = (img.width(), img.height());
        let mut rgba = img.into_raw();
        if w > max_dim || h > max_dim {
            let scale = (max_dim as f32 / w.max(h) as f32).min(1.0);
            let nw = ((w as f32 * scale).round() as u32).max(8);
            let nh = ((h as f32 * scale).round() as u32).max(8);
            let src = image::RgbaImage::from_raw(w, h, rgba).context("rebuild rgba")?;
            let r = image::imageops::resize(&src, nw, nh, image::imageops::FilterType::Lanczos3);
            w = nw;
            h = nh;
            rgba = r.into_raw();
        }
        let name = p.file_name().unwrap().to_string_lossy().into_owned();
        let (material, class) = split_name(&name);
        let ah = ahash(&rgba, w, h);
        texs.push(Tex { material, class, rgba, ahash: ah });
    }
    if texs.is_empty() {
        anyhow::bail!("no images under {}", corpus.display());
    }

    let n = texs.len();
    let total_rgba: usize = texs.iter().map(|t| t.rgba.len()).sum();
    let total_indep_zstd: usize = texs.iter().map(|t| zc(&t.rgba)).sum();

    // 1. EXACT dups.
    let mut by_content: HashMap<u64, usize> = HashMap::new();
    for t in &texs {
        *by_content.entry(content_key(&t.rgba)).or_insert(0) += 1;
    }
    let exact_dup_copies: usize = by_content.values().map(|c| c - 1).sum();
    let exact_dup_groups = by_content.values().filter(|c| **c > 1).count();

    // 2. NEAR dups (perceptual hash within Hamming radius), excluding exact.
    let mut near_pairs = 0usize;
    for i in 0..n {
        for j in (i + 1)..n {
            let d = (texs[i].ahash ^ texs[j].ahash).count_ones();
            if d > 0 && d <= near_radius {
                near_pairs += 1;
            }
        }
    }

    // 3. INTRA-MATERIAL cross-map redundancy.
    let mut groups: HashMap<String, Vec<usize>> = HashMap::new();
    for (i, t) in texs.iter().enumerate() {
        groups.entry(t.material.clone()).or_default().push(i);
    }
    let mut indep_total = 0usize;
    let mut joint_total = 0usize;
    let mut multimap_groups = 0usize;
    for idxs in groups.values() {
        if idxs.len() < 2 {
            continue;
        }
        multimap_groups += 1;
        let indep: usize = idxs.iter().map(|&i| zc(&texs[i].rgba)).sum();
        let mut concat = Vec::new();
        for &i in idxs {
            concat.extend_from_slice(&texs[i].rgba);
        }
        let joint = zc(&concat);
        indep_total += indep;
        joint_total += joint;
    }
    let crossmap_prize = indep_total.saturating_sub(joint_total);
    let crossmap_pct = if indep_total > 0 {
        crossmap_prize as f64 / indep_total as f64 * 100.0
    } else {
        0.0
    };

    println!("\n=== CROSS-ASSET DEDUP PRIZE — {} ({n} textures) ===", corpus.display());
    println!("total pixels       : {:.2} MiB RGBA", mib(total_rgba));
    println!("independently zstd : {:.2} MiB  (per-texture baseline)", mib(total_indep_zstd));
    println!();
    println!("1. EXACT dups   : {exact_dup_groups} group(s), {exact_dup_copies} redundant copies");
    println!("2. NEAR dups    : {near_pairs} pair(s) within Hamming {near_radius} (reference+delta candidates)");
    println!("3. INTRA-MATERIAL cross-map redundancy ({multimap_groups} multi-map materials):");
    println!("   independent zstd : {:.2} MiB", mib(indep_total));
    println!("   joint zstd       : {:.2} MiB", mib(joint_total));
    println!("   CROSS-MAP PRIZE  : {:.2} MiB  ({crossmap_pct:.1}% of independent)", mib(crossmap_prize));
    println!();
    println!("read: exact/near dups size the INTER-texture prize (whole-texture dedup);");
    println!("cross-map prize sizes the INTRA-material prize (predict a map from its siblings).");
    println!("Both are invisible to per-texture BC-residual coding — that's the point.\n");
    Ok(())
}
