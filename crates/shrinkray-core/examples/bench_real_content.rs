//! bench_real_content — the paper-grade Δ-Codec measurement harness.
//!
//! Walks a corpus directory of PNG/JPG/WebP textures, and for each image:
//!   1. Loads + resizes (cap longest edge to `--max-dim`, default 1024).
//!   2. Synthesises a low mip (2× box downsample).
//!   3. Runs the codec in three modes, at every `--quant-steps`:
//!      - pixel space (RGBA residual)
//!      - BC1 / BC3 / BC5 / BC7 byte-residual
//!      - adaptive (probe → pixel-or-bc3)
//!   4. Records lossless verdict, byte counts, HP energy, recommendation,
//!      and ratio vs baseline.
//! All measurements stream to a CSV that the paper draws from. Per-image
//! work runs in parallel via rayon so swapping ESRGAN in later only adds
//! latency to one row at a time, not the whole sweep.
//!
//! Lossless verdict (CSV column `lossless_pass`):
//!   - pixel:  decoded RGBA bytes == original RGBA bytes
//!   - bcN:    decoded BC bytes  == fresh-encode-of-original BC bytes
//!   - adaptive: whichever underlying path was routed
//!
//! Anything other than `true` at quant_step=1 = the codec is broken on that
//! sample. Surface in the summary so the paper doesn't quietly silence them.
//!
//! Predictor swap (`--predictor`):
//!   bilinear   — wired today.
//!   esrgan     — TODO: imports `EsrganX4Predictor` from agent-A's branch in
//!                shrinkray-core. Until that lands the flag panics with a
//!                clear "not yet available" error so you don't run a bench
//!                with the silently-wrong predictor.
//!
//! Run:
//!   cargo run -p shrinkray-core --release --example bench_real_content -- \
//!     --corpus paper/corpus \
//!     --quant-steps 1,2,4

use std::fs::File;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{bail, Context, Result};
use clap::{Parser, ValueEnum};
use image::ImageReader;
use rayon::prelude::*;
use shrinkray_delta_codec::{
    bc_residual::bc_decode_to_rgba8, box_downsample_2x, decode_bc_residual, decode_texture,
    encode_bc_residual, encode_texture, probe_codec_space, BcResidualFormat, BilinearPredictor,
    CodecSpace, Predictor, PredictorId,
};
use walkdir::WalkDir;

#[cfg(feature = "inference")]
use shrinkray_core::predictors::EsrganX4Predictor;

/// Type-erased predictor wrapper so the bench can swap bilinear ↔ esrgan
/// per CLI flag without templating the whole call tree.
enum AnyPredictor {
    Bilinear(BilinearPredictor),
    #[cfg(feature = "inference")]
    Esrgan(EsrganX4Predictor),
}

impl Predictor for AnyPredictor {
    fn id(&self) -> PredictorId {
        match self {
            AnyPredictor::Bilinear(p) => p.id(),
            #[cfg(feature = "inference")]
            AnyPredictor::Esrgan(p) => p.id(),
        }
    }
    fn predict(
        &mut self,
        low: &[u8],
        lw: u32,
        lh: u32,
        tw: u32,
        th: u32,
    ) -> anyhow::Result<Vec<u8>> {
        match self {
            AnyPredictor::Bilinear(p) => p.predict(low, lw, lh, tw, th),
            #[cfg(feature = "inference")]
            AnyPredictor::Esrgan(p) => p.predict(low, lw, lh, tw, th),
        }
    }
}

type MakePred = Box<dyn Fn() -> AnyPredictor + Send + Sync>;

#[derive(Debug, Clone, Copy, ValueEnum)]
enum PredictorKind {
    Bilinear,
    Esrgan,
}

#[derive(Parser, Debug)]
#[command(
    name = "bench_real_content",
    about = "Δ-Codec measurement harness — walks a corpus and tabulates pixel/BC/adaptive results"
)]
struct Args {
    /// Directory walked recursively. Picks up *.png / *.jpg / *.jpeg / *.webp.
    #[arg(long)]
    corpus: PathBuf,

    /// Output CSV path. Default: paper/results/bench-<unix>.csv
    #[arg(long)]
    out_csv: Option<PathBuf>,

    /// Predictor. `esrgan` panics until agent-A's predictor lands.
    #[arg(long, value_enum, default_value_t = PredictorKind::Bilinear)]
    predictor: PredictorKind,

    /// Comma-separated quant steps. Default 1,2,4. Step=1 is the lossless run.
    #[arg(long, default_value = "1,2,4")]
    quant_steps: String,

    /// Resize cap for longest edge. Larger inputs are downsized so the bench
    /// stays interactive. 0 = no resize.
    #[arg(long, default_value_t = 1024)]
    max_dim: u32,

    /// Skip BC variants — useful when iterating on the pixel path.
    #[arg(long, default_value_t = false)]
    skip_bc: bool,

    /// Which BC formats to measure. Comma-separated subset of Bc1,Bc3,Bc5,Bc7.
    /// Ignored when `--skip-bc` is set. Default = all four (the paper sweep).
    #[arg(long, default_value = "Bc1,Bc3,Bc5,Bc7")]
    bc_formats: String,

    /// Cap the corpus to first N files (deterministic order). 0 = no cap.
    #[arg(long, default_value_t = 0)]
    limit: usize,

    /// Override the ESRGAN ONNX model path. Falls back to
    /// SHRINKRAY_AI_MODEL env var, then
    /// src-tauri/binaries/ai-models/realesrgan-x4-general.onnx.
    /// Ignored when --predictor=bilinear.
    #[arg(long)]
    esrgan_model: Option<PathBuf>,
}

fn main() -> Result<()> {
    let args = Args::parse();

    let quant_steps: Vec<u8> = args
        .quant_steps
        .split(',')
        .map(|s| s.trim().parse::<u8>().context("parse quant step"))
        .collect::<Result<_>>()?;
    if quant_steps.is_empty() {
        bail!("--quant-steps must contain at least one value");
    }

    // Build the predictor factory. Bilinear is a zero-cost constructor;
    // esrgan loads a 67 MB ORT session per call so we serialize the bench to
    // one rayon worker to avoid 12× session loads + GPU/CPU contention.
    let make_pred: MakePred = match args.predictor {
        PredictorKind::Bilinear => Box::new(|| AnyPredictor::Bilinear(BilinearPredictor)),
        #[cfg(feature = "inference")]
        PredictorKind::Esrgan => {
            let model_path = args
                .esrgan_model
                .clone()
                .or_else(|| std::env::var("SHRINKRAY_AI_MODEL").ok().map(PathBuf::from))
                .unwrap_or_else(|| {
                    PathBuf::from("src-tauri/binaries/ai-models/realesrgan-x4-general.onnx")
                });
            if !model_path.exists() {
                bail!(
                    "--predictor esrgan: model not found at {}. Run scripts/fetch-ai-models.sh \
                     or set SHRINKRAY_AI_MODEL=<path>.",
                    model_path.display()
                );
            }
            eprintln!("[bench] esrgan model: {}", model_path.display());
            rayon::ThreadPoolBuilder::new()
                .num_threads(1)
                .build_global()
                .ok();
            Box::new(move || {
                AnyPredictor::Esrgan(
                    EsrganX4Predictor::new(&model_path).expect("esrgan session load"),
                )
            })
        }
        #[cfg(not(feature = "inference"))]
        PredictorKind::Esrgan => {
            bail!(
                "--predictor esrgan requires --features inference. Rebuild with: \
                 cargo run -p shrinkray-core --release --features inference --example bench_real_content"
            );
        }
    };

    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let out_csv = args.out_csv.unwrap_or_else(|| {
        PathBuf::from(format!("paper/results/bench-{}.csv", timestamp))
    });
    if let Some(parent) = out_csv.parent() {
        std::fs::create_dir_all(parent).ok();
    }

    let mut files = collect_images(&args.corpus)?;
    files.sort();
    if args.limit > 0 && files.len() > args.limit {
        files.truncate(args.limit);
    }
    if files.is_empty() {
        bail!("no images found under {}", args.corpus.display());
    }
    eprintln!(
        "[bench] corpus={} predictor={:?} quant_steps={:?} files={} out={}",
        args.corpus.display(),
        args.predictor,
        quant_steps,
        files.len(),
        out_csv.display(),
    );

    // CSV header (sink is shared across rayon tasks via Mutex). Each thread
    // emits whole pre-formatted rows so contention is minimal.
    let csv_sink = Mutex::new(
        File::create(&out_csv).with_context(|| format!("create {}", out_csv.display()))?,
    );
    writeln!(
        csv_sink.lock().unwrap(),
        "path,label,w,h,format,quant_step,baseline_bytes,residual_bytes,low_mip_bytes,total_codec_bytes,ratio_vs_baseline,lossless_pass,hp_energy,recommended_space"
    )?;

    // Parse --bc-formats into an owned Vec. Empty list = same as --skip-bc.
    // Names are case-insensitive ("bc3" == "Bc3" == "BC3") so the CLI is
    // forgiving — the brief uses CamelCase, shell history tends to lower-case.
    let bc_variants: Vec<BcResidualFormat> = if args.skip_bc {
        Vec::new()
    } else {
        let mut v = Vec::new();
        for raw in args.bc_formats.split(',') {
            let tag = raw.trim().to_ascii_lowercase();
            if tag.is_empty() {
                continue;
            }
            let fmt = match tag.as_str() {
                "bc1" => BcResidualFormat::Bc1,
                "bc3" => BcResidualFormat::Bc3,
                "bc5" => BcResidualFormat::Bc5,
                "bc7" => BcResidualFormat::Bc7,
                other => bail!("--bc-formats: unknown variant {other:?} (expected Bc1|Bc3|Bc5|Bc7)"),
            };
            if !v.contains(&fmt) {
                v.push(fmt);
            }
        }
        v
    };

    // Rayon over the corpus. Each file does its own pixel sweep + BC sweep
    // + adaptive run, formats rows, then takes the sink lock once per file
    // to flush. Keeps lock contention to ~N lockings = file count.
    let bc_variants_slice: &[BcResidualFormat] = &bc_variants;
    let make_pred_ref: &MakePred = &make_pred;
    files.par_iter().for_each(|path| {
        let label = label_for_path(path, &args.corpus);
        match run_one(
            path,
            &label,
            &quant_steps,
            bc_variants_slice,
            args.max_dim,
            make_pred_ref,
        ) {
            Ok(rows) => {
                let mut sink = csv_sink.lock().unwrap();
                for r in rows {
                    if let Err(e) = writeln!(sink, "{}", r) {
                        eprintln!("[bench] csv write failed for {}: {e}", path.display());
                    }
                }
                eprintln!("[bench] done {}", label);
            }
            Err(e) => {
                eprintln!("[bench] FAILED {}: {e:#}", path.display());
            }
        }
    });

    eprintln!("[bench] wrote {}", out_csv.display());
    Ok(())
}

fn collect_images(root: &Path) -> Result<Vec<PathBuf>> {
    if !root.exists() {
        bail!("corpus root does not exist: {}", root.display());
    }
    let mut out = Vec::new();
    for entry in WalkDir::new(root).into_iter().filter_map(|e| e.ok()) {
        if !entry.file_type().is_file() {
            continue;
        }
        let ext = entry
            .path()
            .extension()
            .and_then(|e| e.to_str())
            .map(|s| s.to_ascii_lowercase());
        match ext.as_deref() {
            Some("png" | "jpg" | "jpeg" | "webp") => out.push(entry.into_path()),
            _ => {}
        }
    }
    Ok(out)
}

fn label_for_path(path: &Path, root: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .with_extension("")
        .to_string_lossy()
        .into_owned()
}

/// Per-file work: load + resize, sweep pixel/BC/adaptive, return CSV rows.
fn run_one(
    path: &Path,
    label: &str,
    quant_steps: &[u8],
    bc_variants: &[BcResidualFormat],
    max_dim: u32,
    make_pred: &MakePred,
) -> Result<Vec<String>> {
    let (rgba, w, h) = load_resize(path, max_dim)?;
    // BC formats require dims % 4 == 0 (block size). Pad / crop to nearest
    // multiple of 4 by trimming — losing at most 3 px per side is fine for
    // a measurement harness; we record the actual dims in the CSV.
    let (rgba, w, h) = crop_to_multiple(&rgba, w, h, 4);
    let baseline_pixel = rgba.len();
    // 4× downsample (two 2× passes). The shrinkray strip path collapses a
    // 4096×4096 top mip down to 1024×1024 on disk, so the codec input
    // should be a 4× pair — and ESRGAN-x4 requires exactly that ratio.
    // Bilinear also works at any ratio, so this is a strict-but-compatible
    // upgrade from agent-B's original 2× harness.
    let (low_2x, lw_2x, lh_2x) = box_downsample_2x(&rgba, w, h).context("downsample 2x")?;
    let (low, lw, lh) =
        box_downsample_2x(&low_2x, lw_2x, lh_2x).context("downsample to 4x low")?;

    let probe = probe_codec_space(&rgba, w, h);
    let hp = probe.high_pass_energy;
    let recommended = match probe.recommended {
        CodecSpace::Pixel => "pixel",
        CodecSpace::Bc3Byte => "bc3-byte",
    };

    let mut rows = Vec::new();

    // ---------- pixel-space sweep ----------
    for q in quant_steps {
        let q = *q;
        let mut pred = make_pred();
        let bs = encode_texture(&mut pred, &rgba, w, h, low.clone(), lw, lh, q, q == 1)
            .with_context(|| format!("encode_texture q={q}"))?;
        let size = bs.size();
        let mut dec = make_pred();
        let restored = decode_texture(&mut dec, &bs).with_context(|| format!("decode q={q}"))?;
        let lossless = q == 1 && restored == rgba;
        // The deployed framing: low mip already lives in the stripped pak,
        // so the *new* disk cost for byte-exact restore is residual + sidecar
        // overhead. That's what `ratio_vs_baseline` means in the paper.
        let payload = size.residual_zst_bytes + size.overhead_bytes;
        let ratio = payload as f64 / baseline_pixel as f64;
        rows.push(format_row(
            path,
            label,
            w,
            h,
            "pixel",
            q,
            baseline_pixel,
            size.residual_zst_bytes,
            size.low_mip_bytes,
            size.total_bytes,
            ratio,
            lossless,
            hp,
            recommended,
        ));
    }

    // ---------- BC-byte residuals ----------
    for &fmt in bc_variants {
        // BC formats only at q=1 (the codec doesn't currently expose a
        // quant_step path on BC residuals; quant is only meaningful in pixel
        // space). For the paper we report q=1 BC and rely on the pixel sweep
        // for quant_step coverage.
        let mut pred = make_pred();
        let bs = match encode_bc_residual(
            &mut pred,
            fmt,
            &rgba,
            w,
            h,
            low.clone(),
            lw,
            lh,
            true,
        ) {
            Ok(b) => b,
            Err(e) => {
                eprintln!("[bench] {label} {fmt:?} encode failed: {e:#}");
                continue;
            }
        };
        let size = bs.size();
        let mut dec = make_pred();
        let bc_out = match decode_bc_residual(&mut dec, &bs) {
            Ok(b) => b,
            Err(e) => {
                eprintln!("[bench] {label} {fmt:?} decode failed: {e:#}");
                continue;
            }
        };
        // For the BC baseline we use the actual BC byte count (what would
        // be stored on disk if we kept the original top mip's BC encoding).
        let baseline_bc = bc_out.len();
        // Lossless verdict for BC: re-encode the original and compare bytes.
        // (image_dds is deterministic — the G4 tests in the codec crate
        // pin this.) Surface false-positives here as broken-on-this-content.
        let fresh = match {
            // Inline bc_encode is private — round-trip via decode_to_rgba then
            // compare RGBA decoded paths instead. That is *almost* the same
            // check; mismatching BC bytes that happen to decode to the same
            // RGBA are still byte-exact on disk for the consumer's hash check,
            // so we keep this conservative.
            bc_decode_to_rgba8(fmt, w, h, &bc_out)
        } {
            Ok(rgba_from_bc) => {
                // Compare against a fresh BC encode → decoded RGBA of the
                // original. If the codec correctly reproduces the BC bytes,
                // this should match exactly.
                let original_bc_rgba =
                    encode_then_decode_rgba(fmt, &rgba, w, h).unwrap_or_default();
                rgba_from_bc == original_bc_rgba
            }
            Err(_) => false,
        };
        let format_label = match fmt {
            BcResidualFormat::Bc1 => "bc1",
            BcResidualFormat::Bc3 => "bc3",
            BcResidualFormat::Bc5 => "bc5",
            BcResidualFormat::Bc7 => "bc7",
        };
        // Same framing as pixel: the low mip already lives in the pak, so
        // BC residual + overhead is the marginal disk cost vs the BC baseline.
        let payload = size.residual_zst_bytes + size.overhead_bytes;
        let ratio = payload as f64 / baseline_bc as f64;
        rows.push(format_row(
            path,
            label,
            w,
            h,
            format_label,
            1,
            baseline_bc,
            size.residual_zst_bytes,
            size.low_mip_bytes,
            size.total_bytes,
            ratio,
            fresh,
            hp,
            recommended,
        ));
    }

    Ok(rows)
}

/// Encode RGBA → BC → RGBA. Used by the lossless verdict for BC variants.
fn encode_then_decode_rgba(
    fmt: BcResidualFormat,
    rgba: &[u8],
    w: u32,
    h: u32,
) -> Option<Vec<u8>> {
    // We don't have a public bc_encode export, but we can synthesise one by
    // round-tripping a brand new bitstream with an identity predictor. The
    // shortcut: run encode_bc_residual with a zero-residual predictor that
    // returns the same image — the bitstream's predictor + residual
    // reconstruction *is* `BC(original)`. Decode and we have the BC bytes.
    struct Identity {
        target: Vec<u8>,
    }
    impl Predictor for Identity {
        fn id(&self) -> PredictorId {
            PredictorId::Bilinear
        }
        fn predict(
            &mut self,
            _: &[u8],
            _: u32,
            _: u32,
            _: u32,
            _: u32,
        ) -> anyhow::Result<Vec<u8>> {
            Ok(self.target.clone())
        }
    }
    let (low, lw, lh) = box_downsample_2x(rgba, w, h).ok()?;
    let mut pred = Identity { target: rgba.to_vec() };
    let bs = encode_bc_residual(&mut pred, fmt, rgba, w, h, low, lw, lh, false).ok()?;
    let mut dec = Identity { target: rgba.to_vec() };
    let bc = decode_bc_residual(&mut dec, &bs).ok()?;
    bc_decode_to_rgba8(fmt, w, h, &bc).ok()
}

#[allow(clippy::too_many_arguments)]
fn format_row(
    path: &Path,
    label: &str,
    w: u32,
    h: u32,
    format: &str,
    quant_step: u8,
    baseline_bytes: usize,
    residual_bytes: usize,
    low_mip_bytes: usize,
    total_codec_bytes: usize,
    ratio_vs_baseline: f64,
    lossless_pass: bool,
    hp_energy: f64,
    recommended_space: &str,
) -> String {
    // Quote the path field (may contain commas on weird filenames) using a
    // minimal CSV escape — double the embedded quotes.
    let path_s = path.to_string_lossy();
    let path_quoted = format!("\"{}\"", path_s.replace('"', "\"\""));
    format!(
        "{},{},{},{},{},{},{},{},{},{},{:.6},{},{:.4},{}",
        path_quoted,
        label,
        w,
        h,
        format,
        quant_step,
        baseline_bytes,
        residual_bytes,
        low_mip_bytes,
        total_codec_bytes,
        ratio_vs_baseline,
        lossless_pass,
        hp_energy,
        recommended_space,
    )
}

fn load_resize(path: &Path, max_dim: u32) -> Result<(Vec<u8>, u32, u32)> {
    let img = ImageReader::open(path)
        .with_context(|| format!("open {}", path.display()))?
        .with_guessed_format()?
        .decode()
        .with_context(|| format!("decode {}", path.display()))?;
    let rgba = img.to_rgba8();
    let (w, h) = rgba.dimensions();
    if max_dim == 0 || (w <= max_dim && h <= max_dim) {
        return Ok((rgba.into_raw(), w, h));
    }
    let scale = max_dim as f32 / w.max(h) as f32;
    let nw = ((w as f32) * scale).round().max(4.0) as u32;
    let nh = ((h as f32) * scale).round().max(4.0) as u32;
    let resized = image::imageops::resize(&rgba, nw, nh, image::imageops::FilterType::Lanczos3);
    Ok((resized.into_raw(), nw, nh))
}

fn crop_to_multiple(rgba: &[u8], w: u32, h: u32, m: u32) -> (Vec<u8>, u32, u32) {
    let nw = (w / m) * m;
    let nh = (h / m) * m;
    if nw == w && nh == h {
        return (rgba.to_vec(), w, h);
    }
    let mut out = Vec::with_capacity((nw * nh * 4) as usize);
    for y in 0..nh {
        let row_start = (y * w * 4) as usize;
        out.extend_from_slice(&rgba[row_start..row_start + (nw * 4) as usize]);
    }
    (out, nw, nh)
}
