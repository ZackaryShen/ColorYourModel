use super::texture::TextureImportJobs;
use super::workspace::{MeshGuard, TabInfo, Workspace};
use crate::mesh::loader::load_stl;
use crate::mesh::model::MeshDataDto;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use tauri::{ipc::Response, Emitter, State};

pub struct AppState {
    pub workspace: Arc<Mutex<Workspace>>,
}
impl AppState {
    pub fn new() -> Self {
        let workspace = Arc::new(Mutex::new(Workspace::new()));
        Self { workspace }
    }
    pub fn lock_mesh(&self, tab_id: u64) -> Result<MeshGuard<'_>, String> {
        MeshGuard::new(self.workspace.lock().map_err(|e| e.to_string())?, tab_id)
    }
}

fn import_progress(app: tauri::AppHandle) -> impl Fn(f32, &str) {
    move |fraction, stage| {
        let _ = app.emit(
            "import-progress",
            serde_json::json!({"progress":fraction,"stage":stage}),
        );
    }
}

pub(super) fn mesh_response(mut dto: MeshDataDto) -> Result<Response, String> {
    // Five little-endian lengths, then aligned numeric buffers and JSON metadata.
    // Keeping the large arrays binary avoids printing/parsing millions of numbers.
    let vertices = std::mem::take(&mut dto.vertices);
    let faces = std::mem::take(&mut dto.faces);
    let colors = std::mem::take(&mut dto.face_colors);
    let labels = std::mem::take(&mut dto.segment_labels);
    let metadata = serde_json::to_vec(&dto).map_err(|e| e.to_string())?;
    let lengths = [
        vertices.len(),
        faces.len(),
        labels.len(),
        colors.len(),
        metadata.len(),
    ];
    let mut bytes = Vec::with_capacity(
        20 + 4 * (lengths[0] + lengths[1] + lengths[2]) + lengths[3] + lengths[4],
    );
    for len in lengths {
        bytes.extend_from_slice(
            &u32::try_from(len)
                .map_err(|_| "Model exceeds display size")?
                .to_le_bytes(),
        );
    }
    for v in vertices {
        bytes.extend_from_slice(&v.to_le_bytes());
    }
    for f in faces {
        bytes.extend_from_slice(&f.to_le_bytes());
    }
    for label in labels {
        bytes.extend_from_slice(&label.to_le_bytes());
    }
    bytes.extend_from_slice(&colors);
    bytes.extend_from_slice(&metadata);
    Ok(Response::new(bytes))
}

#[tauri::command]
pub async fn load_model(
    path: String,
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    jobs: State<'_, TextureImportJobs>,
) -> Result<Response, String> {
    let workspace = Arc::clone(&state.workspace);
    let jobs = Arc::clone(jobs.inner());
    tauri::async_runtime::spawn_blocking(move || {
        let path = PathBuf::from(path);
        let progress = import_progress(app);
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_lowercase();
        let dto = match ext.as_str() {
            "stl" => {
                let mesh = load_stl(&path, &progress)?;
                let name = path.file_stem().and_then(|s| s.to_str()).unwrap_or("plate");
                let mut ws = workspace.lock().map_err(|e| e.to_string())?;
                let mut jobs = jobs.lock().map_err(|e| e.to_string())?;
                ws.append(mesh, name.to_owned())?;
                jobs.invalidate();
                ws.dto()?
            }
            "3mf" => {
                let mut ws = workspace.lock().map_err(|e| e.to_string())?;
                let mut jobs = jobs.lock().map_err(|e| e.to_string())?;
                ws.open_project(&path, &progress)?;
                jobs.invalidate();
                ws.dto()?
            }
            _ => return Err(format!("Unsupported file format: {ext}")),
        };
        mesh_response(dto)
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn switch_tab(
    tab_id: u64,
    app: tauri::AppHandle,
    state: State<'_, AppState>,
) -> Result<Response, String> {
    let workspace = Arc::clone(&state.workspace);
    tauri::async_runtime::spawn_blocking(move || {
        let dto = {
            let mut ws = workspace.lock().map_err(|e| e.to_string())?;
            ws.switch_with_progress(tab_id, &import_progress(app))?;
            ws.dto()?
        };
        mesh_response(dto)
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn close_tab(tab_id: u64, state: State<'_, AppState>) -> Result<Response, String> {
    let workspace = Arc::clone(&state.workspace);
    tauri::async_runtime::spawn_blocking(move || {
        let dto = {
            let mut ws = workspace.lock().map_err(|e| e.to_string())?;
            ws.close(tab_id)?;
            if ws.active.is_none() {
                return Ok(Response::new(Vec::<u8>::new()));
            }
            ws.dto()?
        };
        mesh_response(dto)
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub fn rename_tab(
    tab_id: u64,
    name: String,
    state: State<AppState>,
) -> Result<Vec<TabInfo>, String> {
    let mut ws = state.workspace.lock().map_err(|e| e.to_string())?;
    ws.rename(tab_id, name)?;
    Ok(ws.infos())
}
#[tauri::command]
pub fn project_tabs(state: State<AppState>) -> Result<Vec<TabInfo>, String> {
    Ok(state.workspace.lock().map_err(|e| e.to_string())?.infos())
}
#[tauri::command]
pub fn get_face_color(
    face_id: u32,
    tab_id: u64,
    state: State<AppState>,
) -> Result<[u8; 4], String> {
    let guard = state.lock_mesh(tab_id)?;
    let mesh = guard.as_ref().ok_or("No mesh loaded")?;
    mesh.face_colors
        .get(face_id as usize)
        .copied()
        .ok_or_else(|| "Face ID out of range".into())
}
