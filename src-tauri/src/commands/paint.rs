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
pub fn project_image_paint(
    path: String,
    axis: String,
    stroke_id: Option<u64>,
    state: State<AppState>,
) -> Result<PaintResult, String> {
    // 0.2.0-P2 req #9: the frontend hands over a file PATH (dialog plugin
    // pattern — adversarial round (e): multi-MB byte arrays over invoke are
    // the exact anti-pattern history.rs:14-26 walked back).
    let bytes = std::fs::read(&path).map_err(|e| format!("project: read image: {e}"))?;
    let img = image::load_from_memory(&bytes)
        .map_err(|e| format!("project: decode image: {e}"))?
        .to_rgba8();
    let axis = crate::paint::projection::parse_axis(&axis)?;
    let mut mesh_guard = state.mesh.lock().map_err(|e| e.to_string())?;
    let mesh = mesh_guard.as_mut().ok_or("No mesh loaded")?;
    let updates = crate::paint::projection::project_image_paint(mesh, &img, axis);
    let _ = axis; // parsed; kept for the log below
    log::info!("[cmd:project_image_paint] faces={}", updates.len());
    Ok(commit(mesh, stroke_id, updates))
}

/// One sample along a gradient path: face index + normalized path position.
#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GradientPathSample {
    pub face: u32,
    pub t: f32,
}

/// 0.2.0-P2 gradient v3: apply the knee-piecewise gradient to an explicit
/// sample list in ONE backend call.
///
/// Why a batch command instead of per-sample brush_paint recolor: (1) the
/// frontend cannot compute absolute colors for falloff-halo faces (it lacks
/// the pre-stroke colors — they live in the history entry); (2) one command =
/// one coalesced undo entry with the SAME stroke_id as the provisional
/// paints, so Ctrl+Z still reverts to the true pre-stroke colors.
///
/// The frontend sends (face, t) pairs — t is the sample's normalized position
/// along the drawn path (arc length / total). Colors are computed here with
/// the same piecewise knee function as the provisional preview.
/// 拐点分段渐变的纯函数（0.2.0-P2 渐变 v3 语义）：
/// t ≤ knee：lerp(colorA, mix50, t/knee)；t > knee：lerp(mix50, colorB, (t−knee)/(1−knee))。
/// mix50 = 50/50 混色（拐点处两段严格连续）。t 与 knee 均在此钳制。
pub(crate) fn gradient_path_color(
    t: f32,
    knee: f32,
    color_a: [u8; 4],
    color_b: [u8; 4],
) -> [u8; 4] {
    let knee = knee.clamp(0.05, 0.95);
    let t = t.clamp(0.0, 1.0);
    let mix50 = [
        ((color_a[0] as f32) * 0.5 + (color_b[0] as f32) * 0.5) as u8,
        ((color_a[1] as f32) * 0.5 + (color_b[1] as f32) * 0.5) as u8,
        ((color_a[2] as f32) * 0.5 + (color_b[2] as f32) * 0.5) as u8,
        255,
    ];
    let lerp_channel = |a: u8, b: u8, k: f32| -> u8 {
        (a as f32 + (b as f32 - a as f32) * k).round().clamp(0.0, 255.0) as u8
    };
    if t <= knee {
        let k = if knee > 0.0 { t / knee } else { 1.0 };
        [
            lerp_channel(color_a[0], mix50[0], k),
            lerp_channel(color_a[1], mix50[1], k),
            lerp_channel(color_a[2], mix50[2], k),
            255,
        ]
    } else {
        let k = if t < 1.0 { (t - knee) / (1.0 - knee) } else { 1.0 };
        [
            lerp_channel(mix50[0], color_b[0], k),
            lerp_channel(mix50[1], color_b[1], k),
            lerp_channel(mix50[2], color_b[2], k),
            255,
        ]
    }
}

#[tauri::command]
pub fn gradient_path_paint(
    stroke_id: Option<u64>,
    knee: f32,
    color_a: [u8; 4],
    color_b: [u8; 4],
    samples: Vec<GradientPathSample>,
    state: State<AppState>,
) -> Result<PaintResult, String> {
    let mut mesh_guard = state.mesh.lock().map_err(|e| e.to_string())?;
    let mesh = mesh_guard.as_mut().ok_or("No mesh loaded")?;

    let mut updates: Vec<(u32, [u8; 4])> = Vec::with_capacity(samples.len());
    for s in &samples {
        updates.push((s.face, gradient_path_color(s.t, knee, color_a, color_b)));
    }
    log::info!(
        "[cmd:gradient_path_paint] knee={} samples={} updates={}",
        knee,
        samples.len(),
        updates.len()
    );
    Ok(commit(mesh, stroke_id, updates))
}

#[tauri::command]
pub fn gradient_radial_paint(
    center_face: u32,
    radius: f32,
    color_inner: [u8; 4],
    color_outer: [u8; 4],
    stroke_id: Option<u64>,
    state: State<AppState>,
) -> Result<PaintResult, String> {
    let mut mesh_guard = state.mesh.lock().map_err(|e| e.to_string())?;
    let mesh = mesh_guard
        .as_mut()
        .ok_or("No mesh loaded")?;
    let updates =
        crate::paint::gradient::gradient_radial_hit(mesh, center_face, radius, color_inner, color_outer);
    log::info!(
        "[cmd:gradient_radial_paint] center={} radius={} faces={}",
        center_face,
        radius,
        updates.len()
    );
    Ok(commit(mesh, stroke_id, updates))
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

/// One row of the colour panel's "used colours" section (issue #23).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UsedColorDto {
    pub color: [u8; 4],
    pub faces: u32,
    pub regions: u32,
    /// True for the never-painted default grey — the panel marks the swatch
    /// so "no colour yet" stays distinguishable from a painted grey.
    pub is_default: bool,
}

/// Every distinct colour currently on the model, most faces first (issue
/// #23). The command is a thin DTO map over `MeshModel::used_colors` — the
/// counting logic and its tests live in the mesh layer.
#[tauri::command]
pub fn used_colors(state: State<AppState>) -> Result<Vec<UsedColorDto>, String> {
    let mesh_guard = state.mesh.lock().map_err(|e| e.to_string())?;
    let mesh = mesh_guard.as_ref().ok_or("No mesh loaded")?;
    Ok(mesh
        .used_colors()
        .into_iter()
        .map(|e| UsedColorDto {
            // Colour-equality only: a user who deliberately paints the exact
            // default grey gets the dashed "default" swatch. Distinguishing
            // painted-grey from never-painted would need per-face provenance
            // the colour buffer does not carry — accepted limitation.
            is_default: e.color == DEFAULT_FACE_COLOR,
            color: e.color,
            faces: e.faces,
            regions: e.regions,
        })
        .collect())
}

/// Result of a colour replace (issue #29): the incremental paint payload
/// (same wire shape as [`PaintResult`], consumed by the GPU bridge) plus the
/// authoritative counts for the applied-status message.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReplaceColorResult {
    pub updated_faces: Vec<u32>,
    pub updated_colors: Vec<[u8; 4]>,
    pub faces: u32,
    pub regions: u32,
}

/// Faces whose exact colour equals `from` (RGBA identity — the same rule as
/// the used-colours list), optionally restricted to `labels`. One O(F) pass;
/// regions are the DISTINCT segment labels among the matches.
fn replace_updates(
    mesh: &crate::mesh::model::MeshModel,
    from: [u8; 4],
    labels: Option<&[u32]>,
) -> (Vec<u32>, u32) {
    let scope = labels.map(std::collections::HashSet::<_>::from_iter);
    let mut faces = Vec::new();
    let mut regions = std::collections::HashSet::new();
    for (f, &c) in mesh.face_colors.iter().enumerate() {
        if c != from {
            continue;
        }
        if let Some(scope) = &scope {
            let label = mesh.segment_labels[f];
            if !scope.contains(&label) {
                continue;
            }
        }
        faces.push(f as u32);
        regions.insert(mesh.segment_labels[f]);
    }
    (faces, regions.len() as u32)
}

/// Pure core of `replace_face_color` (issue #29) — colours only: labels,
/// names and segmentation are never touched. All matched faces go through
/// `apply_paint` with `stroke_id: None`, so the whole replace lands as
/// exactly ONE history entry; a no-match replace records NOTHING (an empty
/// undo step would be noise the user has to step over).
pub fn apply_replace_face_color(
    mesh: &mut crate::mesh::model::MeshModel,
    from: [u8; 4],
    to: [u8; 4],
    labels: Option<&[u32]>,
) -> ReplaceColorResult {
    if from == to || from == DEFAULT_FACE_COLOR {
        // Same no-op/refusal contract as the command wrapper (tests exercise
        // the core directly); see `replace_face_color` for the rationale.
        return ReplaceColorResult {
            updated_faces: Vec::new(),
            updated_colors: Vec::new(),
            faces: 0,
            regions: 0,
        };
    }
    let (faces, region_count) = replace_updates(mesh, from, labels);
    if faces.is_empty() {
        return ReplaceColorResult {
            updated_faces: Vec::new(),
            updated_colors: Vec::new(),
            faces: 0,
            regions: 0,
        };
    }
    let updates: Vec<(u32, [u8; 4])> = faces.iter().map(|&f| (f, to)).collect();
    let result = commit(mesh, None, updates);
    ReplaceColorResult {
        updated_faces: result.updated_faces,
        updated_colors: result.updated_colors,
        faces: faces.len() as u32,
        regions: region_count,
    }
}

/// Apply the replace: one O(F) pass, exactly one history entry (issue #29).
/// The guard rails are enforced here TOO, not only in the UI (backend
/// authority): `from == to` is a mechanical no-op returning an empty result,
/// and a whole-model repaint FROM the never-painted default grey is refused —
/// it would silently recolour every untouched face.
#[tauri::command]
pub fn replace_face_color(
    from: [u8; 4],
    to: [u8; 4],
    labels: Option<Vec<u32>>,
    state: State<AppState>,
) -> Result<ReplaceColorResult, String> {
    if from == to {
        return Ok(ReplaceColorResult {
            updated_faces: Vec::new(),
            updated_colors: Vec::new(),
            faces: 0,
            regions: 0,
        });
    }
    if from == DEFAULT_FACE_COLOR {
        return Err(
            "replace from the unpainted default grey is refused: it would repaint every untouched face"
                .to_string(),
        );
    }
    let mut mesh_guard = state.mesh.lock().map_err(|e| e.to_string())?;
    let mesh = mesh_guard.as_mut().ok_or("No mesh loaded")?;
    let result = apply_replace_face_color(mesh, from, to, labels.as_deref());
    log::info!(
        "[cmd:replace_face_color] faces={} regions={}",
        result.faces,
        result.regions
    );
    Ok(result)
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

#[cfg(test)]
mod replace_colour_tests {
    //! Issue #29: colour find & replace. The whole-model / scoped replace
    //! must be a single O(F) pass that lands as exactly ONE history entry
    //! (one Ctrl+Z restores every replaced face), never touches labels, and
    //! records nothing when nothing matches.

    use super::*;
    use crate::mesh::model::{MeshModel, DEFAULT_FACE_COLOR};

    const RED: [u8; 4] = [139, 69, 19, 255];
    const GREEN: [u8; 4] = [40, 167, 69, 255];

    /// Tetrahedron, two regions: label 1 = faces {0, 1}, label 2 = {2, 3}.
    /// Faces 0 and 2 are pre-painted RED, face 1 BLUE, face 3 default grey.
    fn replace_mesh() -> MeshModel {
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
        m.face_colors[0] = RED;
        m.face_colors[2] = RED;
        m
    }

    #[test]
    fn whole_model_replace_is_one_history_step() {
        let mut m = replace_mesh();
        let before = m.face_colors.clone();

        let result = apply_replace_face_color(&mut m, RED, GREEN, None);

        assert_eq!(result.updated_faces, vec![0, 2]);
        assert_eq!(result.faces, 2);
        assert_eq!(result.regions, 2, "faces 0 and 2 sit in different regions");
        assert_eq!(m.face_colors[0], GREEN);
        assert_eq!(m.face_colors[2], GREEN);
        // Exactly one undo step — not one per face.
        assert_eq!(m.history.undo_depth(), 1);
        // One Ctrl+Z (jump back to pristine) restores every face exactly —
        // the split field borrow mirrors how commands/history.rs drives it.
        {
            let crate::mesh::model::MeshModel { history, face_colors, segment_labels, vertices, .. } = &mut m;
            history.jump_to(0, face_colors, segment_labels, vertices);
        }
        assert_eq!(m.face_colors[0], RED, "undo restores the original colour");
        assert_eq!(m.face_colors[2], RED);
        assert_eq!(m.face_colors[1], before[1]);
    }

    #[test]
    fn scoped_replace_touches_only_requested_labels() {
        let mut m = replace_mesh();

        let result = apply_replace_face_color(&mut m, RED, GREEN, Some(&[2]));

        assert_eq!(result.updated_faces, vec![2]);
        assert_eq!(result.faces, 1);
        assert_eq!(result.regions, 1);
        assert_eq!(m.face_colors[0], RED, "label 1's red face is out of scope");
        assert_eq!(m.face_colors[2], GREEN);
    }

    #[test]
    fn no_match_records_no_history() {
        let mut m = replace_mesh();
        let absent: [u8; 4] = [1, 2, 3, 255];

        let result = apply_replace_face_color(&mut m, absent, GREEN, None);

        assert_eq!(result.updated_faces.len(), 0);
        assert_eq!(result.faces, 0);
        assert_eq!(result.regions, 0);
        // A no-op must not create an undo step the user has to step over.
        assert_eq!(m.history.undo_depth(), 0);
    }

    #[test]
    fn replace_keeps_labels_and_segmentation_intact() {
        let mut m = replace_mesh();
        let labels_before = m.segment_labels.clone();

        apply_replace_face_color(&mut m, RED, GREEN, None);

        assert_eq!(m.segment_labels, labels_before, "replace is a paint edit, not a segmentation edit");
    }

    #[test]
    fn replace_updates_counts_faces_and_regions() {
        let m = replace_mesh();

        let (faces, regions) = replace_updates(&m, RED, None);
        assert_eq!((faces.len(), regions as usize), (2, 2));

        let (faces, regions) = replace_updates(&m, RED, Some(&[2]));
        assert_eq!((faces.len(), regions as usize), (1, 1));

        // Faces 1 and 3 were never painted, so they still carry the default
        // grey — countable (the guard refuses that replace, not the math).
        let (faces, regions) = replace_updates(&m, DEFAULT_FACE_COLOR, None);
        assert_eq!((faces.len(), regions as usize), (2, 2));
    }

    #[test]
    fn core_refuses_default_source_and_same_colour() {
        let mut m = replace_mesh();

        // from == to: mechanical no-op, empty result.
        let result = apply_replace_face_color(&mut m, RED, RED, None);
        assert_eq!(result.faces, 0);

        // Default grey as source: refused by the same contract the UI guard
        // implements — the IPC surface does not trust frontend state.
        let result = apply_replace_face_color(&mut m, DEFAULT_FACE_COLOR, GREEN, None);
        assert_eq!(result.faces, 0);
        assert_eq!(m.history.undo_depth(), 0);
    }
}
