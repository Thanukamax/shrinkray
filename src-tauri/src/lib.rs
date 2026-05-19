use std::collections::HashSet;
use std::path::PathBuf;

use shrinkray_core::{analyze, backup, recompress, strip};

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

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .invoke_handler(tauri::generate_handler![
            analyze_folder,
            backup_status,
            ensure_backup,
            restore_folder,
            plan_strip,
            apply_strip,
            detect_encoders,
            plan_recompress,
            apply_recompress,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
