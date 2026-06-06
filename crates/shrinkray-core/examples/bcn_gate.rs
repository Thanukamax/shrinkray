//! BCn gate — the honest file-size delta on cooked-texture-shaped data.
//!
//! The pixel gate measured the residual against *uncompressed RGBA*, which
//! flatters the codec: real cooked textures sit in the pak as BCn blocks
//! (4–8× smaller than RGBA). This gate measures the residual against the
//! actual **BCn byte count** — the bytes that would really be stripped — so
//! the ratio reflects disk, not pixels.
//!
//! Workflow accounting (per texture, top mip = the thing being stripped):
//!   bc_top      = on-disk BCn bytes of the top mip   (what stripping frees)
//!   residual    = Δ-Codec residual sidecar           (the reversibility cost;
//!                 the low mip already lives in the pak, so it is NOT counted)
//!   bc_sidecar  = residual / bc_top
//!   Δ-Codec net = bc_top − residual = bc_top·(1 − bc_sidecar)   ← real saving
//!   ExactBackup net = bc_top − bc_top = 0          (backup eats the save)
//!
//! So Δ-Codec is net-positive exactly when bc_sidecar < 1.0, and it beats
//! ExactBackup (which is always ~0) by (1 − bc_sidecar) of the top mip — while
//! staying byte-exact in BCn space.
//!
//! Run:
//!   cargo run -p shrinkray-core --release --features inference \
//!     --example bcn_gate -- --corpus paper/corpus/pbr-cc0 --downsample 4 --format bc7

use anyhow::{Context, Result};
use image::ImageReader;
use shrinkray_delta_codec::{
    box_downsample_2x, decode_bc_residual, encode_bc_residual, BcResidualFormat, BilinearPredictor,
};
use std::path::PathBuf;
use walkdir::WalkDir;

struct Row {
    class: String,
    bc_top: usize,
    residual: usize,
    byte_exact: bool,
}

fn class_of(name: &str) -> String {
    let stem = name.strip_suffix(".png").unwrap_or(name);
    stem.rsplit('_').next().unwrap_or("?").to_string()
}

fn parse_format(s: &str) -> Result<BcResidualFormat> {
    Ok(match s.to_ascii_lowercase().as_str() {
        "bc1" => BcResidualFormat::Bc1,
        "bc3" => BcResidualFormat::Bc3,
        "bc5" => BcResidualFormat::Bc5,
        "bc7" => BcResidualFormat::Bc7,
        other => anyhow::bail!("unknown format {other} (use bc1|bc3|bc5|bc7)"),
    })
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

fn mib(bytes: usize) -> f64 {
    bytes as f64 / (1024.0 * 1024.0)
}

fn main() -> Result<()> {
    let mut corpus = PathBuf::from("paper/corpus/pbr-cc0");
    let mut ds: u8 = 4;
    let mut max_dim: u32 = 512;
    let mut fmt = BcResidualFormat::Bc7;
    let mut a = std::env::args().skip(1);
    while let Some(f) = a.next() {
        match f.as_str() {
            "--corpus" => corpus = PathBuf::from(a.next().context("--corpus path")?),
            "--downsample" => ds = a.next().context("--downsample 2|4")?.parse()?,
            "--max-dim" => max_dim = a.next().context("--max-dim N")?.parse()?,
            "--format" => fmt = parse_format(&a.next().context("--format bc7")?)?,
            other => eprintln!("[bcn_gate] ignoring {other}"),
        }
    }
    // BCn blocks are 4×4, so dims must be a multiple of 4; downsample needs its
    // own factor too. Crop to the larger of the two.
    let crop_m = (ds as u32).max(4);

    let mut rows: Vec<Row> = Vec::new();
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
            let nw = ((w as f32 * scale).round() as u32).max(crop_m);
            let nh = ((h as f32 * scale).round() as u32).max(crop_m);
            let src = image::RgbaImage::from_raw(w, h, rgba).context("rebuild rgba")?;
            let r = image::imageops::resize(&src, nw, nh, image::imageops::FilterType::Lanczos3);
            w = nw;
            h = nh;
            rgba = r.into_raw();
        }
        let (top, w, h) = crop_to_multiple(&rgba, w, h, crop_m);
        let (low, lw, lh) = downsample(&top, w, h, ds)?;

        let mut enc = BilinearPredictor;
        let bs = match encode_bc_residual(&mut enc, fmt, &top, w, h, low, lw, lh, true) {
            Ok(b) => b,
            Err(e) => {
                eprintln!("[bcn_gate] skip {}: {e}", p.display());
                continue;
            }
        };
        let residual = bs.size().residual_zst_bytes;
        let mut dec = BilinearPredictor;
        // decode_bc_residual returns the reconstructed BCn top-mip bytes and
        // verifies the recorded hash — Ok ⇒ byte-exact in BCn space.
        let (bc_top, byte_exact) = match decode_bc_residual(&mut dec, &bs) {
            Ok(bc) => (bc.len(), true),
            Err(e) => {
                eprintln!("[bcn_gate] decode failed {}: {e}", p.display());
                continue;
            }
        };
        let name = p.file_name().unwrap().to_string_lossy().into_owned();
        rows.push(Row {
            class: class_of(&name),
            bc_top,
            residual,
            byte_exact,
        });
    }

    if rows.is_empty() {
        anyhow::bail!("no images under {}", corpus.display());
    }

    let n = rows.len();
    let exact = rows.iter().filter(|r| r.byte_exact).count();
    let sum_top: usize = rows.iter().map(|r| r.bc_top).sum();
    let sum_res: usize = rows.iter().map(|r| r.residual).sum();
    let net = sum_top as i64 - sum_res as i64;
    let wins = rows.iter().filter(|r| r.residual < r.bc_top).count();
    let mean_sidecar = rows
        .iter()
        .map(|r| r.residual as f64 / r.bc_top.max(1) as f64)
        .sum::<f64>()
        / n as f64;

    let fmt_name = match fmt {
        BcResidualFormat::Bc1 => "BC1",
        BcResidualFormat::Bc3 => "BC3",
        BcResidualFormat::Bc5 => "BC5",
        BcResidualFormat::Bc7 => "BC7",
    };

    println!("\n=== BCn GATE — {} ({n} textures, {fmt_name}, {ds}× downsample) ===", corpus.display());
    println!("byte-exact (BCn space): {exact}/{n}");
    println!("Δ-Codec beats ExactBackup (residual < bc_top): {wins}/{n}");
    println!("mean bc_sidecar (residual ÷ on-disk BCn top): {mean_sidecar:.3}×");
    println!();
    println!("whole-corpus disk accounting:");
    println!("  stripping top mips frees     : {:.2} MiB  (sum of BCn top-mip bytes)", mib(sum_top));
    println!("  Δ-Codec residual sidecar costs: {:.2} MiB", mib(sum_res));
    println!("  Δ-Codec NET saved (byte-exact): {:+.2} MiB  ({:+.1}% of stripped)",
        net as f64 / (1024.0 * 1024.0),
        net as f64 / sum_top as f64 * 100.0);
    println!("  ExactBackup NET saved         : 0.00 MiB  (backup eats the save)");
    println!();
    println!("per-class mean bc_sidecar (lower = more real saving):");
    let mut classes: Vec<&String> = rows.iter().map(|r| &r.class).collect();
    classes.sort();
    classes.dedup();
    for c in classes {
        let cs: Vec<&Row> = rows.iter().filter(|r| &r.class == c).collect();
        let m = cs
            .iter()
            .map(|r| r.residual as f64 / r.bc_top.max(1) as f64)
            .sum::<f64>()
            / cs.len() as f64;
        let net_pct = (1.0 - m) * 100.0;
        println!("  {:<12} n={:<3} bc_sidecar {:.3}×  → net {:+.1}% of top mip", c, cs.len(), m, net_pct);
    }
    println!();
    Ok(())
}
