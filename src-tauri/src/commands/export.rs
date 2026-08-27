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

/// Normalize the output path's extension: the OS save dialog (notably Windows
/// "Save As") appends the filter extension even when the user already typed it,
/// producing "...model.3mf.3mf". Strip every trailing copy of the target
/// extension, then add exactly one, so the user can name the file freely
/// without producing an invisible double-extension file.
fn with_normalized_extension(path: &str, ext: &str) -> PathBuf {
    let mut p = PathBuf::from(path);
    let norm_ext = format!(".{}", ext);
    let mut name = p
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "model".to_string());
    while name.to_lowercase().ends_with(&norm_ext) {
        let cut = name.len().saturating_sub(norm_ext.len());
        name.truncate(cut);
    }
    name.push_str(&norm_ext);
    p.set_file_name(&name);
    p
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

    let mut output_path = with_normalized_extension(&path, "3mf");

    let progress = export_progress(&app);
    progress(0.05, "3mf:start");
    export_3mf(mesh, &output_path, selection.as_ref(), &progress)?;
    progress(0.97, "3mf:verify");

    // Adversarial hardening: do NOT trust zip.finish()'s Ok. Confirm the file
    // actually landed on disk and is non-empty, otherwise surface the real
    // failure instead of leaving the user with "100% progress, no file".
    let meta = std::fs::metadata(&output_path)
        .map_err(|e| format!("3MF export finished but file missing: {}", e))?;
    if meta.len() == 0 {
        return Err("3MF export finished but the file is empty (0 bytes)".into());
    }
    let final_path = output_path
        .canonicalize()
        .unwrap_or_else(|_| output_path.clone());
    progress(1.0, "done");

    Ok(format!("Exported to {}", final_path.display()))
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

    let mut output_path = with_normalized_extension(&path, "obj");

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_double_extension() {
        assert_eq!(
            with_normalized_extension("C:\\tmp\\mymodel.3mf.3mf", "3mf")
                .to_string_lossy(),
            "C:\\tmp\\mymodel.3mf"
        );
        assert_eq!(
            with_normalized_extension("mymodel", "3mf").to_string_lossy(),
            "mymodel.3mf"
        );
        assert_eq!(
            with_normalized_extension("my.model.3mf", "3mf").to_string_lossy(),
            "my.model.3mf"
        );
        assert_eq!(
            with_normalized_extension("a.obj.3mf", "3mf").to_string_lossy(),
            "a.obj.3mf"
        );
        assert_eq!(
            with_normalized_extension("model.3MF", "3mf").to_string_lossy(),
            "model.3mf"
        );
    }
}
