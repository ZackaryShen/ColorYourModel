use std::path::PathBuf;

use tauri::{AppHandle, Emitter, State};

use crate::commands::mesh::AppState;
use crate::export::obj::export_obj;
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

/// Build a progress callback that forwards to the frontend as `export-progress`
/// events. The command is `async`, so it runs off the webview thread and these
/// events arrive live instead of being batched until the call returns.
fn export_progress(app: &AppHandle) -> impl Fn(f32, &str) {
    let app = app.clone();
    move |frac: f32, stage: &str| {
        let _ = app.emit(
            "export-progress",
            serde_json::json!({ "progress": frac, "stage": stage }),
        );
    }
}

/// Export the loaded mesh as a `.3mf`.
///
/// `selection` is optional. `None` keeps the original behaviour — a synthetic
/// "Generic Printer / 0.20mm Standard / Generic PLA" profile — so the command
/// still works before the user has ever opened the export dialog, and so a
/// broken preset library can never block an export outright.
#[tauri::command]
pub async fn export_3mf_command(
    path: String,
    selection: Option<ExportSelection>,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<String, String> {
    let mesh_guard = state.mesh.lock().map_err(|e| e.to_string())?;
    let mesh = mesh_guard.as_ref().ok_or("No mesh loaded")?;

    let mut output_path = PathBuf::from(&path);
    // suffix so the file is visible / openable as a 3MF (REF: export broken
    // report — bytes were written but under a suffix-less name).
    if output_path.extension().and_then(|e| e.to_str()) != Some("3mf") {
        let mut name = output_path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "model".to_string());
        if !name.to_lowercase().ends_with(".3mf") {
            name.push_str(".3mf");
        }
        output_path.set_file_name(name);
    }

    let progress = export_progress(&app);
    progress(0.05, "3mf:start");
    export_3mf(mesh, &output_path, selection.as_ref(), &progress)?;
    progress(1.0, "done");

    Ok(format!("Exported to {}", output_path.display()))
}

/// Export the loaded mesh as a `.obj` + sibling `.mtl`, carrying per-face RGB
/// colour through `usemtl` groups. Unlike 3MF, OBJ needs no machine / process /
/// filament selection — the colour is written directly, so the export dialog
/// skips those pickers for this format.
#[tauri::command]
pub async fn export_obj_command(
    path: String,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<String, String> {
    let mesh_guard = state.mesh.lock().map_err(|e| e.to_string())?;
    let mesh = mesh_guard.as_ref().ok_or("No mesh loaded")?;

    let mut output_path = PathBuf::from(&path);
    if output_path.extension().and_then(|e| e.to_str()) != Some("obj") {
        let mut name = output_path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "model".to_string());
        if !name.to_lowercase().ends_with(".obj") {
            name.push_str(".obj");
        }
        output_path.set_file_name(name);
    }

    let progress = export_progress(&app);
    progress(0.05, "obj:start");
    export_obj(mesh, &output_path, &progress)?;
    progress(1.0, "done");

    let mtl_name = output_path
        .with_extension("mtl")
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("model.mtl")
        .to_string();
    Ok(format!(
        "Exported to {} (with {})",
        output_path.display(),
        mtl_name
    ))
}
