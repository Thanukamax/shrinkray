use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Mutex;

use anyhow::Context;
use base64::prelude::{Engine as _, BASE64_STANDARD};
use tauri::Emitter as _;
use shrinkray_audit::AuditReport;
use shrinkray_core::ai_restore::{plan_restore_ai, RestoreAiPlan};
use shrinkray_core::classifier::{Policy, TextureFacts};
use shrinkray_core::texture_strip::{self, AppliedTexture, PakStripReport, SkippedTexture};
use shrinkray_core::inference;
use shrinkray_core::{analyze, backup, recompress, strip};
use shrinkray_sidecar::{
    ApplyStripMipsResult, InspectAssetResult, ListAssetsResult, PingResult, PlanStripMipsResult,
    Sidecar, StripTarget,
};

/// Lazy-initialised sidecar handle. We spawn the .NET process on first use and
/// keep it alive for the lifetime of the app — JSON IPC is cheap, process
/// startup is ~50ms which we'd rather pay once.
#[derive(Default)]
struct SidecarHandle(Mutex<Option<Sidecar>>);

impl SidecarHandle {
    fn with<R>(&self, f: impl FnOnce(&mut Sidecar) -> anyhow::Result<R>) -> Result<R, String> {
        let mut guard = self.0.lock().map_err(|e| e.to_string())?;
        if guard.is_none() {
            let path = Sidecar::locate().map_err(|e| format!("sidecar locate failed: {e}"))?;
            let s = Sidecar::spawn(&path).map_err(|e| format!("sidecar spawn failed: {e}"))?;
            *guard = Some(s);
        }
        f(guard.as_mut().unwrap()).map_err(|e| e.to_string())
    }
}

/// Step 1: extension-based folder census + L10N detection + pak classification.
/// Wrapped in catch_unwind so a panic inside `analyze` surfaces as an IPC error
/// string instead of deadlocking the front-end's `invoke()` await.
#[tauri::command]
fn analyze_folder(path: String) -> Result<analyze::AnalysisReport, String> {
    let root = PathBuf::from(path);
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| analyze::analyze(&root)))
        .map_err(|p| {
            if let Some(s) = p.downcast_ref::<&'static str>() {
                format!("analyze panicked: {s}")
            } else if let Some(s) = p.downcast_ref::<String>() {
                format!("analyze panicked: {s}")
            } else {
                "analyze panicked (non-string payload)".to_string()
            }
        })
}

/// Step 2: returns None if no backup exists for this folder, otherwise a
/// summary of the existing manifest.
#[tauri::command]
fn backup_status(path: String) -> Option<backup::BackupStatus> {
    backup::status(&PathBuf::from(path))
}

/// Step 2: idempotent — returns existing backup status or initialises a fresh
/// differential backup. Apply commands refuse without this in place.
#[tauri::command]
fn ensure_backup(path: String) -> Result<backup::BackupStatus, String> {
    let root = PathBuf::from(&path);
    if let Some(st) = backup::status(&root) {
        return Ok(st);
    }
    backup::Backup::new(&root, backup::BackupMode::Differential)
        .map_err(|e| e.to_string())?;
    backup::status(&root).ok_or_else(|| "backup created but status read failed".to_string())
}

/// Step 2: restores every recorded edit in reverse order.
#[tauri::command]
fn restore_folder(path: String) -> Result<backup::RestoreReport, String> {
    let b = backup::Backup::load(&PathBuf::from(path)).map_err(|e| e.to_string())?;
    b.restore().map_err(|e| e.to_string())
}

/// v0.7.4 — project what an existing backup would weigh if Δ-Codec had been
/// used instead of full-bytes ExactBackup. Reads the existing manifest,
/// applies bench-validated per-class ratios from `docs/delta-codec-spec.md`,
/// and returns aggregate + per-class projections.
///
/// Returns None when no backup exists for the folder.
#[tauri::command]
fn delta_codec_project_backup(path: String) -> Option<backup::DeltaCodecProjection> {
    backup::project_delta_codec_savings(&PathBuf::from(path))
}

/// Step 3: dry-run for L10N strip. Returns what would be deleted / rewritten.
#[tauri::command]
fn plan_strip(path: String, drop_languages: Vec<String>) -> Result<strip::StripPlan, String> {
    let langs: HashSet<String> = drop_languages.into_iter().collect();
    strip::plan(&PathBuf::from(path), &langs).map_err(|e| e.to_string())
}

/// Step 3: applies the L10N strip; refuses without a loaded backup.
#[tauri::command]
fn apply_strip(
    path: String,
    drop_languages: Vec<String>,
) -> Result<strip::StripReport, String> {
    let root = PathBuf::from(&path);
    let mut b = backup::Backup::load(&root)
        .map_err(|e| format!("backup required before apply_strip: {}", e))?;
    let langs: HashSet<String> = drop_languages.into_iter().collect();
    strip::apply(&root, &langs, &mut b).map_err(|e| e.to_string())
}

/// Step 4: probes which loose-file encoders are installed locally.
#[tauri::command]
fn detect_encoders() -> Vec<recompress::EncoderAvailability> {
    recompress::detect_encoders()
}

/// Step 4: dry-run for loose-file recompression — finds every PNG/WAV/FLAC.
#[tauri::command]
fn plan_recompress(path: String) -> Result<recompress::RecompressPlan, String> {
    recompress::plan(&PathBuf::from(path)).map_err(|e| e.to_string())
}

/// Step 4: applies the recompression; refuses without a loaded backup.
#[tauri::command]
fn apply_recompress(path: String) -> Result<recompress::RecompressReport, String> {
    let root = PathBuf::from(&path);
    let mut b = backup::Backup::load(&root)
        .map_err(|e| format!("backup required before apply_recompress: {}", e))?;
    recompress::apply(&root, &mut b).map_err(|e| e.to_string())
}

/// v0.4: read-only bloat audit — runs the default detector roster and returns
/// the report. Never writes a byte.
#[tauri::command]
fn audit_folder(path: String) -> Result<AuditReport, String> {
    shrinkray_audit::audit(&PathBuf::from(path)).map_err(|e| e.to_string())
}

/// v0.5 (preview): probe the .NET sidecar — returns its version string. Used by the
/// UI to verify the sidecar binary is present before exposing the inspector.
#[tauri::command]
fn sidecar_ping(state: tauri::State<SidecarHandle>) -> Result<PingResult, String> {
    state.with(|s| s.ping())
}

/// v0.5 (preview): list cooked entries inside a single readable .pak via CUE4Parse.
/// Returns a structured empty result with `encrypted: true` for AES-locked paks
/// instead of erroring, so the UI can surface that affordance.
#[tauri::command]
fn sidecar_list_assets(
    pak_path: String,
    limit: Option<u32>,
    state: tauri::State<SidecarHandle>,
) -> Result<ListAssetsResult, String> {
    state.with(|s| s.list_assets_with(&pak_path, limit, None))
}

/// v0.5 (preview): inspect a single cooked package — class names, exports,
/// imports, custom-version fingerprint. Foundation for Phase 2 rewriting.
#[tauri::command]
fn sidecar_inspect_asset(
    pak_path: String,
    asset_path: String,
    state: tauri::State<SidecarHandle>,
) -> Result<InspectAssetResult, String> {
    state.with(|s| s.inspect_asset(&pak_path, &asset_path, None))
}

/// v0.5 (preview): walk all readable packages in a pak and project the savings
/// from capping each texture's top mip dimension to `max_dim`. Read-only.
/// `game` is a CUE4Parse EGame string (e.g. "GAME_UE4_27"); UE4 cooks need a
/// UE4 version or typed UTexture casts silently fail.
#[tauri::command]
fn sidecar_plan_strip_mips(
    pak_path: String,
    max_dim: i32,
    limit: Option<i32>,
    game: Option<String>,
    app: tauri::AppHandle,
    state: tauri::State<SidecarHandle>,
) -> Result<PlanStripMipsResult, String> {
    state.with(|s| {
        // The planner emits {op:"plan_strip_mips", current, total, asset_path}
        // every 10 packages. Frontend listens on the same `strip-progress`
        // channel and switches on `op` for the apply vs planner display.
        let mut args = serde_json::Map::new();
        args.insert("pak_path".into(), serde_json::Value::String(pak_path.clone()));
        args.insert("max_dim".into(), serde_json::Value::Number(max_dim.into()));
        if let Some(l) = limit {
            args.insert("limit".into(), serde_json::Value::Number(l.into()));
        }
        if let Some(g) = game.as_deref() {
            args.insert("game".into(), serde_json::Value::String(g.into()));
        }
        let result = s.call_with_progress(
            "plan_strip_mips",
            Some(serde_json::Value::Object(args)),
            |progress| {
                let _ = app.emit("strip-progress", progress);
            },
        )?;
        Ok::<_, anyhow::Error>(serde_json::from_value(result)?)
    })
}

/// v0.6.0-rc1 apply path. Takes the pak + a list of per-texture (asset_path,
/// max_dim) targets, plus optional game/engine version strings. The sidecar
/// returns `ApplyStripMipsResult { applied, skipped, total_saved_bytes }` —
/// targets that hit the in-flight UE4.22 per-mip parser path land in
/// `skipped` with a diagnostic reason, so the UI can surface partial success.
#[tauri::command]
fn sidecar_apply_strip_mips(
    pak_path: String,
    targets: Vec<StripTarget>,
    game: Option<String>,
    engine_version: Option<String>,
    app: tauri::AppHandle,
    state: tauri::State<SidecarHandle>,
) -> Result<ApplyStripMipsResult, String> {
    state.with(|s| {
        s.apply_strip_mips_with_progress(
            &pak_path,
            &targets,
            game.as_deref(),
            engine_version.as_deref(),
            |progress| {
                // Fire-and-forget — a frontend listen failure shouldn't fail the apply.
                let _ = app.emit("strip-progress", progress);
            },
        )
    })
}

/// v0.6.1: end-to-end mip strip applied to a pak on disk. Loads the backup
/// for `folder_path` (fails if absent — mirrors `apply_strip` / `apply_recompress`
/// gate), asks the sidecar to compute modified .uasset/.uexp/.ubulk bytes for
/// every target, then hands those bytes to `texture_strip::apply_to_pak` for
/// the repak substitution + backup recording + atomic rename.
///
/// Per-texture failures (sidecar skips) are surfaced in the report; one bad
/// texture doesn't halt the run. If every target is skipped, the pak is left
/// untouched and the report carries `original_size == new_size`.
#[tauri::command]
fn apply_strip_mips_to_folder(
    folder_path: String,
    pak_path: String,
    targets: Vec<StripTarget>,
    game: Option<String>,
    engine_version: Option<String>,
    app: tauri::AppHandle,
    state: tauri::State<SidecarHandle>,
) -> Result<PakStripReport, String> {
    state.with(|s| {
        apply_strip_mips_to_folder_impl_with_progress(
            s,
            &folder_path,
            &pak_path,
            &targets,
            game.as_deref(),
            engine_version.as_deref(),
            |progress| {
                let _ = app.emit("strip-progress", progress);
            },
        )
    })
}

/// Pure-Rust body of the Tauri command, exposed so integration tests can
/// drive the end-to-end pipeline against a real sidecar + a Pamali pak copy
/// without spinning up the Tauri runtime. Caller supplies the live `Sidecar`.
pub fn apply_strip_mips_to_folder_impl(
    sidecar: &mut Sidecar,
    folder_path: &str,
    pak_path: &str,
    targets: &[StripTarget],
    game: Option<&str>,
    engine_version: Option<&str>,
) -> anyhow::Result<PakStripReport> {
    apply_strip_mips_to_folder_impl_with_progress(
        sidecar,
        folder_path,
        pak_path,
        targets,
        game,
        engine_version,
        |_| {},
    )
}

/// v0.7.2: streamed-progress variant used by the Tauri command. `on_progress`
/// fires once per texture the sidecar finishes (apply_strip_mips emits one
/// event per target before the terminal result).
pub fn apply_strip_mips_to_folder_impl_with_progress<F>(
    sidecar: &mut Sidecar,
    folder_path: &str,
    pak_path: &str,
    targets: &[StripTarget],
    game: Option<&str>,
    engine_version: Option<&str>,
    on_progress: F,
) -> anyhow::Result<PakStripReport>
where
    F: FnMut(&serde_json::Value),
{
    let root = PathBuf::from(folder_path);
    let mut b = backup::Backup::load(&root)
        .with_context(|| format!("backup required before apply_strip_mips_to_folder for {}", root.display()))?;
    let pak = PathBuf::from(pak_path);
    if !pak.exists() {
        anyhow::bail!("pak not found: {}", pak.display());
    }
    let sidecar_result = sidecar.apply_strip_mips_with_progress(
        pak_path,
        targets,
        game,
        engine_version,
        on_progress,
    )?;

    // Convert sidecar StripAppliedTexture → core AppliedTexture (base64 decode
    // the file payloads). Any decode failure aborts the whole run rather than
    // half-rewriting the pak.
    let mut applied = Vec::with_capacity(sidecar_result.applied.len());
    for t in sidecar_result.applied {
        let mut files = std::collections::HashMap::with_capacity(t.files.len());
        for f in t.files {
            let bytes = BASE64_STANDARD
                .decode(&f.bytes_base64)
                .with_context(|| format!("base64 decode failed for {}", f.pak_path))?;
            files.insert(f.pak_path, bytes);
        }
        applied.push(AppliedTexture {
            asset_path: t.asset_path,
            export_name: t.export_name,
            drop_mip_count: t.drop_mip_count.max(0) as u32,
            kept_mip_count: t.kept_mip_count.max(0) as u32,
            original_top_dim: t.original_top_dim.max(0) as u32,
            stripped_top_dim: t.kept_top_dim.max(0) as u32,
            saved_bytes: t.saved_bytes.max(0) as u64,
            pixel_format: t.pixel_format,
            compression_settings: t.compression_settings,
            files,
        });
    }
    let skipped: Vec<SkippedTexture> = sidecar_result
        .skipped
        .into_iter()
        .map(|s| SkippedTexture { asset_path: s.asset_path, reason: s.reason })
        .collect();

    texture_strip::apply_to_pak(&pak, applied, skipped, &mut b)
}

/// v0.7 scaffold: for the given pak + max_dim, produce a per-texture restore
/// routing plan (AI vs exact-backup vs skip). Internally calls plan_strip_mips
/// to get the texture set, then runs each through `shrinkray_core::classifier`.
///
/// The plan's `executor_ready` field is always `false` in v0.7 scaffold — the
/// ONNX inference path lands in v0.7 proper. The plan is still useful pre-v0.7
/// because v0.6's strip path consults it to decide which textures get backed
/// up vs skipped (normal-map exemption etc).
#[tauri::command]
fn sidecar_plan_restore_ai(
    pak_path: String,
    max_dim: i32,
    limit: Option<i32>,
    game: Option<String>,
    policy: Option<String>,
    state: tauri::State<SidecarHandle>,
) -> Result<RestoreAiPlan, String> {
    let resolved_policy = match policy.as_deref() {
        Some("conservative") => Policy::Conservative,
        Some("aggressive") => Policy::Aggressive,
        Some("never_strip") | Some("never-strip") => Policy::NeverStrip,
        _ => Policy::Smart,
    };
    let strip_plan = state.with(|s| s.plan_strip_mips(&pak_path, max_dim, limit, game.as_deref()))?;
    let facts: Vec<TextureFacts> = strip_plan
        .items
        .iter()
        .map(|item| TextureFacts {
            class_name: item.class_name.clone(),
            name: item.export_name.clone(),
            compression_settings: item.compression_settings.clone(),
            pixel_format: item.pixel_format.clone(),
        })
        .collect();
    let mut plan = plan_restore_ai(&facts, resolved_policy);
    // Patch asset_path onto each planned texture using the strip plan's
    // matching by index — plan_restore_ai itself only sees the facts.
    for (planned, item) in plan.textures.iter_mut().zip(strip_plan.items.iter()) {
        planned.asset_path = item.asset_path.clone();
    }
    Ok(plan)
}

/// v0.7 scaffold: stub for the eventual AI-driven restore executor. Returns the
/// plan with `executor_ready=false` and a note explaining v0.7's ONNX work is
/// pending. Lets the frontend wire the "Restore (AI)" button now so v0.7
/// becomes a swap-in, not a new IPC surface.
#[tauri::command]
fn sidecar_apply_restore_ai(
    pak_path: String,
    max_dim: i32,
    policy: Option<String>,
    state: tauri::State<SidecarHandle>,
) -> Result<RestoreAiPlan, String> {
    // Apply path is intentionally the same shape as plan path during scaffold —
    // returns the plan so the UI can render "would have done X" without
    // actually mutating anything yet.
    sidecar_plan_restore_ai(pak_path, max_dim, None, None, policy, state)
}

/// v0.7.0: probe an ONNX model file to confirm it loads + report its
/// input/output shapes. The UI uses this for two things: (1) validate that
/// a user-picked model file is loadable by ORT before kicking off any
/// long-running restore op, and (2) surface input/output dims so the
/// classifier-routed restore in v0.7.1 can refuse mismatched models early.
#[tauri::command]
fn probe_ai_model(model_path: String) -> Result<ProbeAiModelReport, String> {
    let probe = inference::probe_model(std::path::Path::new(&model_path))
        .map_err(|e| e.to_string())?;
    Ok(ProbeAiModelReport {
        inputs: probe
            .inputs
            .into_iter()
            .map(|s| ProbeIo { name: s.name, shape: s.shape })
            .collect(),
        outputs: probe
            .outputs
            .into_iter()
            .map(|s| ProbeIo { name: s.name, shape: s.shape })
            .collect(),
    })
}

#[derive(serde::Serialize)]
struct ProbeAiModelReport {
    inputs: Vec<ProbeIo>,
    outputs: Vec<ProbeIo>,
}

#[derive(serde::Serialize)]
struct ProbeIo {
    name: String,
    shape: String,
}

/// v0.4.x: in-app Win7 Open dialog backing API. Lists one directory level
/// without recursion. Errors map to a string so the front-end can show them.
#[derive(serde::Serialize)]
struct DirEntry {
    name: String,
    path: String,
    is_dir: bool,
    size: u64,
    modified: i64,
    extension: String,
}

#[tauri::command]
fn list_dir(path: String) -> Result<Vec<DirEntry>, String> {
    let p = PathBuf::from(&path);
    let read = std::fs::read_dir(&p).map_err(|e| format!("{}: {e}", p.display()))?;
    let mut out = Vec::new();
    for entry in read.flatten() {
        let meta = match entry.metadata() {
            Ok(m) => m,
            Err(_) => continue,
        };
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.starts_with('.') {
            continue;
        }
        let ext = entry
            .path()
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        let modified = meta
            .modified()
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);
        out.push(DirEntry {
            name,
            path: entry.path().to_string_lossy().into_owned(),
            is_dir: meta.is_dir(),
            size: meta.len(),
            modified,
            extension: ext,
        });
    }
    Ok(out)
}

#[derive(serde::Serialize)]
struct QuickLink {
    label: String,
    path: String,
    kind: &'static str,
}

#[tauri::command]
fn quick_links() -> Vec<QuickLink> {
    let mut out = Vec::new();
    let home = dirs::home_dir();
    let push = |out: &mut Vec<QuickLink>, label: &str, path: Option<PathBuf>, kind: &'static str| {
        if let Some(p) = path {
            if p.exists() {
                out.push(QuickLink {
                    label: label.to_string(),
                    path: p.to_string_lossy().into_owned(),
                    kind,
                });
            }
        }
    };
    push(&mut out, "Home", home.clone(), "home");
    push(&mut out, "Desktop", dirs::desktop_dir(), "desktop");
    push(&mut out, "Downloads", dirs::download_dir(), "download");
    push(&mut out, "Documents", dirs::document_dir(), "doc");
    push(&mut out, "Videos", dirs::video_dir(), "video");
    // Linux mount points where games typically live.
    for media_root in ["/media", "/mnt", "/run/media"] {
        let mr = PathBuf::from(media_root);
        if let Ok(read) = std::fs::read_dir(&mr) {
            for entry in read.flatten() {
                if entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                    // Inside /media/<user>/... on Fedora — flatten one level.
                    if media_root == "/media" || media_root == "/run/media" {
                        if let Ok(inner) = std::fs::read_dir(entry.path()) {
                            for sub in inner.flatten() {
                                if sub.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                                    out.push(QuickLink {
                                        label: sub.file_name().to_string_lossy().into_owned(),
                                        path: sub.path().to_string_lossy().into_owned(),
                                        kind: "drive",
                                    });
                                }
                            }
                            continue;
                        }
                    }
                    out.push(QuickLink {
                        label: entry.file_name().to_string_lossy().into_owned(),
                        path: entry.path().to_string_lossy().into_owned(),
                        kind: "drive",
                    });
                }
            }
        }
    }
    out
}

#[tauri::command]
fn path_parent(path: String) -> Option<String> {
    PathBuf::from(path)
        .parent()
        .map(|p| p.to_string_lossy().into_owned())
}

// ============================================================
// v0.7.4 — Δ-Codec live demo Tauri commands.
//
// See `docs/delta-codec-spec.md` for the claim, bitstream, and measurement
// protocol. The two commands below run the bench in-process so the UI can
// surface real numbers (not pre-baked screenshots) during the demo. Output
// rows match `shrinkray_delta_codec::bench` shape so the React side can
// render without further translation.

#[derive(serde::Serialize, Debug)]
struct DeltaCodecBenchRow {
    sample: String,
    quant_step: u8,
    top_mip_bytes: usize,
    low_mip_bytes: usize,
    residual_zst_bytes: usize,
    delta_total_bytes: usize,
    ratio: f64,
    max_channel_error: u32,
    byte_exact: bool,
}

#[derive(serde::Serialize, Debug)]
struct DeltaCodecBenchResult {
    rows: Vec<DeltaCodecBenchRow>,
    best_lossless_ratio: f64,
    lossless_runs: u32,
    total_runs: u32,
    spec_version: String,
}

fn run_bench_on_sample(
    label: &str,
    rgba: &[u8],
    w: u32,
    h: u32,
) -> Result<Vec<DeltaCodecBenchRow>, String> {
    use shrinkray_delta_codec::{
        box_downsample_2x, decode_texture, encode_texture, BilinearPredictor,
    };

    let baseline = rgba.len();
    let (low, lw, lh) = box_downsample_2x(rgba, w, h).map_err(|e| e.to_string())?;
    let mut rows = Vec::with_capacity(3);
    for q in [1u8, 2, 4] {
        let mut predictor = BilinearPredictor;
        let bs = encode_texture(
            &mut predictor,
            rgba,
            w,
            h,
            low.clone(),
            lw,
            lh,
            q,
            q == 1,
        )
        .map_err(|e| e.to_string())?;
        let size = bs.size();
        let mut dec_pred = BilinearPredictor;
        let restored = decode_texture(&mut dec_pred, &bs).map_err(|e| e.to_string())?;
        let max_err = rgba
            .iter()
            .zip(restored.iter())
            .map(|(a, b)| ((*a as i32) - (*b as i32)).unsigned_abs())
            .max()
            .unwrap_or(0);
        let byte_exact = restored == rgba;
        rows.push(DeltaCodecBenchRow {
            sample: label.to_string(),
            quant_step: q,
            top_mip_bytes: baseline,
            low_mip_bytes: size.low_mip_bytes,
            residual_zst_bytes: size.residual_zst_bytes,
            delta_total_bytes: size.total_bytes,
            ratio: size.total_bytes as f64 / baseline as f64,
            max_channel_error: max_err,
            byte_exact,
        });
    }
    Ok(rows)
}

fn finalize_bench(mut rows: Vec<DeltaCodecBenchRow>) -> DeltaCodecBenchResult {
    rows.sort_by_key(|r| (r.sample.clone(), r.quant_step));
    let mut best_ratio = f64::INFINITY;
    let mut lossless_runs = 0u32;
    let total_runs = rows.len() as u32;
    for r in &rows {
        if r.quant_step == 1 && r.byte_exact {
            lossless_runs += 1;
        }
        if r.ratio < best_ratio {
            best_ratio = r.ratio;
        }
    }
    DeltaCodecBenchResult {
        rows,
        best_lossless_ratio: best_ratio,
        lossless_runs,
        total_runs,
        spec_version: "delta-codec-v1".to_string(),
    }
}

/// v0.7.4 — run Δ-Codec against three deterministic synthetic content
/// classes (smooth gradient, textured gradient, high-frequency noise) at
/// 256×256. Fast (<1s), reproducible, no model needed. The UI calls this
/// first so a fresh user sees real numbers immediately.
#[tauri::command]
fn delta_codec_run_synthetic_bench() -> Result<DeltaCodecBenchResult, String> {
    let dim = 256u32;
    let mut all = Vec::new();
    all.extend(run_bench_on_sample(
        "smooth_gradient",
        &synth_smooth(dim, dim),
        dim,
        dim,
    )?);
    all.extend(run_bench_on_sample(
        "textured_gradient",
        &synth_textured(dim, dim),
        dim,
        dim,
    )?);
    all.extend(run_bench_on_sample(
        "high_freq_noise",
        &synth_noise(dim, dim),
        dim,
        dim,
    )?);
    Ok(finalize_bench(all))
}

/// v0.7.4 — Δ-Codec against a user-supplied image file (PNG/JPG). Resizes
/// the image to a 1024×1024 max so the bench stays interactive. This is the
/// "pick any image, watch it become byte-exactly compressed" demo move.
#[tauri::command]
fn delta_codec_run_file_bench(path: String) -> Result<DeltaCodecBenchResult, String> {
    let img = image::open(&path).map_err(|e| format!("open {path}: {e}"))?;
    let rgba = img.to_rgba8();
    let (w, h) = rgba.dimensions();
    let max_dim = 1024u32;
    let (final_w, final_h, bytes) = if w > max_dim || h > max_dim {
        let scale = (max_dim as f32 / w.max(h) as f32).min(1.0);
        let nw = (w as f32 * scale).round() as u32;
        let nh = (h as f32 * scale).round() as u32;
        let resized = image::imageops::resize(
            &rgba,
            nw,
            nh,
            image::imageops::FilterType::Lanczos3,
        );
        (nw, nh, resized.into_raw())
    } else {
        (w, h, rgba.into_raw())
    };
    let label = std::path::Path::new(&path)
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "<unnamed>".to_string());
    let rows = run_bench_on_sample(&label, &bytes, final_w, final_h)?;
    Ok(finalize_bench(rows))
}

fn synth_smooth(w: u32, h: u32) -> Vec<u8> {
    let mut out = Vec::with_capacity((w * h * 4) as usize);
    for y in 0..h {
        for x in 0..w {
            let cx = w as f32 / 2.0;
            let cy = h as f32 / 2.0;
            let dx = x as f32 - cx;
            let dy = y as f32 - cy;
            let d = (dx * dx + dy * dy).sqrt();
            let intensity = (255.0 - d).clamp(0.0, 255.0) as u8;
            out.push(intensity);
            out.push((intensity / 2 + 64) as u8);
            out.push((255 - intensity) as u8);
            out.push(255);
        }
    }
    out
}

fn synth_textured(w: u32, h: u32) -> Vec<u8> {
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

fn synth_noise(w: u32, h: u32) -> Vec<u8> {
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

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .manage(SidecarHandle::default())
        .invoke_handler(tauri::generate_handler![
            analyze_folder,
            audit_folder,
            backup_status,
            ensure_backup,
            restore_folder,
            plan_strip,
            apply_strip,
            detect_encoders,
            plan_recompress,
            apply_recompress,
            sidecar_ping,
            sidecar_list_assets,
            sidecar_inspect_asset,
            sidecar_plan_strip_mips,
            sidecar_apply_strip_mips,
            apply_strip_mips_to_folder,
            sidecar_plan_restore_ai,
            sidecar_apply_restore_ai,
            probe_ai_model,
            list_dir,
            quick_links,
            path_parent,
            delta_codec_run_synthetic_bench,
            delta_codec_run_file_bench,
            delta_codec_project_backup,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
