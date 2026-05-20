use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Mutex;

use shrinkray_audit::AuditReport;
use shrinkray_core::{analyze, backup, recompress, strip};
use shrinkray_sidecar::{InspectAssetResult, ListAssetsResult, PingResult, Sidecar};

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
#[tauri::command]
fn analyze_folder(path: String) -> analyze::AnalysisReport {
    analyze::analyze(&PathBuf::from(path))
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
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
