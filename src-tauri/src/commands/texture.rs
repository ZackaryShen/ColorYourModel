use std::path::Path;
use std::sync::{Arc, Mutex};

use serde::Serialize;
use tauri::{ipc::Response, State};

use super::mesh::{mesh_response, AppState};
use super::texture_jobs::TextureJobs;
use super::workspace::Workspace;
use crate::mesh::loader::build_mesh_indices;
use crate::mesh::model::{MeshDataDto, MeshModel};
use crate::mesh::texture::{load_texture, BakedTexture, OriginalTexture};

pub(crate) type TextureImportJobs = Arc<Mutex<TextureJobs<TextureCandidate>>>;

pub(crate) struct TextureCandidate {
    model: MeshModel,
    name: String,
    active_tab: Option<u64>,
}

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
    jobs: State<'_, TextureImportJobs>,
) -> Result<TexturePreview, String> {
    let (active_tab, cancelled) = {
        // Model replacement and job changes take the workspace lock first.
        let workspace = state.workspace.lock().map_err(|error| error.to_string())?;
        let cancelled = jobs
            .lock()
            .map_err(|error| error.to_string())?
            .start(job_id)?;
        (workspace.active_id(), cancelled)
    };
    let result = tauri::async_runtime::spawn_blocking(move || {
        let path = Path::new(&path);
        let baked = load_texture(path, color_limit, &cancelled)?;
        let (model, preview) = preview_model(baked);
        let name = path
            .file_stem()
            .and_then(|stem| stem.to_str())
            .unwrap_or("plate");
        Ok::<_, String>((
            TextureCandidate {
                model,
                name: name.to_owned(),
                active_tab,
            },
            preview,
        ))
    })
    .await
    .map_err(|error| format!("Texture conversion failed: {error}"))
    .and_then(|value| value);

    let mut jobs = jobs.lock().map_err(|error| error.to_string())?;
    match result {
        Ok((candidate, preview)) => {
            jobs.publish(job_id, candidate)?;
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
    jobs: State<'_, TextureImportJobs>,
) -> Result<Response, String> {
    let mut candidate = jobs
        .lock()
        .map_err(|error| error.to_string())?
        .take_for_apply(job_id)?;
    let indexed = tauri::async_runtime::spawn_blocking(move || {
        candidate.model.compute_normals();
        build_mesh_indices(&mut candidate.model, &|_, _| {});
        candidate
    })
    .await;

    let mut workspace = state.workspace.lock().map_err(|error| error.to_string())?;
    let mut jobs = jobs.lock().map_err(|error| error.to_string())?;
    let candidate = indexed.map_err(|error| {
        jobs.cancel(job_id);
        format!("Could not prepare the texture mesh for editing: {error}")
    })?;
    mesh_response(install_texture_candidate(
        &mut workspace,
        &mut jobs,
        job_id,
        candidate,
    )?)
}

fn install_texture_candidate(
    workspace: &mut Workspace,
    jobs: &mut TextureJobs<TextureCandidate>,
    job_id: u64,
    candidate: TextureCandidate,
) -> Result<MeshDataDto, String> {
    if workspace.active_id() != candidate.active_tab {
        jobs.cancel(job_id);
        return Err("The active plate changed while preparing the texture".into());
    }
    workspace
        .ensure_idle()
        .inspect_err(|_| jobs.cancel(job_id))?;
    jobs.complete_apply(job_id)?;
    workspace.append(candidate.model, candidate.name)?;
    workspace.dto()
}

#[tauri::command]
pub(crate) fn cancel_texture(
    job_id: u64,
    jobs: State<'_, TextureImportJobs>,
) -> Result<(), String> {
    jobs.lock()
        .map_err(|error| error.to_string())?
        .cancel(job_id);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ready_candidate(workspace: &Workspace) -> TextureJobs<TextureCandidate> {
        let mut jobs = TextureJobs::default();
        jobs.start(1).unwrap();
        let mut model = MeshModel::new();
        model.texture_painting = true;
        jobs.publish(
            1,
            TextureCandidate {
                model,
                name: "Texture".into(),
                active_tab: workspace.active_id(),
            },
        )
        .unwrap();
        jobs
    }

    #[test]
    fn confirmed_texture_adds_one_dirty_plate_and_keeps_project_path() {
        let mut workspace = Workspace::new();
        let old = workspace
            .append(MeshModel::new(), "Existing".into())
            .unwrap();
        workspace.tabs[0].dirty = false;
        workspace.project_path = Some("existing.3mf".into());
        let mut jobs = ready_candidate(&workspace);
        let candidate = jobs.take_for_apply(1).unwrap();
        let dto = install_texture_candidate(&mut workspace, &mut jobs, 1, candidate).unwrap();

        assert_eq!(workspace.tabs.len(), 2);
        assert_eq!(workspace.tabs[0].id, old);
        assert!(!workspace.tabs[0].dirty);
        assert!(!workspace.tabs[0].mesh.as_ref().unwrap().texture_painting);
        assert_eq!(
            workspace.project_path.as_deref(),
            Some(Path::new("existing.3mf"))
        );
        assert_eq!(workspace.active_id(), Some(dto.tab_id));
        assert!(dto.texture_painting);
        assert!(workspace.tabs[1].dirty);
    }

    #[test]
    fn superseded_or_tab_switched_texture_cannot_append_a_plate() {
        let mut workspace = Workspace::new();
        let a = workspace.append(MeshModel::new(), "A".into()).unwrap();
        let b = workspace.append(MeshModel::new(), "B".into()).unwrap();
        let mut jobs = ready_candidate(&workspace);
        let candidate = jobs.take_for_apply(1).unwrap();
        workspace.switch(a).unwrap();
        assert!(install_texture_candidate(&mut workspace, &mut jobs, 1, candidate).is_err());
        assert_eq!(workspace.tabs.len(), 2);
        assert_eq!(workspace.active_id(), Some(a));
        assert_eq!(workspace.tabs[1].id, b);

        let mut jobs = ready_candidate(&workspace);
        let candidate = jobs.take_for_apply(1).unwrap();
        jobs.cancel(1);
        assert!(install_texture_candidate(&mut workspace, &mut jobs, 1, candidate).is_err());
        assert_eq!(workspace.tabs.len(), 2);
        assert_eq!(workspace.active_id(), Some(a));
    }
}
