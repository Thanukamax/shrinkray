mod analyze;
mod audio;
mod backup;
mod pak;
mod texture;

use std::path::PathBuf;

/// Step 1: extension-based folder census + L10N detection + pak classification.
/// Step 3+ extends with real per-asset analysis.
#[tauri::command]
fn analyze_folder(path: String) -> analyze::AnalysisReport {
    analyze::analyze(&PathBuf::from(path))
}

/// Step 2: returns None if no backup exists for this folder, otherwise a
/// summary of the existing manifest. Cheap — does not load full entries.
#[tauri::command]
fn backup_status(path: String) -> Option<backup::BackupStatus> {
    backup::status(&PathBuf::from(path))
}

/// Step 2: restores every recorded edit in reverse order. Returns per-entry
/// success/failure; no partial rollback. Safe to invoke multiple times.
#[tauri::command]
fn restore_folder(path: String) -> Result<backup::RestoreReport, String> {
    let b = backup::Backup::load(&PathBuf::from(path)).map_err(|e| e.to_string())?;
    b.restore().map_err(|e| e.to_string())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .invoke_handler(tauri::generate_handler![
            analyze_folder,
            backup_status,
            restore_folder,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
