//! Router gate — warmcore Phase-1 discipline applied to the Δ-Codec router.
//!
//! BEFORE training any router, prove there is measurable headroom in the
//! decision it would make. The router's job is NOT predictor-selection (the
//! G13 probe already showed ~no headroom there: 5.3% oracle lift). Its job is
//! the high-contrast **codec-vs-raw-backup** decision: use the codec when the
//! residual sidecar is smaller than a full backup (sidecar < 1.0×), otherwise
//! just back up the raw top mip (cost 1.0×). That caps the downside — the
//! noise case (sidecar 1.07×) proved the codec can LOSE to a backup.
//!
//! This harness measures, per corpus:
//!   1. sidecar cost (bilinear q=1) per texture + the cheap high-pass feature,
//!   2. how often backup would win (the decision actually flips),
//!   3. disk a perfect router captures vs always-codec and vs always-backup,
//!   4. whether a single high-pass-energy threshold reproduces the oracle route,
//!   5. per-class sidecar means (feeds the predictor-zoo question).
//!
//! Run:
//!   cargo run -p shrinkray-core --release --features inference \
//!     --example router_gate -- --corpus paper/corpus/pbr-cc0 --downsample 4

use anyhow::{Context, Result};
use image::ImageReader;
use shrinkray_delta_codec::{
    box_downsample_2x, encode_texture, probe_codec_space, BilinearPredictor,
};
use std::path::PathBuf;
use walkdir::WalkDir;

struct Sample {
    class: String,
    sidecar: f64,
    hp_energy: f64,
}

/// AmbientCG naming: `Material###_Class.png` → trailing token is the class.
fn class_of(name: &str) -> String {
    let stem = name.strip_suffix(".png").unwrap_or(name);
    // strip a trailing resolution token like "1K"/"2K" if present
    let tok = stem.rsplit('_').next().unwrap_or("?");
    tok.to_string()
}

fn crop_to_multiple(rgba: &[u8], w: u32, h: u32, m: u32) -> (Vec<u8>, u32, u32) {
    let nw = w - (w % m);
    let nh = h - (h % m);
    if nw == w && nh == h {
        return (rgba.to_vec(), w, h);
    }
    let mut out = Vec::with_capacity((nw * nh * 4) as usize);
    for y in 0..nh {
        let r = (y * w * 4) as usize;
        out.extend_from_slice(&rgba[r..r + (nw * 4) as usize]);
    }
    (out, nw, nh)
}

fn downsample(rgba: &[u8], w: u32, h: u32, factor: u8) -> Result<(Vec<u8>, u32, u32)> {
    let steps = if factor == 4 { 2 } else { 1 };
    let mut cur = (rgba.to_vec(), w, h);
    for _ in 0..steps {
        let (d, dw, dh) = box_downsample_2x(&cur.0, cur.1, cur.2)?;
        cur = (d, dw, dh);
    }
    Ok(cur)
}

fn main() -> Result<()> {
    let mut corpus = PathBuf::from("paper/corpus/pbr-cc0");
    let mut ds: u8 = 4;
    let mut max_dim: u32 = 512;
    let mut a = std::env::args().skip(1);
    while let Some(f) = a.next() {
        match f.as_str() {
            "--corpus" => corpus = PathBuf::from(a.next().context("--corpus needs a path")?),
            "--downsample" => ds = a.next().context("--downsample needs 2|4")?.parse()?,
            "--max-dim" => max_dim = a.next().context("--max-dim needs N")?.parse()?,
            other => eprintln!("[router_gate] ignoring arg {other}"),
        }
    }

    let mut samples: Vec<Sample> = Vec::new();
    for e in WalkDir::new(&corpus).into_iter().filter_map(|e| e.ok()) {
        let p = e.path();
        if !p.is_file() {
            continue;
        }
        let ext = p.extension().and_then(|s| s.to_str()).unwrap_or("");
        if !matches!(ext, "png" | "jpg" | "jpeg" | "webp") {
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
            let nw = ((w as f32 * scale).round() as u32).max(ds as u32);
            let nh = ((h as f32 * scale).round() as u32).max(ds as u32);
            let src = image::RgbaImage::from_raw(w, h, rgba).context("rebuild rgba")?;
            let resized = image::imageops::resize(&src, nw, nh, image::imageops::FilterType::Lanczos3);
            w = nw;
            h = nh;
            rgba = resized.into_raw();
        }
        let (top, w, h) = crop_to_multiple(&rgba, w, h, ds as u32);
        let (low, lw, lh) = downsample(&top, w, h, ds)?;
        let mut pred = BilinearPredictor;
        let bs = encode_texture(&mut pred, &top, w, h, low, lw, lh, 1, true)?;
        let sidecar = bs.size().residual_zst_bytes as f64 / top.len().max(1) as f64;
        let hp = probe_codec_space(&top, w, h).high_pass_energy;
        let name = p.file_name().unwrap().to_string_lossy().into_owned();
        samples.push(Sample {
            class: class_of(&name),
            sidecar,
            hp_energy: hp,
        });
    }

    if samples.is_empty() {
        anyhow::bail!("no images found under {}", corpus.display());
    }

    let n = samples.len();
    let backup_wins = samples.iter().filter(|s| s.sidecar >= 1.0).count();
    let codec_wins = n - backup_wins;

    // Per-texture costs (equal-weighted means; backup baseline = 1.0×).
    let mean = |f: &dyn Fn(&Sample) -> f64| samples.iter().map(|s| f(s)).sum::<f64>() / n as f64;
    let always_codec = mean(&|s| s.sidecar);
    let oracle = mean(&|s| s.sidecar.min(1.0));
    let always_backup = 1.0_f64;

    println!("\n=== ROUTER GATE — {} ({} textures, {}× downsample) ===", corpus.display(), n, ds);
    println!("codec wins (sidecar<1.0): {codec_wins}   backup wins (sidecar>=1.0): {backup_wins}");
    println!("mean per-texture cost (lower=better):");
    println!("  always-backup : {always_backup:.3}×  (the safe baseline)");
    println!("  always-codec  : {always_codec:.3}×  (no router; can overshoot 1.0 on hostile content)");
    println!("  perfect router: {oracle:.3}×  (min(codec, backup) per texture)");
    let head_vs_codec = always_codec - oracle;
    let value_vs_backup = always_backup - oracle;
    println!("router headroom vs always-codec : {head_vs_codec:.4}  ({})",
        if head_vs_codec > 0.005 { "REAL — flips happen" } else { "~none on this corpus" });
    println!("codec value vs always-backup    : {value_vs_backup:.4}  (why use the codec at all)");

    // Separability: can one high-pass threshold reproduce the codec/backup route?
    if backup_wins == 0 {
        println!("\nGATE VERDICT: codec never loses to backup on this corpus → NO routing decision to make here.");
        println!("The router earns its keep only on corpora containing incompressible textures.");
        println!("Re-run on a mixed/noise corpus to exercise the decision.");
    } else {
        let mut best_t = 0.0;
        let mut best_acc = 0.0;
        let lo = samples.iter().map(|s| s.hp_energy).fold(f64::INFINITY, f64::min);
        let hi = samples.iter().map(|s| s.hp_energy).fold(f64::NEG_INFINITY, f64::max);
        let steps = 200;
        for i in 0..=steps {
            let t = lo + (hi - lo) * (i as f64 / steps as f64);
            // rule: hp_energy >= t → predict backup
            let correct = samples
                .iter()
                .filter(|s| (s.hp_energy >= t) == (s.sidecar >= 1.0))
                .count();
            let acc = correct as f64 / n as f64;
            if acc > best_acc {
                best_acc = acc;
                best_t = t;
            }
        }
        let majority = (codec_wins.max(backup_wins)) as f64 / n as f64;
        println!("\nseparability (high-pass-energy threshold → codec/backup):");
        println!("  best threshold : hp >= {best_t:.2} ⇒ backup");
        println!("  accuracy       : {:.1}%   (majority baseline {:.1}%)", best_acc * 100.0, majority * 100.0);
        println!("GATE VERDICT: {}",
            if best_acc > majority + 0.02 {
                "PASS — a cheap threshold beats the constant predictor; distilling a router is justified."
            } else {
                "FAIL — the cheap feature does not beat always-guessing; do not ship a learned router."
            });
    }

    // Per-class sidecar means — feeds the predictor-zoo question (where's the headroom?).
    let mut classes: Vec<&String> = samples.iter().map(|s| &s.class).collect();
    classes.sort();
    classes.dedup();
    println!("\nper-class mean sidecar (q=1 bilinear; higher = more residual = more room for a specialist):");
    for c in classes {
        let cs: Vec<&Sample> = samples.iter().filter(|s| &s.class == c).collect();
        let m = cs.iter().map(|s| s.sidecar).sum::<f64>() / cs.len() as f64;
        println!("  {:<12} n={:<3} mean sidecar {:.3}×", c, cs.len(), m);
    }
    println!();
    Ok(())
}
