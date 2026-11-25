use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

use tauri::{Emitter, State};

use super::texture::TextureImportJobs;
use crate::engineering::open_project;
use crate::mesh::loader::{load_stl, ProgressFn};
use crate::mesh::model::{MeshDataDto, MeshModel};

/// Global app state holding the current mesh
pub struct AppState {
    pub mesh: Mutex<Option<MeshModel>>,
    pub(crate) texture_jobs: TextureImportJobs,
    mesh_operation: AtomicBool,
    /// Hidden-region view state (#24/#29), synced from the frontend via
    /// `set_hidden_labels`. Consumed by the implicit spatial paint tools'
    /// cull; NOT used by explicit list-based actions (multi-select fill,
    /// colour replace). Emptied on model load by the frontend sync.
    pub hidden_labels: Mutex<std::collections::HashSet<u32>>,
}

impl AppState {
    pub fn new() -> Self {
        Self {
            mesh: Mutex::new(None),
            texture_jobs: Mutex::default(),
            mesh_operation: AtomicBool::new(false),
            hidden_labels: Mutex::new(std::collections::HashSet::new()),
        }
    }

    /// Import and edits that move the mesh out of its mutex cannot overlap.
    /// Regular paint operations keep using the mesh mutex.
    pub(crate) fn begin_mesh_operation(&self) -> Result<MeshOperation<'_>, &'static str> {
        self.mesh_operation
            .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
            .map_err(|_| "Wait for the current model operation to finish")?;
        Ok(MeshOperation(&self.mesh_operation))
    }
}

pub(crate) struct MeshOperation<'a>(&'a AtomicBool);

impl Drop for MeshOperation<'_> {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}

#[tauri::command]
pub async fn load_model(
    path: String,
    app: tauri::AppHandle,
    state: State<'_, AppState>,
) -> Result<MeshDataDto, String> {
    let _operation = state.begin_mesh_operation()?;
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

    let (model, warning) =
        tauri::async_runtime::spawn_blocking(move || match ext.as_str() {
            "stl" => load_stl(&file_path, &*progress_cb).map(|model| (model, None)),
            "3mf" => open_project(&file_path, &*progress_cb),
            _ => Err(format!("Unsupported file format: {}", ext)),
        })
        .await
        .map_err(|error| format!("Model import failed: {error}"))??;

    // Final progress: model loaded, converting to DTO
    let _ = app.emit(
        "import-progress",
        serde_json::json!({ "progress": 1.0, "stage": "Import complete" }),
    );

    let mut dto = model.to_dto();
    dto.import_warning = warning;

    log::info!(
        "[cmd:load_model] DTO ready: {} verts, {} faces, {} color_entries",
        dto.vertices.len() / 3,
        dto.faces.len() / 3,
        dto.face_colors.len() / 4
    );

    let mut jobs = state.texture_jobs.lock().map_err(|e| e.to_string())?;
    let mut mesh_guard = state.mesh.lock().map_err(|e| e.to_string())?;
    let mut hidden_labels = state.hidden_labels.lock().map_err(|e| e.to_string())?;
    jobs.invalidate();
    hidden_labels.clear();
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

#[cfg(test)]
mod replacement_tests {
    use super::*;

    #[test]
    fn replacement_cannot_interleave_a_mesh_owning_edit() {
        let state = AppState::new();
        let editing = state.begin_mesh_operation().unwrap();
        assert!(state.begin_mesh_operation().is_err());
        drop(editing);
        assert!(state.begin_mesh_operation().is_ok());
    }
}
