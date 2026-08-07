use std::path::PathBuf;

use tauri::State;

use crate::commands::mesh::AppState;
use crate::export::paint_color::MAX_EXTRUDER_SLOT;
use crate::export::presets::{summary, ExportSelection, MachineSummary};
use crate::export::quantize::quantize_face_colors;
use crate::export::threemf::export_3mf;

/// The printer / process / filament catalogue the export dialog renders.
///
/// Returns the stripped summary rather than the full library: the flattened
/// preset configs are ~200 KB and the dialog only needs names, nozzle
/// compatibility and material labels.
#[tauri::command]
pub fn list_export_presets() -> Vec<MachineSummary> {
    summary()
}

/// The extruder palette the current mesh would quantise to, as `#RRGGBB`.
///
/// The export dialog renders one filament picker per entry. It has to come
/// from the backend: the dialog must show the slots that will actually be
/// written, and re-deriving them in TypeScript would mean two quantisers that
/// can drift apart.
#[tauri::command]
pub fn export_palette_preview(state: State<AppState>) -> Result<Vec<String>, String> {
    let mesh_guard = state.mesh.lock().map_err(|e| e.to_string())?;
    let mesh = mesh_guard.as_ref().ok_or("No mesh loaded")?;
    let quantized = quantize_face_colors(&mesh.face_colors, MAX_EXTRUDER_SLOT as usize);
    Ok(quantized
        .palette
        .iter()
        .map(|c| format!("#{:02X}{:02X}{:02X}", c[0], c[1], c[2]))
        .collect())
}

/// Export the loaded mesh as a `.3mf`.
///
/// `selection` is optional. `None` keeps the original behaviour — a synthetic
/// "Generic Printer / 0.20mm Standard / Generic PLA" profile — so the command
/// still works before the user has ever opened the export dialog, and so a
/// broken preset library can never block an export outright.
#[tauri::command]
pub fn export_3mf_command(
    path: String,
    selection: Option<ExportSelection>,
    state: State<AppState>,
) -> Result<String, String> {
    let mesh_guard = state.mesh.lock().map_err(|e| e.to_string())?;
    let mesh = mesh_guard.as_ref().ok_or("No mesh loaded")?;

    let output_path = PathBuf::from(&path);
    export_3mf(mesh, &output_path, selection.as_ref())?;

    Ok(format!("Exported to {}", output_path.display()))
}
