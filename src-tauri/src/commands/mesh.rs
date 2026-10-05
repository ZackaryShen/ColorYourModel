use std::path::PathBuf;
use std::sync::Mutex;

use tauri::{Emitter, State};

use crate::mesh::loader::{load_stl, ProgressFn};
use crate::mesh::model::{MeshDataDto, MeshModel};

/// Global app state holding the current mesh
pub struct AppState {
    pub mesh: Mutex<Option<MeshModel>>,
}

impl AppState {
    pub fn new() -> Self {
        Self {
            mesh: Mutex::new(None),
        }
    }
}

#[tauri::command]
pub fn load_model(
    path: String,
    app: tauri::AppHandle,
    state: State<AppState>,
) -> Result<MeshDataDto, String> {
    log::info!("[cmd:load_model] path={}", path);

    let file_path = PathBuf::from(&path);

    let ext = file_path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_lowercase())
        .unwrap_or_default();

    // Build progress callback that emits Tauri events to the frontend
    let app_for_progress = app.clone();
    let progress_cb: Box<ProgressFn> = Box::new(move |fraction: f32, stage: &str| {
        let _ = app_for_progress.emit(
            "import-progress",
            serde_json::json!({ "progress": fraction, "stage": stage }),
        );
    });

    let model = match ext.as_str() {
        "stl" => load_stl(&file_path, &*progress_cb)?,
        _ => return Err(format!("Unsupported file format: {}", ext)),
    };

    // Final progress: model loaded, converting to DTO
    let _ = app.emit(
        "import-progress",
        serde_json::json!({ "progress": 1.0, "stage": "Import complete" }),
    );

    let dto = model.to_dto();

    log::info!(
        "[cmd:load_model] DTO ready: {} verts, {} faces, {} color_entries",
        dto.vertices.len() / 3,
        dto.faces.len() / 3,
        dto.face_colors.len() / 4
    );

    let mut mesh_guard = state.mesh.lock().map_err(|e| e.to_string())?;
    *mesh_guard = Some(model);

    log::info!("[cmd:load_model] state stored, returning to frontend");
    Ok(dto)
}

#[tauri::command]
pub fn get_face_color(face_id: u32, state: State<AppState>) -> Result<[u8; 4], String> {
    let mesh_guard = state.mesh.lock().map_err(|e| e.to_string())?;
    let mesh = mesh_guard.as_ref().ok_or("No mesh loaded")?;
    log::debug!("[cmd:get_face_color] face_id={}", face_id);
    if (face_id as usize) < mesh.face_colors.len() {
        Ok(mesh.face_colors[face_id as usize])
    } else {
        Err("Face ID out of range".to_string())
    }
}

/// Open a .cym project: load the model, replace the app state, and return the
/// same DTO shape as `load_model` so the frontend can reuse its setMeshData
/// path. Undo history is deliberately reset (the format does not persist it).
#[tauri::command]
pub fn load_project(path: String, state: State<AppState>) -> Result<MeshDataDto, String> {
    log::info!("[cmd:load_project] path={}", path);
    let file_path = PathBuf::from(&path);
    let ext = file_path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_lowercase())
        .unwrap_or_default();
    if ext != "cym" {
        return Err(format!("Unsupported project format: {} (expected .cym)", ext));
    }

    let model = crate::mesh::project::load_cym(&file_path)?;
    let dto = model.to_dto();
    log::info!(
        "[cmd:load_project] DTO ready: {} verts, {} faces, {} segments",
        dto.vertices.len() / 3,
        dto.faces.len() / 3,
        dto.segments.len()
    );

    let mut mesh_guard = state.mesh.lock().map_err(|e| e.to_string())?;
    *mesh_guard = Some(model);
    Ok(dto)
}

/// Save the current model as a .cym project. v1 semantics: every save picks
/// a path (no Ctrl+S / last-path memory — documented in the 0.2.0 plan).
/// Zip writing is millisecond-scale, so there is no progress event here.
#[tauri::command]
pub fn save_project(path: String, state: State<AppState>) -> Result<(), String> {
    log::info!("[cmd:save_project] path={}", path);
    let file_path = PathBuf::from(&path);
    let ext = file_path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_lowercase())
        .unwrap_or_default();
    if ext != "cym" {
        return Err(format!(
            "Unsupported project format: {} (expected .cym)",
            ext
        ));
    }

    let mesh_guard = state.mesh.lock().map_err(|e| e.to_string())?;
    let mesh = mesh_guard.as_ref().ok_or("No mesh loaded")?;
    crate::mesh::project::save_cym(mesh, &file_path)?;
    log::info!("[cmd:save_project] saved");
    Ok(())
}
