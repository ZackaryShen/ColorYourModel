use std::path::PathBuf;
use std::sync::Mutex;

use tauri::{Emitter, State};

use crate::mesh::loader::{load_stl, ProgressFn};
use crate::mesh::model::{MeshDataDto, MeshModel};

/// Global app state holding the current mesh
pub struct AppState {
    pub mesh: Mutex<Option<MeshModel>>,
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
            hidden_labels: Mutex::new(std::collections::HashSet::new()),
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

    // Format dispatch: both branches yield the mesh to show plus how many
    // plates the source declared, so the DTO is built once below.
    let (model, plate_count): (MeshModel, u32) = match ext.as_str() {
        "stl" => (load_stl(&file_path, &*progress_cb)?, 1),
        "3mf" => {
            let project = crate::threemf::read_project(&file_path, &*progress_cb)?;
            let count = project.plates.len() as u32;
            if count > 1 {
                log::warn!("[cmd:load_model] {count} plates; P0 imports the first only");
            }
            let mut plates = project.plates;
            // `read_project` always yields at least one plate (a file with no
            // plate metadata gets a single implicit one).
            (plates.remove(0).mesh, count)
        }
        _ => return Err(format!("Unsupported file format: {}", ext)),
    };

    // Final progress: model loaded, converting to DTO
    let _ = app.emit(
        "import-progress",
        serde_json::json!({ "progress": 1.0, "stage": "Import complete" }),
    );

    let mut dto = model.to_dto();
    dto.plate_index = 1;
    dto.plate_count = plate_count;

    log::info!(
        "[cmd:load_model] DTO ready: {} verts, {} faces, {} color_entries, plate {}/{}",
        dto.vertices.len() / 3,
        dto.faces.len() / 3,
        dto.face_colors.len() / 4,
        dto.plate_index,
        dto.plate_count
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
