mod analyze;
mod audio;
mod pak;
mod texture;

use std::path::PathBuf;

/// Phase 0: extension-based folder census. Returns counts + sizes per category
/// plus a rough savings estimate. Phase 1+ replaces the estimate with real
/// per-asset analysis from texture.rs / audio.rs / pak.rs.
#[tauri::command]
fn analyze_folder(path: String) -> analyze::AnalysisReport {
    analyze::analyze(&PathBuf::from(path))
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .invoke_handler(tauri::generate_handler![analyze_folder])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
