use std::path::PathBuf;

use tauri::State;

use crate::commands::mesh::AppState;
use crate::export::threemf::export_3mf;

#[tauri::command]
pub fn export_3mf_command(path: String, state: State<AppState>) -> Result<String, String> {
    let mesh_guard = state.mesh.lock().map_err(|e| e.to_string())?;
    let mesh = mesh_guard.as_ref().ok_or("No mesh loaded")?;

    let output_path = PathBuf::from(&path);
    export_3mf(mesh, &output_path)?;

    Ok(format!("Exported to {}", output_path.display()))
}
