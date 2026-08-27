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

    Ok(apply_fill(mesh, face_id, color, radius))
}

/// Core of [`fill_paint`] without the Tauri wrapper.
///
/// radius > 0 → LOCAL fill: a bounded blob of faces within `radius` of the
/// clicked face center (kdtree query, O(k), no flood-fill). This is the
/// "智能区分" local-fill path: a single/whole-model partition or unsegmented
/// model no longer floods the ENTIRE model (iteration 18, Issue 1).
/// radius == 0 (legacy / Shift+click escape hatch) → whole connected-component
/// flood_fill, constrained by segment labels when any exist.
pub fn apply_fill(
    mesh: &mut crate::mesh::model::MeshModel,
    face_id: u32,
    color: [u8; 4],
    radius: f32,
) -> PaintResult {
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
    commit(mesh, None, updates)
}

#[tauri::command]
pub fn fill_segment_paint(
    segment_id: u32,
    color: [u8; 4],
    state: State<AppState>,
) -> Result<PaintResult, String> {
    let mut mesh_guard = state.mesh.lock().map_err(|e| e.to_string())?;
    let mesh = mesh_guard.as_mut().ok_or("No mesh loaded")?;

    Ok(apply_fill_segment(mesh, segment_id, color))
}

/// Core of [`fill_segment_paint`] without the Tauri wrapper.
///
/// Faces are selected strictly from `segment_labels` — region membership is
/// never inferred from colours. Filling region C with the same colour as
/// region A must leave every face of A byte-identical; that contract is the
/// backend half of the "fill C repainted A" defect report (2026-08-27).
pub fn apply_fill_segment(
    mesh: &mut crate::mesh::model::MeshModel,
    segment_id: u32,
    color: [u8; 4],
) -> PaintResult {
    let faces = segment_faces(mesh, segment_id);
    let updates: Vec<(u32, [u8; 4])> = faces.into_iter().map(|fid| (fid, color)).collect();
    commit(mesh, None, updates)
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

#[cfg(test)]
mod fill_tests {
    use super::*;
    use crate::mesh::model::{MeshModel, DEFAULT_FACE_COLOR};

    const RED: [u8; 4] = [220, 30, 30, 255];

    /// Tetrahedron split into two adjacent regions:
    /// A = faces {0, 1}, C = faces {2, 3}. Both share edges, so any
    /// colour- or adjacency-based bleed between the two shows immediately.
    fn two_region_mesh() -> MeshModel {
        let mut m = MeshModel::new();
        m.vertices = vec![
            [0.0, 0.0, 0.0],
            [1.0, 0.0, 0.0],
            [0.0, 1.0, 0.0],
            [0.0, 0.0, 1.0],
        ];
        m.faces = vec![[0, 1, 2], [0, 2, 3], [0, 3, 1], [1, 3, 2]];
        m.compute_normals();
        m.build_adjacency();
        m.init_default_colors();
        m.segment_labels = vec![1, 1, 2, 2];
        m
    }

    fn colors_of(m: &MeshModel, faces: &[usize]) -> Vec<[u8; 4]> {
        faces.iter().map(|&f| m.face_colors[f]).collect()
    }

    /// Filling a region repaints exactly that region's faces.
    #[test]
    fn fill_segment_targets_only_its_own_region() {
        let mut m = two_region_mesh();
        apply_fill_segment(&mut m, 1, RED);

        assert_eq!(colors_of(&m, &[0, 1]), vec![RED; 2]);
        assert_eq!(
            colors_of(&m, &[2, 3]),
            vec![DEFAULT_FACE_COLOR; 2],
            "the other region must be untouched"
        );
    }

    /// THE defect report (2026-08-27): fill region A red, then fill region C
    /// the SAME red — A must stay byte-identical red, C must become red, and
    /// labels must not move. Colours never feed back into region membership,
    /// so two regions may share one colour without evicting each other.
    #[test]
    fn same_colour_on_two_regions_never_repaints_the_first() {
        let mut m = two_region_mesh();
        apply_fill_segment(&mut m, 1, RED);
        let after_first_fill = colors_of(&m, &[0, 1]);

        apply_fill_segment(&mut m, 2, RED);

        assert_eq!(
            colors_of(&m, &[0, 1]),
            after_first_fill,
            "filling C with the same red must not touch a single face of A"
        );
        assert_eq!(colors_of(&m, &[2, 3]), vec![RED; 2]);
        assert_eq!(m.segment_labels, vec![1, 1, 2, 2], "labels are the region truth");
    }

    /// The Shift+click flood route (radius == 0) is label-constrained: it must
    /// stop at the boundary even when both sides carry the identical colour.
    #[test]
    fn flood_route_stops_at_label_boundary_even_when_colours_match() {
        let mut m = two_region_mesh();
        // Pre-paint EVERYTHING red, as if both regions were already filled.
        for c in m.face_colors.iter_mut() {
            *c = RED;
        }

        let result = apply_fill(&mut m, 0, [10, 200, 10, 255], 0.0);
        assert_eq!(result.updated_faces, vec![0, 1], "flood stays inside label 1");
        assert_eq!(colors_of(&m, &[2, 3]), vec![RED; 2]);
    }
}
