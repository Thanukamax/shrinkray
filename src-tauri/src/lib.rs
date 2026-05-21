use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Mutex;

use shrinkray_audit::AuditReport;
use shrinkray_core::ai_restore::{plan_restore_ai, RestoreAiPlan};
use shrinkray_core::classifier::{Policy, TextureFacts};
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
    state: tauri::State<SidecarHandle>,
) -> Result<PlanStripMipsResult, String> {
    state.with(|s| s.plan_strip_mips(&pak_path, max_dim, limit, game.as_deref()))
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
    state: tauri::State<SidecarHandle>,
) -> Result<ApplyStripMipsResult, String> {
    state.with(|s| s.apply_strip_mips(&pak_path, &targets, game.as_deref(), engine_version.as_deref()))
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
            sidecar_plan_restore_ai,
            sidecar_apply_restore_ai,
            list_dir,
            quick_links,
            path_parent,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
