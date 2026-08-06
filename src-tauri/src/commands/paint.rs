use serde::{Deserialize, Serialize};
use tauri::State;

use crate::commands::mesh::AppState;
use crate::mesh::face_colors::mix_color;
use crate::mesh::kdtree::distance;
use crate::mesh::model::DEFAULT_FACE_COLOR;
use crate::paint::brush::{brush_hit, falloff_strength};
use crate::paint::fill::{region_faces, segment_faces};
use crate::paint::smart_snap::smart_brush_hit;
use crate::paint::spray::spray_hit;

/// Result of a paint operation: list of (face_id, [r,g,b,a])
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PaintResult {
    pub updated_faces: Vec<u32>,
    pub updated_colors: Vec<[u8; 4]>,
}

/// Turn a list of `(face, colour)` updates into the wire format, after
/// committing them through the history-recording write path.
///
/// Every paint command ends this way. Going through one helper is what keeps
/// "painted" and "undoable" from drifting apart: there is no way to write a
/// colour here and forget to record it.
fn commit(
    mesh: &mut crate::mesh::model::MeshModel,
    stroke_id: Option<u64>,
    updates: Vec<(u32, [u8; 4])>,
) -> PaintResult {
    mesh.apply_paint(stroke_id, &updates);
    let (updated_faces, updated_colors) = updates.into_iter().unzip();
    PaintResult {
        updated_faces,
        updated_colors,
    }
}

#[tauri::command]
pub fn brush_paint(
    center_face: u32,
    radius: f32,
    strength: f32,
    falloff_mode: String,
    color: [u8; 4],
    stroke_id: Option<u64>,
    state: State<AppState>,
) -> Result<PaintResult, String> {
    let mut mesh_guard = state.mesh.lock().map_err(|e| e.to_string())?;
    let mesh = mesh_guard.as_mut().ok_or("No mesh loaded")?;

    log::info!(
        "[cmd:brush_paint] center_face={}, radius={:.2}, color={:?}",
        center_face, radius, color
    );

    // [DIAG-B] Log vertex indices for cross-check with frontend [DIAG-A]
    if (center_face as usize) < mesh.faces.len() {
        let f = &mesh.faces[center_face as usize];
        log::info!("[DIAG-B] center_face={} verts=[{},{},{}] face_colors_len={}",
            center_face, f[0], f[1], f[2], mesh.face_colors.len());
    } else {
        log::error!("[DIAG-B] center_face={} OUT OF RANGE (faces.len={})",
            center_face, mesh.faces.len());
    }

    let hits = brush_hit(mesh, center_face, radius);
    let center = mesh.face_center(center_face);

    // Colours are computed against the *current* buffer and written only after
    // the whole batch is handed to `commit`, so a face blended twice inside one
    // call cannot see a half-applied state.
    let updates: Vec<(u32, [u8; 4])> = hits
        .into_iter()
        .map(|fid| {
            let d = distance(&center, &mesh.face_center(fid));
            let s = falloff_strength(d, radius, &falloff_mode) * strength;
            (fid, mix_color(&mesh.face_colors[fid as usize], &color, s))
        })
        .collect();

    Ok(commit(mesh, stroke_id, updates))
}

#[tauri::command]
pub fn fill_paint(
    face_id: u32,
    color: [u8; 4],
    radius: f32,
    state: State<AppState>,
) -> Result<PaintResult, String> {
    let mut mesh_guard = state.mesh.lock().map_err(|e| e.to_string())?;
    let mesh = mesh_guard.as_mut().ok_or("No mesh loaded")?;

    log::info!(
        "[cmd:fill_paint] face_id={}, color={:?}, radius={:.2}",
        face_id, color, radius
    );

    // radius > 0 → LOCAL fill: a bounded blob of faces within `radius` of the
    // clicked face center (kdtree query, O(k), no flood-fill). This is the
    // "智能区分" local-fill path: a single/whole-model partition or unsegmented
    // model no longer floods the ENTIRE model (iteration 18, Issue 1). radius == 0
    // (legacy / backwards-compat) → whole connected-component flood_fill.
    let faces = if radius > 0.0 {
        let center = mesh.face_center(face_id);
        let cand = mesh.faces_within_radius(&center, radius);
        let mut local = Vec::new();
        for fid in cand {
            if distance(&center, &mesh.face_center(fid)) < radius {
                local.push(fid);
            }
        }
        local
    } else {
        region_faces(mesh, face_id)
    };

    // A fill is a single click, never a drag, so it can never coalesce with
    // anything: `None`.
    let updates: Vec<(u32, [u8; 4])> = faces.into_iter().map(|fid| (fid, color)).collect();
    Ok(commit(mesh, None, updates))
}

#[tauri::command]
pub fn fill_segment_paint(
    segment_id: u32,
    color: [u8; 4],
    state: State<AppState>,
) -> Result<PaintResult, String> {
    let mut mesh_guard = state.mesh.lock().map_err(|e| e.to_string())?;
    let mesh = mesh_guard.as_mut().ok_or("No mesh loaded")?;

    let faces = segment_faces(mesh, segment_id);
    let updates: Vec<(u32, [u8; 4])> = faces.into_iter().map(|fid| (fid, color)).collect();
    Ok(commit(mesh, None, updates))
}

#[tauri::command]
pub fn spray_paint(
    center_face: u32,
    radius: f32,
    strength: f32,
    color: [u8; 4],
    density: u32,
    stroke_id: Option<u64>,
    state: State<AppState>,
) -> Result<PaintResult, String> {
    let mut mesh_guard = state.mesh.lock().map_err(|e| e.to_string())?;
    let mesh = mesh_guard.as_mut().ok_or("No mesh loaded")?;

    let updates = spray_hit(mesh, center_face, radius, strength, &color, density);
    Ok(commit(mesh, stroke_id, updates))
}

#[tauri::command]
pub fn smart_brush_paint(
    center_face: u32,
    radius: f32,
    strength: f32,
    falloff_mode: String,
    color: [u8; 4],
    stroke_id: Option<u64>,
    state: State<AppState>,
) -> Result<PaintResult, String> {
    let mut mesh_guard = state.mesh.lock().map_err(|e| e.to_string())?;
    let mesh = mesh_guard.as_mut().ok_or("No mesh loaded")?;

    let updates = smart_brush_hit(mesh, center_face, radius, strength, &falloff_mode, &color);
    Ok(commit(mesh, stroke_id, updates))
}

#[tauri::command]
pub fn erase_paint(
    center_face: u32,
    radius: f32,
    stroke_id: Option<u64>,
    state: State<AppState>,
) -> Result<PaintResult, String> {
    let mut mesh_guard = state.mesh.lock().map_err(|e| e.to_string())?;
    let mesh = mesh_guard.as_mut().ok_or("No mesh loaded")?;

    let hits = brush_hit(mesh, center_face, radius);
    // Iteration 21: the eraser restores DEFAULT_FACE_COLOR, not literal white,
    // so erased faces match never-painted ones on both canvas themes.
    let updates: Vec<(u32, [u8; 4])> = hits
        .into_iter()
        .map(|fid| (fid, DEFAULT_FACE_COLOR))
        .collect();

    Ok(commit(mesh, stroke_id, updates))
}

#[tauri::command]
pub fn pick_color(face_id: u32, state: State<AppState>) -> Result<[u8; 4], String> {
    log::debug!("[cmd:pick_color] face_id={}", face_id);
    let mesh_guard = state.mesh.lock().map_err(|e| e.to_string())?;
    let mesh = mesh_guard.as_ref().ok_or("No mesh loaded")?;
    if (face_id as usize) < mesh.face_colors.len() {
        Ok(mesh.face_colors[face_id as usize])
    } else {
        Err("Face ID out of range".to_string())
    }
}
