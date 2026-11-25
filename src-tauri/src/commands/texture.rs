use std::path::Path;
use std::sync::Mutex;

use serde::Serialize;
use tauri::State;

use super::mesh::AppState;
use super::texture_jobs::TextureJobs;
use crate::mesh::loader::build_mesh_indices;
use crate::mesh::model::{MeshDataDto, MeshModel};
use crate::mesh::texture::{load_texture, BakedTexture, OriginalTexture};

pub type TextureImportJobs = Mutex<TextureJobs<MeshModel>>;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct TexturePreview {
    original: OriginalTexture,
    converted: MeshDataDto,
    palette: Vec<[u8; 3]>,
    #[serde(skip_serializing_if = "Option::is_none")]
    warning: Option<&'static str>,
}

fn preview_model(baked: BakedTexture) -> (MeshModel, TexturePreview) {
    let mut model = MeshModel::new();
    model.vertices = baked.vertices;
    model.faces = baked.faces;
    model.face_colors = baked.face_colors;
    model.segment_labels = vec![0; model.faces.len()];
    model.texture_painting = true;
    model.compute_bbox();
    let preview = TexturePreview {
        original: baked.original,
        converted: model.to_dto(),
        palette: baked.palette,
        warning: baked.warning,
    };
    (model, preview)
}

#[tauri::command]
pub(crate) async fn prepare_texture(
    path: String,
    job_id: u64,
    color_limit: u8,
    state: State<'_, AppState>,
) -> Result<TexturePreview, String> {
    let cancelled = state
        .texture_jobs
        .lock()
        .map_err(|error| error.to_string())?
        .start(job_id)?;
    let result = tauri::async_runtime::spawn_blocking(move || {
        let baked = load_texture(Path::new(&path), color_limit, &cancelled)?;
        Ok::<_, String>(preview_model(baked))
    })
    .await
    .map_err(|error| format!("Texture conversion failed: {error}"))
    .and_then(|value| value);

    let mut jobs = state
        .texture_jobs
        .lock()
        .map_err(|error| error.to_string())?;
    match result {
        Ok((model, preview)) => {
            jobs.publish(job_id, model)?;
            Ok(preview)
        }
        Err(error) => {
            jobs.cancel(job_id);
            Err(error)
        }
    }
}

#[tauri::command]
pub(crate) async fn apply_texture(
    job_id: u64,
    state: State<'_, AppState>,
) -> Result<MeshDataDto, String> {
    let _operation = state.begin_mesh_operation()?;
    let mut model = state
        .texture_jobs
        .lock()
        .map_err(|error| error.to_string())?
        .take_for_apply(job_id)?;
    let indexed = tauri::async_runtime::spawn_blocking(move || {
        model.compute_normals();
        build_mesh_indices(&mut model, &|_, _| {});
        let dto = model.to_dto();
        (model, dto)
    })
    .await;

    let mut jobs = state
        .texture_jobs
        .lock()
        .map_err(|error| error.to_string())?;
    let (model, dto) = indexed.map_err(|error| {
        jobs.cancel(job_id);
        format!("Could not prepare the texture mesh for editing: {error}")
    })?;
    let mut active = state.mesh.lock().map_err(|error| error.to_string())?;
    let mut hidden = state
        .hidden_labels
        .lock()
        .map_err(|error| error.to_string())?;
    jobs.complete_apply(job_id)?;
    hidden.clear();
    *active = Some(model);
    Ok(dto)
}

#[tauri::command]
pub(crate) fn cancel_texture(job_id: u64, state: State<'_, AppState>) -> Result<(), String> {
    state
        .texture_jobs
        .lock()
        .map_err(|error| error.to_string())?
        .cancel(job_id);
    Ok(())
}
