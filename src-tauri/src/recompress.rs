//! Step 4 — loose-file recompression via external tools.
//!
//! Per research (B, C, D): cooked UE assets need a parser we don't have yet,
//! so v0.2.0 only touches *loose* files — Marketplace zips, mod packs, RPG-
//! Maker-on-UE ports. Two pipelines, both shell-outs to avoid heavy native
//! deps (libopus, ISPC):
//!
//! - PNG    → `oxipng --opt 4 --strip safe` (lossless re-deflate; same path)
//! - WAV    → `opusenc --bitrate 96` → `<basename>.opus` (lossy, big win)
//! - FLAC   → `opusenc --bitrate 96` → `<basename>.opus` (lossy, big win)
//!
//! Loose DDS → BC7 is deferred (rare in loose form, needs the
//! image_dds + intel_tex_2 stack — Step 4.5 / v0.3+).
//!
//! Every write goes through Backup: PNG via `record_full_replace` (same
//! extension), audio via `record_delete(wav)` + `record_create(opus)`. The
//! new file is validated by its container magic before the swap is committed.

use anyhow::{anyhow, Context, Result};
use serde::Serialize;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use walkdir::WalkDir;

use crate::backup::Backup;

const OPUS_BITRATE_KBPS: u32 = 96;
const PNG_MAGIC: [u8; 8] = [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
const OGG_MAGIC: [u8; 4] = [b'O', b'g', b'g', b'S'];

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum Encoder {
    Oxipng,
    Opusenc,
}

impl Encoder {
    fn binary(&self) -> &'static str {
        match self {
            Encoder::Oxipng => "oxipng",
            Encoder::Opusenc => "opusenc",
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct EncoderAvailability {
    pub encoder: Encoder,
    pub available: bool,
    pub version: Option<String>,
    pub install_hint: &'static str,
}

pub fn detect_encoders() -> Vec<EncoderAvailability> {
    [Encoder::Oxipng, Encoder::Opusenc]
        .iter()
        .map(|&e| probe_encoder(e))
        .collect()
}

fn probe_encoder(encoder: Encoder) -> EncoderAvailability {
    let install_hint = match encoder {
        Encoder::Oxipng => "cargo install oxipng  (or your distro's package)",
        Encoder::Opusenc => "install opus-tools (libopus + opusenc)",
    };
    let output = Command::new(encoder.binary()).arg("--version").output();
    match output {
        Ok(out) if out.status.success() => {
            let raw = String::from_utf8_lossy(&out.stdout);
            let version = raw.lines().next().map(|s| s.trim().to_string());
            EncoderAvailability {
                encoder,
                available: true,
                version,
                install_hint,
            }
        }
        _ => EncoderAvailability {
            encoder,
            available: false,
            version: None,
            install_hint,
        },
    }
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    Png,
    Wav,
    Flac,
}

impl Kind {
    fn encoder(&self) -> Encoder {
        match self {
            Kind::Png => Encoder::Oxipng,
            Kind::Wav | Kind::Flac => Encoder::Opusenc,
        }
    }
}

fn classify(path: &Path) -> Option<Kind> {
    let ext = path.extension().and_then(|e| e.to_str())?.to_ascii_lowercase();
    match ext.as_str() {
        "png" => Some(Kind::Png),
        "wav" => Some(Kind::Wav),
        "flac" => Some(Kind::Flac),
        _ => None,
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct PlannedItem {
    pub path: String,
    pub kind: Kind,
    pub encoder: Encoder,
    pub size: u64,
}

#[derive(Debug, Serialize, Default)]
pub struct RecompressPlan {
    pub root: String,
    pub items: Vec<PlannedItem>,
    pub total_input_bytes: u64,
    pub missing_encoders: Vec<Encoder>,
}

pub fn plan(root: &Path) -> Result<RecompressPlan> {
    let canonical = root.canonicalize()
        .with_context(|| format!("canonicalize {}", root.display()))?;
    let mut out = RecompressPlan {
        root: canonical.to_string_lossy().into_owned(),
        ..Default::default()
    };
    let availability = detect_encoders();
    out.missing_encoders = availability
        .iter()
        .filter(|a| !a.available)
        .map(|a| a.encoder)
        .collect();

    for entry in WalkDir::new(&canonical).into_iter().filter_map(Result::ok) {
        if !entry.file_type().is_file() {
            continue;
        }
        if let Some(kind) = classify(entry.path()) {
            let size = entry.metadata().map(|m| m.len()).unwrap_or(0);
            out.items.push(PlannedItem {
                path: rel_posix(entry.path(), &canonical),
                kind,
                encoder: kind.encoder(),
                size,
            });
            out.total_input_bytes += size;
        }
    }
    Ok(out)
}

#[derive(Debug, Serialize)]
pub struct RecompressResult {
    pub path: String,
    pub kind: Kind,
    pub original_size: u64,
    pub new_size: u64,
    pub bytes_saved: i64,
    /// Final on-disk path. Differs from `path` for audio (extension changes
    /// to .opus).
    pub new_path: String,
}

#[derive(Debug, Serialize)]
pub struct RecompressFailure {
    pub path: String,
    pub kind: Kind,
    pub reason: String,
}

#[derive(Debug, Serialize, Default)]
pub struct RecompressReport {
    pub recompressed: Vec<RecompressResult>,
    pub skipped_no_improvement: Vec<String>,
    pub failures: Vec<RecompressFailure>,
    pub total_bytes_saved: i64,
}

pub fn apply(root: &Path, backup: &mut Backup) -> Result<RecompressReport> {
    let plan = plan(root)?;
    let canonical = PathBuf::from(&plan.root);
    let availability = detect_encoders();
    let mut report = RecompressReport::default();

    for item in plan.items {
        let installed = availability
            .iter()
            .any(|a| a.encoder == item.encoder && a.available);
        if !installed {
            report.failures.push(RecompressFailure {
                path: item.path.clone(),
                kind: item.kind,
                reason: format!("encoder {:?} not installed", item.encoder),
            });
            continue;
        }
        let abs = canonical.join(&item.path);
        match recompress_one(&abs, item.kind, backup) {
            Ok(Some(res)) => {
                report.total_bytes_saved += res.bytes_saved;
                report.recompressed.push(res);
            }
            Ok(None) => report.skipped_no_improvement.push(item.path),
            Err(e) => report.failures.push(RecompressFailure {
                path: item.path,
                kind: item.kind,
                reason: e.to_string(),
            }),
        }
    }
    Ok(report)
}

fn recompress_one(
    abs: &Path,
    kind: Kind,
    backup: &mut Backup,
) -> Result<Option<RecompressResult>> {
    let original_size = fs::metadata(abs).map(|m| m.len()).unwrap_or(0);
    match kind {
        Kind::Png => recompress_png(abs, original_size, backup),
        Kind::Wav | Kind::Flac => recompress_audio(abs, kind, original_size, backup),
    }
}

fn recompress_png(
    abs: &Path,
    original_size: u64,
    backup: &mut Backup,
) -> Result<Option<RecompressResult>> {
    let tmp = abs.with_extension("png.shrinkray-tmp");
    let status = Command::new("oxipng")
        .arg("--opt").arg("4")
        .arg("--strip").arg("safe")
        .arg("--quiet")
        .arg("--out").arg(&tmp)
        .arg(abs)
        .status()
        .with_context(|| format!("spawn oxipng for {}", abs.display()))?;
    if !status.success() {
        let _ = fs::remove_file(&tmp);
        return Err(anyhow!("oxipng exited {} for {}", status, abs.display()));
    }
    let new_bytes = fs::read(&tmp)
        .with_context(|| format!("read oxipng output {}", tmp.display()))?;
    if !has_magic(&new_bytes, &PNG_MAGIC) {
        let _ = fs::remove_file(&tmp);
        return Err(anyhow!("oxipng output for {} is not a PNG", abs.display()));
    }
    let new_size = new_bytes.len() as u64;
    if new_size >= original_size {
        let _ = fs::remove_file(&tmp);
        return Ok(None);
    }
    backup.record_full_replace(abs, &new_bytes)?;
    fs::rename(&tmp, abs)
        .with_context(|| format!("rename {} -> {}", tmp.display(), abs.display()))?;
    let rel = strip_root_prefix(abs, backup.root());
    Ok(Some(RecompressResult {
        path: rel.clone(),
        kind: Kind::Png,
        original_size,
        new_size,
        bytes_saved: original_size as i64 - new_size as i64,
        new_path: rel,
    }))
}

fn recompress_audio(
    src: &Path,
    kind: Kind,
    original_size: u64,
    backup: &mut Backup,
) -> Result<Option<RecompressResult>> {
    let dst = src.with_extension("opus");
    if dst.exists() {
        return Err(anyhow!(
            "target {} already exists — refusing to overwrite",
            dst.display(),
        ));
    }
    let status = Command::new("opusenc")
        .arg("--bitrate").arg(OPUS_BITRATE_KBPS.to_string())
        .arg("--quiet")
        .arg(src)
        .arg(&dst)
        .status()
        .with_context(|| format!("spawn opusenc for {}", src.display()))?;
    if !status.success() {
        let _ = fs::remove_file(&dst);
        return Err(anyhow!("opusenc exited {} for {}", status, src.display()));
    }
    let new_bytes = fs::read(&dst)
        .with_context(|| format!("read opusenc output {}", dst.display()))?;
    if !has_magic(&new_bytes, &OGG_MAGIC) {
        let _ = fs::remove_file(&dst);
        return Err(anyhow!("opusenc output for {} is not an OGG container", src.display()));
    }
    let new_size = new_bytes.len() as u64;
    if new_size >= original_size {
        let _ = fs::remove_file(&dst);
        return Ok(None);
    }
    // opusenc has already written the new file, but we need to record it as a
    // create AFTER the bytes are on disk for the size check to be meaningful.
    // Move-then-record pattern: temporarily move the new file out, record both
    // ops, then put it back.
    let park = dst.with_extension("opus.shrinkray-park");
    fs::rename(&dst, &park)
        .with_context(|| format!("park {} -> {}", dst.display(), park.display()))?;
    backup.record_delete(src)
        .with_context(|| format!("record delete {}", src.display()))?;
    fs::remove_file(src)
        .with_context(|| format!("remove {}", src.display()))?;
    backup.record_create(&dst)
        .with_context(|| format!("record create {}", dst.display()))?;
    fs::rename(&park, &dst)
        .with_context(|| format!("unpark {} -> {}", park.display(), dst.display()))?;

    let src_rel = strip_root_prefix(src, backup.root());
    let dst_rel = strip_root_prefix(&dst, backup.root());
    Ok(Some(RecompressResult {
        path: src_rel,
        kind,
        original_size,
        new_size,
        bytes_saved: original_size as i64 - new_size as i64,
        new_path: dst_rel,
    }))
}

fn has_magic(bytes: &[u8], magic: &[u8]) -> bool {
    bytes.len() >= magic.len() && &bytes[..magic.len()] == magic
}

fn rel_posix(abs: &Path, root: &Path) -> String {
    let stripped = abs.strip_prefix(root).unwrap_or(abs);
    stripped
        .components()
        .map(|c| c.as_os_str().to_string_lossy().into_owned())
        .collect::<Vec<_>>()
        .join("/")
}

fn strip_root_prefix(abs: &Path, root: &Path) -> String {
    rel_posix(abs, root)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backup::BackupMode;

    fn write_wav(path: &Path, sample_rate: u32, samples: usize) {
        // Minimal mono PCM-16 WAV header. Good enough for opusenc to ingest.
        let data_size = samples * 2;
        let byte_rate = sample_rate * 2;
        let mut buf = Vec::with_capacity(44 + data_size);
        buf.extend_from_slice(b"RIFF");
        buf.extend_from_slice(&(36u32 + data_size as u32).to_le_bytes());
        buf.extend_from_slice(b"WAVE");
        buf.extend_from_slice(b"fmt ");
        buf.extend_from_slice(&16u32.to_le_bytes());
        buf.extend_from_slice(&1u16.to_le_bytes());          // PCM
        buf.extend_from_slice(&1u16.to_le_bytes());          // mono
        buf.extend_from_slice(&sample_rate.to_le_bytes());
        buf.extend_from_slice(&byte_rate.to_le_bytes());
        buf.extend_from_slice(&2u16.to_le_bytes());          // block align
        buf.extend_from_slice(&16u16.to_le_bytes());         // bits per sample
        buf.extend_from_slice(b"data");
        buf.extend_from_slice(&(data_size as u32).to_le_bytes());
        // Simple sine-ish content so opusenc has something to encode.
        for i in 0..samples {
            let v = ((i as i32 * 17) & 0x7FFF) as i16;
            buf.extend_from_slice(&v.to_le_bytes());
        }
        fs::write(path, buf).unwrap();
    }

    #[test]
    fn classify_recognises_target_extensions() {
        assert_eq!(classify(Path::new("foo.png")), Some(Kind::Png));
        assert_eq!(classify(Path::new("foo.PNG")), Some(Kind::Png));
        assert_eq!(classify(Path::new("foo.wav")), Some(Kind::Wav));
        assert_eq!(classify(Path::new("foo.flac")), Some(Kind::Flac));
        assert_eq!(classify(Path::new("foo.uasset")), None);
        assert_eq!(classify(Path::new("foo.opus")), None); // already opus
    }

    #[test]
    fn plan_finds_loose_files() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("Game");
        fs::create_dir_all(root.join("Content")).unwrap();
        fs::write(root.join("Content/a.png"), vec![0u8; 1000]).unwrap();
        fs::write(root.join("Content/b.wav"), vec![0u8; 5000]).unwrap();
        fs::write(root.join("Content/c.flac"), vec![0u8; 3000]).unwrap();
        fs::write(root.join("Content/d.uasset"), vec![0u8; 2000]).unwrap();
        let p = plan(&root).unwrap();
        assert_eq!(p.items.len(), 3);
        assert_eq!(p.total_input_bytes, 9000);
    }

    #[test]
    fn apply_skips_when_oxipng_missing() {
        // This test runs in any environment — if oxipng is absent it walks
        // the failure path; if installed it walks the success path on a tiny
        // PNG we don't actually have to fabricate. We use a tiny invalid PNG
        // so success would require oxipng to handle it (it won't); either way
        // the file gets reported via failures / no-improvement, never lost.
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("Game");
        fs::create_dir_all(&root).unwrap();
        let png = root.join("a.png");
        // Real PNG magic + invalid IHDR — oxipng will reject.
        let mut bytes = PNG_MAGIC.to_vec();
        bytes.extend_from_slice(b"junk");
        fs::write(&png, &bytes).unwrap();

        let mut backup = Backup::new(&root, BackupMode::Differential).unwrap();
        let report = apply(&root, &mut backup).unwrap();
        // Either the encoder is missing (failure) or oxipng rejected the
        // broken PNG (also failure). Critical assertion: the file on disk is
        // unchanged.
        assert_eq!(fs::read(&png).unwrap(), bytes);
        assert!(report.recompressed.is_empty());
    }

    /// Real opusenc round-trip — only meaningful when opusenc is installed.
    /// Skipped at runtime via early return if the binary is missing so CI
    /// without opus-tools still passes.
    #[test]
    fn apply_recompresses_wav_to_opus_when_available() {
        let opusenc_ok = probe_encoder(Encoder::Opusenc).available;
        if !opusenc_ok {
            eprintln!("skipping: opusenc not installed");
            return;
        }
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("Game");
        fs::create_dir_all(root.join("Content")).unwrap();
        let wav = root.join("Content/voice.wav");
        let opus = root.join("Content/voice.opus");
        // 48 kHz mono, 2 seconds — ~192 KB WAV that opus@96 will crush.
        write_wav(&wav, 48000, 48000 * 2);
        let original_wav_bytes = fs::read(&wav).unwrap();
        let original_size = original_wav_bytes.len() as u64;

        let mut backup = Backup::new(&root, BackupMode::Differential).unwrap();
        let report = apply(&root, &mut backup).unwrap();

        assert_eq!(report.recompressed.len(), 1);
        assert_eq!(report.failures.len(), 0);
        let r = &report.recompressed[0];
        assert_eq!(r.kind, Kind::Wav);
        assert!(r.new_size < original_size, "opus should be smaller than WAV");
        assert!(opus.exists());
        assert!(!wav.exists());

        // Restore must recreate the WAV byte-for-byte and remove the opus.
        let restore = backup.restore().unwrap();
        assert!(restore.failures.is_empty());
        assert!(wav.exists());
        assert!(!opus.exists());
        assert_eq!(fs::read(&wav).unwrap(), original_wav_bytes);
    }

    #[test]
    fn has_magic_works() {
        assert!(has_magic(&PNG_MAGIC, &PNG_MAGIC));
        assert!(has_magic(b"OggS\x00\x02", &OGG_MAGIC));
        assert!(!has_magic(b"junk", &OGG_MAGIC));
        assert!(!has_magic(b"Og", &OGG_MAGIC)); // too short
    }
}
