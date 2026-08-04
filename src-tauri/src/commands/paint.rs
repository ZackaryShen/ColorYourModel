use serde::{Deserialize, Serialize};
use tauri::State;

use crate::commands::mesh::AppState;
use crate::mesh::face_colors::mix_color;
use crate::mesh::kdtree::distance;
use crate::mesh::model::DEFAULT_FACE_COLOR;
use crate::paint::brush::{brush_hit, falloff_strength};
use crate::paint::fill::{fill_region, fill_segment};
use crate::paint::smart_snap::smart_brush_hit;
use crate::paint::spray::spray_hit;

/// Result of a paint operation: list of (face_id, [r,g,b,a])
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PaintResult {
    pub updated_faces: Vec<u32>,
    pub updated_colors: Vec<[u8; 4]>,
}

#[tauri::command]
pub fn brush_paint(
    center_face: u32,
    radius: f32,
    strength: f32,
    falloff_mode: String,
    color: [u8; 4],
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
    let mut updated_faces = Vec::new();
    let mut updated_colors = Vec::new();

    for fid in hits {
        let d = distance(&center, &mesh.face_center(fid));
        let s = falloff_strength(d, radius, &falloff_mode) * strength;
        let new_color = mix_color(&mesh.face_colors[fid as usize], &color, s);
        mesh.face_colors[fid as usize] = new_color;
        updated_faces.push(fid);
        updated_colors.push(new_color);
    }

    Ok(PaintResult {
        updated_faces,
        updated_colors,
    })
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
        // Commit to the authoritative backend paint state. `fill_region` does
        // this internally; the local branch must do it explicitly, otherwise the
        // fill would exist only on the GPU and in the frontend store — and 3MF
        // export / undo-restore would silently lose it.
        for &fid in &local {
            mesh.face_colors[fid as usize] = color;
        }
        local
    } else {
        fill_region(mesh, face_id, color)
    };
    let colors = vec![color; faces.len()];

    Ok(PaintResult {
        updated_faces: faces,
        updated_colors: colors,
    })
}

#[tauri::command]
pub fn fill_segment_paint(
    segment_id: u32,
    color: [u8; 4],
    state: State<AppState>,
) -> Result<PaintResult, String> {
    let mut mesh_guard = state.mesh.lock().map_err(|e| e.to_string())?;
    let mesh = mesh_guard.as_mut().ok_or("No mesh loaded")?;

    let faces = fill_segment(mesh, segment_id, color);
    let colors = vec![color; faces.len()];

    Ok(PaintResult {
        updated_faces: faces,
        updated_colors: colors,
    })
}

#[tauri::command]
pub fn spray_paint(
    center_face: u32,
    radius: f32,
    strength: f32,
    color: [u8; 4],
    density: u32,
    state: State<AppState>,
) -> Result<PaintResult, String> {
    let mut mesh_guard = state.mesh.lock().map_err(|e| e.to_string())?;
    let mesh = mesh_guard.as_mut().ok_or("No mesh loaded")?;

    let results = spray_hit(mesh, center_face, radius, strength, &color, density);
    let (faces, colors): (Vec<_>, Vec<_>) = results.into_iter().unzip();

    Ok(PaintResult {
        updated_faces: faces,
        updated_colors: colors,
    })
}

#[tauri::command]
pub fn smart_brush_paint(
    center_face: u32,
    radius: f32,
    strength: f32,
    falloff_mode: String,
    color: [u8; 4],
    state: State<AppState>,
) -> Result<PaintResult, String> {
    let mut mesh_guard = state.mesh.lock().map_err(|e| e.to_string())?;
    let mesh = mesh_guard.as_mut().ok_or("No mesh loaded")?;

    let results = smart_brush_hit(mesh, center_face, radius, strength, &falloff_mode, &color);
    let (faces, colors): (Vec<_>, Vec<_>) = results.into_iter().unzip();

    Ok(PaintResult {
        updated_faces: faces,
        updated_colors: colors,
    })
}

#[tauri::command]
pub fn erase_paint(
    center_face: u32,
    radius: f32,
    state: State<AppState>,
) -> Result<PaintResult, String> {
    let mut mesh_guard = state.mesh.lock().map_err(|e| e.to_string())?;
    let mesh = mesh_guard.as_mut().ok_or("No mesh loaded")?;

    let hits = brush_hit(mesh, center_face, radius);
    // Iteration 21: the eraser restores DEFAULT_FACE_COLOR, not literal white,
    // so erased faces match never-painted ones on both canvas themes.
    let default_color = DEFAULT_FACE_COLOR;
    let mut updated_faces = Vec::new();
    let mut updated_colors = Vec::new();

    for fid in hits {
        mesh.face_colors[fid as usize] = default_color;
        updated_faces.push(fid);
        updated_colors.push(default_color);
    }

    Ok(PaintResult {
        updated_faces,
        updated_colors,
    })
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
