mod analyze;
mod audio;
mod backup;
mod pak;
mod strip;
mod texture;

use std::collections::HashSet;
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

/// Step 2: ensures a backup exists for this folder. If one already exists, its
/// status is returned. Otherwise a fresh differential backup is initialised.
/// Apply commands ([apply_strip]) refuse to run without this in place.
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

/// Step 2: restores every recorded edit in reverse order. Returns per-entry
/// success/failure; no partial rollback. Safe to invoke multiple times.
#[tauri::command]
fn restore_folder(path: String) -> Result<backup::RestoreReport, String> {
    let b = backup::Backup::load(&PathBuf::from(path)).map_err(|e| e.to_string())?;
    b.restore().map_err(|e| e.to_string())
}

/// Step 3: dry-run. Returns what would be deleted / rewritten if the user
/// applied this strip configuration. No writes. Safe to call interactively.
#[tauri::command]
fn plan_strip(path: String, drop_languages: Vec<String>) -> Result<strip::StripPlan, String> {
    let langs: HashSet<String> = drop_languages.into_iter().collect();
    strip::plan(&PathBuf::from(path), &langs).map_err(|e| e.to_string())
}

/// Step 3: applies the strip. Refuses if no backup exists for this folder —
/// frontend must call [ensure_backup] first (or this fails fast and the user
/// is prompted).
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
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
