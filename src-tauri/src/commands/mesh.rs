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

/// 0.2.0-P2 (3a): bake a whole-model transform into the vertices. The gizmo
/// preview lives on the frontend; on drag-end it sends the composed matrix
/// (in STL-local space, see the frontend conjugation note), the vertices are
/// transformed for real, and the derived state is rebuilt. Undoable via a
/// Transform history entry (inverse matrix; the swap convention stores it
/// inverted, redo re-applies the forward matrix).
///
/// Restore-on-error: the mesh is taken out of the lock during the rebuild
/// (seconds on large models) and put back even on failure — the
/// resegment_region take path historically leaked None here.
#[tauri::command]
pub fn bake_transform(
    matrix: [[f32; 4]; 4],
    state: State<AppState>,
) -> Result<crate::commands::history::HistoryResult, String> {
    log::info!("[cmd:bake_transform]");
    let mut mesh_guard = state.mesh.lock().map_err(|e| e.to_string())?;
    let mut mesh = mesh_guard.take().ok_or("No mesh loaded")?;

    let mut bake = || -> Result<(), String> {
        // Singularity check via nalgebra (transpose shares the determinant, so
        // the convention below is unaffected).
        let nm = nalgebra::Matrix4::from(matrix);
        if nm.try_inverse().is_none() {
            return Err("transform: matrix is singular".to_string());
        }
        // Same column-array formula as History::apply_swap — one convention,
        // two call sites.
        for v in mesh.vertices.iter_mut() {
            let p = [
                matrix[0][0] * v[0] + matrix[1][0] * v[1] + matrix[2][0] * v[2] + matrix[3][0],
                matrix[0][1] * v[0] + matrix[1][1] * v[1] + matrix[2][1] * v[2] + matrix[3][1],
                matrix[0][2] * v[0] + matrix[1][2] * v[1] + matrix[2][2] * v[2] + matrix[3][2],
            ];
            *v = p;
        }
        mesh.compute_normals();
        mesh.compute_bbox();
        mesh.build_kdtree();
        mesh.build_vertex_kdtree();
        mesh.build_adjacency();
        Ok(())
    };
    if let Err(e) = bake() {
        *mesh_guard = Some(mesh); // restore — never leak None (P0 lesson)
        return Err(e);
    }
    mesh.history.record_transform(matrix);

    let dto = mesh.to_dto();
    let result = crate::commands::history::HistoryResult {
        applied: true,
        full: true,
        faces: Vec::new(),
        colors: Vec::new(),
        segments: None,
        segment_labels: None,
        face_colors: None,
        vertices: Some(dto.vertices.clone()),
        bbox: Some(dto.bbox.clone()),
        can_undo: mesh.history.can_undo(),
        can_redo: mesh.history.can_redo(),
    };
    *mesh_guard = Some(mesh);
    Ok(result)
}