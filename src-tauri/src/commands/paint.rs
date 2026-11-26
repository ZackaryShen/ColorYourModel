use serde::{Deserialize, Serialize};
use tauri::State;

use crate::commands::mesh::AppState;
use crate::mesh::face_colors::mix_color;
use crate::mesh::history::{GradientContext, GradientTool};
use crate::mesh::kdtree::distance;
use crate::mesh::model::{GradientSample, MeshModel, DEFAULT_FACE_COLOR};
use crate::paint::brush::{brush_hit, falloff_strength};
use crate::paint::fill::{region_faces, segment_faces};
use crate::paint::gradient::{self, GradientHit};
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
    source: Option<[u8; 4]>,
) -> PaintResult {
    // A visible no-op can still change the coverage of a textured brush source.
    let tracks_coverage = mesh.texture_painting && source.is_some();
    let updates: Vec<_> = updates
        .into_iter()
        .filter(|(face, color)| {
            mesh.face_colors.get(*face as usize).is_some_and(|old| {
                old != color || tracks_coverage || mesh.gradient_faces.contains_key(face)
            })
        })
        .collect();
    mesh.apply_paint(stroke_id, &updates);
    mesh.record_paint_sources(source, &updates);
    let (updated_faces, updated_colors) = updates.into_iter().unzip();
    PaintResult {
        updated_faces,
        updated_colors,
    }
}

fn commit_dabs(
    mesh: &mut crate::mesh::model::MeshModel,
    stroke_id: Option<u64>,
    mut dabs: Vec<(u32, f32)>,
    color: [u8; 4],
) -> PaintResult {
    // A zero-coverage step dab must leave the current paint and its source intact.
    dabs.retain(|&(_, strength)| strength.is_finite() && strength > 0.0);
    let updates = dabs
        .iter()
        .map(|&(face, strength)| {
            let painted = mix_color(&mesh.face_colors[face as usize], &color, strength);
            (face, painted)
        })
        .collect();
    mesh.record_texture_dabs(color, dabs);
    commit(mesh, stroke_id, updates, Some(color))
}

#[tauri::command]
pub fn brush_paint(
    center_face: u32,
    radius: f32,
    strength: f32,
    falloff_mode: String,
    color: [u8; 4],
    stroke_id: Option<u64>,
    tab_id: u64,
    state: State<AppState>,
) -> Result<PaintResult, String> {
    let mut mesh_guard = state.lock_mesh(tab_id)?;
    let mesh = mesh_guard.as_mut().ok_or("No mesh loaded")?;

    log::info!(
        "[cmd:brush_paint] center_face={}, radius={:.2}, color={:?}",
        center_face,
        radius,
        color
    );

    // [DIAG-B] Log vertex indices for cross-check with frontend [DIAG-A]
    if (center_face as usize) < mesh.faces.len() {
        let f = &mesh.faces[center_face as usize];
        log::info!(
            "[DIAG-B] center_face={} verts=[{},{},{}] face_colors_len={}",
            center_face,
            f[0],
            f[1],
            f[2],
            mesh.face_colors.len()
        );
    } else {
        log::error!(
            "[DIAG-B] center_face={} OUT OF RANGE (faces.len={})",
            center_face,
            mesh.faces.len()
        );
    }

    let hidden = hidden_snapshot(mesh);
    let hits = cull_hidden(mesh, brush_hit(mesh, center_face, radius), &hidden);
    let center = mesh.face_center(center_face);

    let dabs = hits
        .into_iter()
        .map(|fid| {
            let d = distance(&center, &mesh.face_center(fid));
            let s = falloff_strength(d, radius, &falloff_mode) * strength;
            (fid, s)
        })
        .collect();

    let result = commit_dabs(mesh, stroke_id, dabs, color);
    mesh_guard.commit_if(!result.updated_faces.is_empty());
    Ok(result)
}

#[tauri::command]
pub fn fill_paint(
    face_id: u32,
    color: [u8; 4],
    radius: f32,
    tab_id: u64,
    state: State<AppState>,
) -> Result<PaintResult, String> {
    let mut mesh_guard = state.lock_mesh(tab_id)?;
    let mesh = mesh_guard.as_mut().ok_or("No mesh loaded")?;

    log::info!(
        "[cmd:fill_paint] face_id={}, color={:?}, radius={:.2}",
        face_id,
        color,
        radius
    );

    let hidden = hidden_snapshot(mesh);
    let result = apply_fill(mesh, face_id, color, radius, &hidden);
    mesh_guard.commit_if(!result.updated_faces.is_empty());
    Ok(result)
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
    hidden: &std::collections::HashSet<u32>,
) -> PaintResult {
    let faces = if radius > 0.0 {
        cull_hidden(mesh, local_fill_faces(mesh, face_id, radius), hidden)
    } else {
        region_faces(mesh, face_id)
    };

    // A fill is a single click, never a drag, so it can never coalesce with
    // anything: `None`.
    let updates: Vec<(u32, [u8; 4])> = faces.into_iter().map(|fid| (fid, color)).collect();
    commit(mesh, None, updates, None)
}

fn local_fill_faces(mesh: &MeshModel, face_id: u32, radius: f32) -> Vec<u32> {
    let center = mesh.face_center(face_id);
    mesh.faces_within_radius(&center, radius)
        .into_iter()
        .filter(|&face| distance(&center, &mesh.face_center(face)) < radius)
        .collect()
}

fn commit_gradient(
    mesh: &mut MeshModel,
    mut updates: Vec<(u32, GradientSample)>,
    context: GradientContext,
) -> PaintResult {
    updates.retain(|(face, sample)| {
        mesh.gradient_faces.get(face).is_none_or(|previous| {
            let candidate = GradientSample { ramp_id: previous.ramp_id, ..*sample };
            candidate != *previous
                || mesh.gradient_ramp(previous.ramp_id) != Some(context.stops.as_ref())
        })
    });
    updates.sort_unstable_by_key(|&(face, _)| face);
    mesh.apply_gradient(&updates, context);
    let (updated_faces, updated_colors) = updates
        .into_iter()
        .map(|(face, _)| (face, mesh.face_colors[face as usize]))
        .unzip();
    PaintResult {
        updated_faces,
        updated_colors,
    }
}

fn apply_gradient_brush(
    mesh: &mut MeshModel,
    points: &[GradientHit],
    stops: &[[u8; 4]],
    radius: f32,
) -> Result<PaintResult, String> {
    gradient::validate(mesh, stops, radius)?;
    let updates = gradient::brush_samples(mesh, points, stops, radius)?;
    Ok(commit_gradient(
        mesh,
        updates,
        GradientContext {
            tool: GradientTool::Brush,
            stops: stops.into(),
            points: points.into(),
            radius,
            whole_region: false,
        },
    ))
}

#[tauri::command]
pub fn gradient_brush_paint(
    points: Vec<GradientHit>,
    stops: Vec<[u8; 4]>,
    radius: f32,
    tab_id: u64,
    state: State<AppState>,
) -> Result<PaintResult, String> {
    let mut mesh_guard = state.lock_mesh(tab_id)?;
    let mesh = mesh_guard.as_mut().ok_or("No mesh loaded")?;
    let result = apply_gradient_brush(mesh, &points, &stops, radius)?;
    mesh_guard.commit_if(!result.updated_faces.is_empty());
    Ok(result)
}

fn apply_gradient_fill(
    mesh: &mut MeshModel,
    start: GradientHit,
    end: GradientHit,
    stops: &[[u8; 4]],
    radius: f32,
    whole_region: bool,
) -> Result<PaintResult, String> {
    gradient::validate(mesh, stops, radius)?;
    let start_point = gradient::hit_point(mesh, start)?;
    let end_point = gradient::hit_point(mesh, end)?;
    let segment = mesh
        .segment_labels
        .get(start.face_id as usize)
        .filter(|label| mesh.segments.contains_key(label));
    let faces = if whole_region {
        region_faces(mesh, start.face_id)
    } else if let Some(&segment) = segment {
        segment_faces(mesh, segment)
    } else {
        local_fill_faces(mesh, start.face_id, radius)
    };
    let faces = cull_hidden(mesh, faces, &mesh.hidden_labels);
    let updates = gradient::fill_samples(mesh, faces, start_point, end_point, stops)?;
    Ok(commit_gradient(
        mesh,
        updates,
        GradientContext {
            tool: GradientTool::Fill,
            stops: stops.into(),
            points: Box::new([start, end]),
            radius,
            whole_region,
        },
    ))
}

#[tauri::command]
pub fn gradient_fill_paint(
    start: GradientHit,
    end: GradientHit,
    stops: Vec<[u8; 4]>,
    radius: f32,
    whole_region: bool,
    tab_id: u64,
    state: State<AppState>,
) -> Result<PaintResult, String> {
    let mut mesh_guard = state.lock_mesh(tab_id)?;
    let mesh = mesh_guard.as_mut().ok_or("No mesh loaded")?;
    let result = apply_gradient_fill(mesh, start, end, &stops, radius, whole_region)?;
    mesh_guard.commit_if(!result.updated_faces.is_empty());
    Ok(result)
}

#[tauri::command]
pub fn fill_segment_paint(
    segment_id: u32,
    color: [u8; 4],
    tab_id: u64,
    state: State<AppState>,
) -> Result<PaintResult, String> {
    let mut mesh_guard = state.lock_mesh(tab_id)?;
    let mesh = mesh_guard.as_mut().ok_or("No mesh loaded")?;

    let result = apply_fill_segment(mesh, segment_id, color);
    mesh_guard.commit_if(!result.updated_faces.is_empty());
    Ok(result)
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
    commit(mesh, None, updates, None)
}

#[tauri::command]
pub fn spray_paint(
    center_face: u32,
    radius: f32,
    strength: f32,
    color: [u8; 4],
    density: u32,
    stroke_id: Option<u64>,
    tab_id: u64,
    state: State<AppState>,
) -> Result<PaintResult, String> {
    let mut mesh_guard = state.lock_mesh(tab_id)?;
    let mesh = mesh_guard.as_mut().ok_or("No mesh loaded")?;

    let hidden = hidden_snapshot(mesh);
    let dabs = cull_hidden_pairs(
        mesh,
        spray_hit(mesh, center_face, radius, strength, density),
        &hidden,
    );
    let result = commit_dabs(mesh, stroke_id, dabs, color);
    mesh_guard.commit_if(!result.updated_faces.is_empty());
    Ok(result)
}

#[tauri::command]
pub fn smart_brush_paint(
    center_face: u32,
    radius: f32,
    strength: f32,
    falloff_mode: String,
    color: [u8; 4],
    stroke_id: Option<u64>,
    tab_id: u64,
    state: State<AppState>,
) -> Result<PaintResult, String> {
    let mut mesh_guard = state.lock_mesh(tab_id)?;
    let mesh = mesh_guard.as_mut().ok_or("No mesh loaded")?;

    let dabs = smart_brush_hit(mesh, center_face, radius, strength, &falloff_mode);
    let result = commit_dabs(mesh, stroke_id, dabs, color);
    mesh_guard.commit_if(!result.updated_faces.is_empty());
    Ok(result)
}

#[tauri::command]
pub fn erase_paint(
    center_face: u32,
    radius: f32,
    stroke_id: Option<u64>,
    tab_id: u64,
    state: State<AppState>,
) -> Result<PaintResult, String> {
    let mut mesh_guard = state.lock_mesh(tab_id)?;
    let mesh = mesh_guard.as_mut().ok_or("No mesh loaded")?;

    let hidden = hidden_snapshot(mesh);
    let hits = cull_hidden(mesh, brush_hit(mesh, center_face, radius), &hidden);
    // Iteration 21: the eraser restores DEFAULT_FACE_COLOR, not literal white,
    // so erased faces match never-painted ones on both canvas themes.
    let updates: Vec<(u32, [u8; 4])> = hits
        .into_iter()
        .map(|fid| (fid, DEFAULT_FACE_COLOR))
        .collect();

    let result = commit(mesh, stroke_id, updates, None);
    mesh_guard.commit_if(!result.updated_faces.is_empty());
    Ok(result)
}

/// Drop faces whose segment label is in `hidden` — the backend half of the
/// hidden-region view state (#24/#29): implicit spatial tools (brush / spray
/// / eraser / radius fills / radial gradients) must not touch faces the user
/// has hidden, even when the dab's radius sweeps over them. The hidden set
/// lives in [`AppState::hidden_labels`], synced from the frontend whenever it
/// changes; EXPLICIT list-based actions (multi-select fill, colour replace)
/// deliberately do NOT cull — hiding is display-only there.
fn cull_hidden(
    mesh: &crate::mesh::model::MeshModel,
    faces: Vec<u32>,
    hidden: &std::collections::HashSet<u32>,
) -> Vec<u32> {
    if hidden.is_empty() {
        return faces;
    }
    faces
        .into_iter()
        .filter(|f| !hidden.contains(&mesh.segment_labels[*f as usize]))
        .collect()
}

/// Same cull for pre-built (face, payload) update lists.
fn cull_hidden_pairs<T>(
    mesh: &crate::mesh::model::MeshModel,
    updates: Vec<(u32, T)>,
    hidden: &std::collections::HashSet<u32>,
) -> Vec<(u32, T)> {
    if hidden.is_empty() {
        return updates;
    }
    updates
        .into_iter()
        .filter(|(f, _)| !hidden.contains(&mesh.segment_labels[*f as usize]))
        .collect()
}

/// Snapshot of the hidden label set (empty = nothing hidden).
fn hidden_snapshot(mesh: &crate::mesh::model::MeshModel) -> std::collections::HashSet<u32> {
    mesh.hidden_labels.clone()
}

/// Core of `fill_segments_paint` (owner scenario on #29): paint EVERY face
/// of the given labels with `color` in a single commit — N regions, exactly
/// ONE history entry, one Ctrl+Z restores all of them. Duplicate/unknown
/// labels are harmless (a face is emitted once; absent labels match nothing).
/// A no-match call records nothing.
pub fn apply_fill_segments(
    mesh: &mut crate::mesh::model::MeshModel,
    labels: &[u32],
    color: [u8; 4],
) -> PaintResult {
    let wanted: std::collections::HashSet<u32> = labels.iter().copied().collect();
    let updates: Vec<(u32, [u8; 4])> = mesh
        .segment_labels
        .iter()
        .enumerate()
        .filter(|(_, l)| wanted.contains(l))
        .map(|(i, _)| (i as u32, color))
        .collect();
    if updates.is_empty() {
        return PaintResult {
            updated_faces: Vec::new(),
            updated_colors: Vec::new(),
        };
    }
    commit(mesh, None, updates, None)
}

/// Fill N regions in one shot (multi-select fill, #29): one IPC, one
/// history entry. The viewport drives this from the unified multi-selection
/// card; the labels arrive from the picked set.
#[tauri::command]
pub fn fill_segments_paint(
    labels: Vec<u32>,
    color: [u8; 4],
    tab_id: u64,
    state: State<AppState>,
) -> Result<PaintResult, String> {
    let mut mesh_guard = state.lock_mesh(tab_id)?;
    let mesh = mesh_guard.as_mut().ok_or("No mesh loaded")?;
    let result = apply_fill_segments(mesh, &labels, color);
    log::info!(
        "[cmd:fill_segments_paint] labels={} faces={}",
        labels.len(),
        result.updated_faces.len()
    );
    mesh_guard.commit_if(!result.updated_faces.is_empty());
    Ok(result)
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

#[derive(Debug, Clone, serde::Serialize)]
#[serde(untagged)]
pub enum PaintSourceGroup {
    Solid { source: [u8; 4], faces: u32 },
    Gradient { stops: Vec<[u8; 4]>, faces: usize },
}

/// The colours and ramps the user painted with, grouped by live provenance.
#[tauri::command]
pub async fn paint_source_groups(
    tab_id: u64,
    state: State<'_, AppState>,
) -> Result<Vec<PaintSourceGroup>, String> {
    let state = AppState {
        workspace: std::sync::Arc::clone(&state.workspace),
    };
    tauri::async_runtime::spawn_blocking(move || {
        let mesh_guard = state.lock_mesh(tab_id)?;
        let mesh = mesh_guard.as_ref().ok_or("No mesh loaded")?;
        Ok(mesh
            .paint_source_groups()
            .into_iter()
            .map(|(source, faces)| PaintSourceGroup::Solid { source, faces })
            .chain(mesh.gradient_paint_groups().into_iter().map(|(stops, faces)| {
                PaintSourceGroup::Gradient { stops, faces: faces.len() }
            }))
            .collect())
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Replace every live face painted with `from` by `to` in ONE undo step
/// (issue #29 governance acceptance: "repaint the whole stroke at once").
/// Returns the replaced face count; 0 = nothing live (no history entry).
#[tauri::command]
pub fn replace_paint_source(
    from: [u8; 4],
    to: [u8; 4],
    tab_id: u64,
    state: State<AppState>,
) -> Result<PaintResult, String> {
    let mut mesh_guard = state.lock_mesh(tab_id)?;
    let mesh = mesh_guard.as_mut().ok_or("No mesh loaded")?;
    if from == to {
        return Ok(PaintResult {
            updated_faces: Vec::new(),
            updated_colors: Vec::new(),
        });
    }
    // Returns the FULL face/colour payload: the frontend feeds it to the GPU
    // bridge — without it the backend flips to the new colour while the
    // canvas never does (acceptance round 4 feedback #2).
    let updates = mesh.replace_paint_source(from, to);
    let changed = !updates.is_empty();
    let (updated_faces, updated_colors) = updates.into_iter().unzip();
    mesh_guard.commit_if(changed);
    Ok(PaintResult {
        updated_faces,
        updated_colors,
    })
}

/// Repaint with the original per-face coverage to preserve the stroke gradient.
/// Texture projects track actual underpaint. STL keeps the legacy gray-base
/// strength estimate.
#[tauri::command]
pub fn replace_paint_source_shaped(
    from: [u8; 4],
    to: [u8; 4],
    tab_id: u64,
    state: State<AppState>,
) -> Result<PaintResult, String> {
    let mut mesh_guard = state.lock_mesh(tab_id)?;
    let mesh = mesh_guard.as_mut().ok_or("No mesh loaded")?;
    let result = apply_replace_paint_source_shaped(mesh, from, to);
    mesh_guard.commit_if(!result.updated_faces.is_empty());
    Ok(result)
}

fn apply_replace_paint_source_shaped(
    mesh: &mut crate::mesh::model::MeshModel,
    from: [u8; 4],
    to: [u8; 4],
) -> PaintResult {
    if from == to {
        return PaintResult {
            updated_faces: Vec::new(),
            updated_colors: Vec::new(),
        };
    }
    let faces: Vec<u32> = mesh.paint_source_faces(from);
    if faces.is_empty() {
        return PaintResult {
            updated_faces: Vec::new(),
            updated_colors: Vec::new(),
        };
    }
    let updates = if mesh.texture_painting {
        faces
            .into_iter()
            .map(|face| (face, mesh.recolour_texture_dab(face, to)))
            .collect()
    } else {
        let base = DEFAULT_FACE_COLOR;
        // Channels close to the gray base cannot recover coverage reliably.
        let deltas = [0, 1, 2].map(|c| {
            let delta = from[c] as f32 - base[c] as f32;
            if delta.abs() > 24.0 {
                delta
            } else {
                0.0
            }
        });
        let denominator: f32 = deltas.iter().map(|delta| delta * delta).sum();
        faces
            .into_iter()
            .map(|face| {
                let painted = mesh.face_colors[face as usize];
                let coverage = if denominator > f32::EPSILON {
                    let numerator: f32 = deltas
                        .iter()
                        .enumerate()
                        .map(|(c, delta)| (painted[c] as f32 - base[c] as f32) * delta)
                        .sum();
                    (numerator / denominator).clamp(0.0, 1.0)
                } else {
                    1.0
                };
                (face, mix_color(&base, &to, coverage))
            })
            .collect()
    };
    commit(mesh, None, updates, Some(to))
}

/// Live faces painted with `source` — hover-highlight payload for the
/// 绘制颜色 swatches (issue #29 governance, acceptance round 4 feedback #4).
#[tauri::command]
pub fn paint_source_faces(
    source: Option<[u8; 4]>,
    stops: Option<Vec<[u8; 4]>>,
    tab_id: u64,
    state: State<AppState>,
) -> Result<Vec<u32>, String> {
    let mesh_guard = state.lock_mesh(tab_id)?;
    let mesh = mesh_guard.as_ref().ok_or("No mesh loaded")?;
    match (source, stops) {
        (Some(source), None) => Ok(mesh.paint_source_faces(source)),
        (None, Some(stops)) => Ok(mesh.gradient_paint_groups().remove(&stops).unwrap_or_default()),
        _ => Err("Choose one paint source or gradient ramp".into()),
    }
}

/// Every distinct colour currently on the model, most faces first (issue
/// #23). The command is a thin DTO map over `MeshModel::used_colors` — the
/// counting logic and its tests live in the mesh layer.
#[tauri::command]
pub async fn used_colors(
    tab_id: u64,
    state: State<'_, AppState>,
) -> Result<Vec<UsedColorDto>, String> {
    let state = AppState {
        workspace: std::sync::Arc::clone(&state.workspace),
    };
    tauri::async_runtime::spawn_blocking(move || {
        let mesh_guard = state.lock_mesh(tab_id)?;
        let mesh = mesh_guard.as_ref().ok_or("No mesh loaded")?;
        Ok(mesh
            .used_colors()
            .into_iter()
            .map(|e| UsedColorDto {
                is_default: is_unpainted_colour(mesh, e.color),
                color: e.color,
                faces: e.faces,
                regions: e.regions,
            })
            .collect())
    })
    .await
    .map_err(|e| e.to_string())?
}

fn is_unpainted_colour(mesh: &crate::mesh::model::MeshModel, rgba: [u8; 4]) -> bool {
    // A baked texture can deliberately contain the exact STL placeholder gray.
    rgba == DEFAULT_FACE_COLOR && !mesh.texture_painting
}

/// Sync the hidden-region view state from the frontend (#24/#29). The set
/// lives here so IMPLICIT spatial tools (brush / spray / eraser / radius
/// fills / radial gradients) can cull hidden faces before committing;
/// replaces the whole set on every call (empty = nothing hidden). Explicit
/// list-based actions (multi-select fill, colour replace) ignore it.
#[tauri::command]
pub fn set_hidden_labels(
    labels: Vec<u32>,
    tab_id: u64,
    state: State<AppState>,
) -> Result<(), String> {
    let mut ws = state.workspace.lock().map_err(|e| e.to_string())?;
    let index = ws.validate_active(tab_id)?;
    let mesh = ws.tabs[index].mesh.as_mut().ok_or("No mesh loaded")?;
    mesh.hidden_labels = labels.into_iter().collect();
    Ok(())
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
/// the used-colours list). One O(F) pass over the WHOLE model — the replace
/// semantic is "every region that used the colour" (owner decision on
/// #29, the scoped variant was removed); regions are the DISTINCT segment
/// labels among the matches.
fn replace_updates(mesh: &crate::mesh::model::MeshModel, from: [u8; 4]) -> (Vec<u32>, u32) {
    let mut faces = Vec::new();
    let mut regions = std::collections::HashSet::new();
    for (f, &c) in mesh.face_colors.iter().enumerate() {
        if c != from {
            continue;
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
) -> ReplaceColorResult {
    if from == to || is_unpainted_colour(mesh, from) {
        // Same no-op/refusal contract as the command wrapper (tests exercise
        // the core directly); see `replace_face_color` for the rationale.
        return ReplaceColorResult {
            updated_faces: Vec::new(),
            updated_colors: Vec::new(),
            faces: 0,
            regions: 0,
        };
    }
    let (faces, region_count) = replace_updates(mesh, from);
    if faces.is_empty() {
        return ReplaceColorResult {
            updated_faces: Vec::new(),
            updated_colors: Vec::new(),
            faces: 0,
            regions: 0,
        };
    }
    let updates: Vec<(u32, [u8; 4])> = faces.iter().map(|&f| (f, to)).collect();
    let result = commit(mesh, None, updates, None);
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
    tab_id: u64,
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
    let mut mesh_guard = state.lock_mesh(tab_id)?;
    let mesh = mesh_guard.as_mut().ok_or("No mesh loaded")?;
    if is_unpainted_colour(mesh, from) {
        return Err(
            "replace from the unpainted default grey is refused: it would repaint every untouched face"
                .to_string(),
        );
    }
    let result = apply_replace_face_color(mesh, from, to);
    log::info!(
        "[cmd:replace_face_color] faces={} regions={}",
        result.faces,
        result.regions
    );
    mesh_guard.commit_if(!result.updated_faces.is_empty());
    Ok(result)
}

#[tauri::command]
pub fn pick_color(face_id: u32, tab_id: u64, state: State<AppState>) -> Result<[u8; 4], String> {
    log::debug!("[cmd:pick_color] face_id={}", face_id);
    let mesh_guard = state.lock_mesh(tab_id)?;
    let mesh = mesh_guard.as_ref().ok_or("No mesh loaded")?;
    if (face_id as usize) < mesh.face_colors.len() {
        Ok(mesh.face_colors[face_id as usize])
    } else {
        Err("Face ID out of range".to_string())
    }
}

#[cfg(test)]
mod shaped_repaint_tests {
    use super::*;
    use crate::mesh::model::MeshModel;

    const BLACK: [u8; 4] = [0, 0, 0, 255];
    const WHITE: [u8; 4] = [255; 4];

    fn textured_face(base: [u8; 4]) -> MeshModel {
        let mut mesh = MeshModel::new();
        mesh.texture_painting = true;
        mesh.vertices = vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]];
        mesh.faces = vec![[0, 1, 2]];
        mesh.face_colors = vec![base];
        mesh.segment_labels = vec![0];
        mesh.compute_normals();
        mesh.build_kdtree();
        mesh
    }

    fn dab(mesh: &mut MeshModel, color: [u8; 4], strength: f32) {
        commit_dabs(mesh, None, vec![(0, strength)], color);
    }

    #[test]
    fn texture_repaint_uses_the_black_face_under_the_white_brush() {
        let mut mesh = textured_face(BLACK);
        dab(&mut mesh, WHITE, 0.5);
        assert_eq!(mesh.face_colors, [[127, 127, 127, 255]]);
        let repaint = apply_replace_paint_source_shaped(&mut mesh, WHITE, BLACK);
        assert_eq!(repaint.updated_colors, [BLACK]);
        assert_eq!(mesh.face_colors, [BLACK]);
    }

    #[test]
    fn texture_repaint_preserves_two_passes_over_the_same_colour() {
        let mut mesh = textured_face(WHITE);
        dab(&mut mesh, WHITE, 0.5);
        dab(&mut mesh, WHITE, 0.5);
        assert_eq!(mesh.face_colors, [WHITE]);

        let repaint = apply_replace_paint_source_shaped(&mut mesh, WHITE, BLACK);
        assert_eq!(repaint.updated_colors, [[63, 63, 63, 255]]);
        let repaint_again = apply_replace_paint_source_shaped(&mut mesh, BLACK, [120, 40, 0, 255]);
        assert_eq!(repaint_again.updated_colors, [[153, 93, 63, 255]]);
    }

    #[test]
    fn different_source_blends_over_the_latest_paint_and_is_undoable() {
        let mut mesh = textured_face(BLACK);
        let red = [255, 0, 0, 255];
        let blue = [0, 0, 255, 255];
        dab(&mut mesh, WHITE, 0.5);
        dab(&mut mesh, red, 0.5);
        let before = mesh.face_colors.clone();
        assert_eq!(before, [[191, 63, 63, 255]]);
        let repaint = apply_replace_paint_source_shaped(&mut mesh, red, blue);
        assert_eq!(repaint.updated_colors, [[63, 63, 191, 255]]);
        mesh.history
            .undo(&mut mesh.face_colors, &mut mesh.segment_labels);
        assert_eq!(mesh.face_colors, before);
    }

    #[test]
    fn zero_strength_step_dab_does_not_steal_the_texture_paint_source() {
        let mut mesh = textured_face(BLACK);
        dab(&mut mesh, WHITE, 0.5);
        let depths = mesh.history.undo_depth();
        dab(
            &mut mesh,
            [0, 255, 0, 255],
            falloff_strength(0.75, 1.0, "step"),
        );
        assert_eq!(mesh.history.undo_depth(), depths);
        assert_eq!(mesh.paint_source_faces(WHITE), [0]);
        let repaint = apply_replace_paint_source_shaped(&mut mesh, WHITE, BLACK);
        assert_eq!(repaint.updated_colors, [BLACK]);
    }

    #[test]
    fn spray_and_smart_brush_provide_coverage_of_same_colour_dabs() {
        let mut spray = textured_face(WHITE);
        let dabs = spray_hit(&spray, 0, 1.0, 0.5, 1);
        let [(_, strength)] = dabs.as_slice() else {
            panic!("one face in the spray")
        };
        let expected = mix_color(&WHITE, &BLACK, *strength);
        commit_dabs(&mut spray, None, dabs, WHITE);
        assert_eq!(
            apply_replace_paint_source_shaped(&mut spray, WHITE, BLACK).updated_colors,
            [expected]
        );

        let mut smart = textured_face(WHITE);
        let dabs = smart_brush_hit(&smart, 0, 1.0, 1.0, "step");
        assert_eq!(dabs, [(0, 1.0)]);
        commit_dabs(&mut smart, None, dabs, WHITE);
        assert_eq!(
            apply_replace_paint_source_shaped(&mut smart, WHITE, BLACK).updated_colors,
            [BLACK]
        );
    }

    #[test]
    fn clearing_texture_paint_retires_a_live_neutral_gray_dab() {
        let mut mesh = textured_face(BLACK);
        dab(&mut mesh, DEFAULT_FACE_COLOR, 1.0);
        assert_eq!(mesh.paint_source_faces(DEFAULT_FACE_COLOR), [0]);
        super::super::segment::reset_mesh_state(&mut mesh);
        assert_eq!(mesh.face_colors, [DEFAULT_FACE_COLOR]);
        assert!(mesh.paint_source_faces(DEFAULT_FACE_COLOR).is_empty());
    }

    #[test]
    fn stl_stroke_repaint_keeps_the_gray_base_estimate() {
        let mut mesh = textured_face(DEFAULT_FACE_COLOR);
        mesh.texture_painting = false;
        dab(&mut mesh, WHITE, 0.5);
        assert_eq!(mesh.face_colors, [[196, 196, 196, 255]]);
        let repaint = apply_replace_paint_source_shaped(&mut mesh, WHITE, BLACK);
        assert_eq!(repaint.updated_colors, [[69, 69, 69, 255]]);
    }
}

#[cfg(test)]
mod fill_tests {
    use super::*;
    use crate::mesh::model::{MeshModel, DEFAULT_FACE_COLOR};
    use std::collections::HashSet;

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
        assert_eq!(
            m.segment_labels,
            vec![1, 1, 2, 2],
            "labels are the region truth"
        );
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

        let result = apply_fill(&mut m, 0, [10, 200, 10, 255], 0.0, &HashSet::new());
        assert_eq!(
            result.updated_faces,
            vec![0, 1],
            "flood stays inside label 1"
        );
        assert_eq!(colors_of(&m, &[2, 3]), vec![RED; 2]);
    }
}

#[cfg(test)]
mod cull_hidden_tests {
    //! Hidden-region view state (#24/#29): implicit spatial tools must not
    //! touch faces the user has hidden, even when the dab's radius sweeps
    //! over them. The backend keeps the hidden LABEL set (synced from the
    //! frontend whenever it changes) and culls hit lists before committing.

    use super::*;
    use crate::mesh::model::MeshModel;
    use std::collections::HashSet;

    fn labeled_mesh() -> MeshModel {
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

    #[test]
    fn faces_of_hidden_labels_are_dropped() {
        let m = labeled_mesh();
        let mut hidden = HashSet::new();
        hidden.insert(2u32);

        let out = cull_hidden(&m, vec![0, 1, 2, 3], &hidden);
        assert_eq!(out, vec![0, 1], "label 2's faces are hidden");
    }

    #[test]
    fn empty_hidden_set_is_a_noop() {
        let m = labeled_mesh();
        let hidden = HashSet::new();
        let out = cull_hidden(&m, vec![2, 3], &hidden);
        assert_eq!(out, vec![2, 3]);
    }

    #[test]
    fn update_pairs_are_culled_by_face_key() {
        let m = labeled_mesh();
        let mut hidden = HashSet::new();
        hidden.insert(2u32);

        let updates = vec![
            (0u32, [9, 9, 9, 255]),
            (2, [8, 8, 8, 255]),
            (3, [7, 7, 7, 255]),
        ];
        let out = cull_hidden_pairs(&m, updates, &hidden);
        assert_eq!(out, vec![(0u32, [9, 9, 9, 255])]);
    }
}

#[cfg(test)]
mod fill_segments_tests {
    //! Owner scenario on #29: multi-select regions (View tool / panel), then
    //! fill them ALL in one go. N regions must be ONE history entry — not N.

    use super::*;
    use crate::mesh::model::MeshModel;

    /// Tetrahedron, two regions: label 1 = faces {0, 1}, label 2 = {2, 3}.
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

    #[test]
    fn paints_union_of_labels_in_one_history_entry() {
        let mut m = two_region_mesh();

        let result = apply_fill_segments(&mut m, &[1, 2], [255, 0, 0, 255]);

        assert_eq!(result.updated_faces.len(), 4);
        assert!(m.face_colors.iter().all(|&c| c == [255, 0, 0, 255]));
        // N regions, ONE undo step.
        assert_eq!(m.history.undo_depth(), 1);
        {
            let crate::mesh::model::MeshModel {
                history,
                face_colors,
                segment_labels,
                ..
            } = &mut m;
            history.jump_to(0, face_colors, segment_labels);
        }
        assert!(m
            .face_colors
            .iter()
            .all(|&c| c == crate::mesh::model::DEFAULT_FACE_COLOR));
    }

    #[test]
    fn duplicate_labels_paint_each_face_once() {
        let mut m = two_region_mesh();

        let result = apply_fill_segments(&mut m, &[1, 1, 2], [10, 20, 30, 255]);

        assert_eq!(result.updated_faces, vec![0, 1, 2, 3], "no face twice");
    }

    #[test]
    fn unknown_or_empty_labels_record_nothing() {
        let mut m = two_region_mesh();

        let empty = apply_fill_segments(&mut m, &[], [1, 2, 3, 255]);
        let absent = apply_fill_segments(&mut m, &[99], [1, 2, 3, 255]);

        assert_eq!(empty.updated_faces.len(), 0);
        assert_eq!(absent.updated_faces.len(), 0);
        assert_eq!(
            m.history.undo_depth(),
            0,
            "no-op must not create an undo step"
        );
    }
}

#[cfg(test)]
mod replace_colour_tests {
    //! Issue #29: colour find & replace. ONE semantic (owner decision): a
    //! whole-model replace — every region that used the source colour — as a
    //! single O(F) pass landing as exactly ONE history entry (one Ctrl+Z
    //! restores every replaced face), never touching labels, recording
    //! nothing when nothing matches.

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

        let result = apply_replace_face_color(&mut m, RED, GREEN);

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
            let crate::mesh::model::MeshModel {
                history,
                face_colors,
                segment_labels,
                ..
            } = &mut m;
            history.jump_to(0, face_colors, segment_labels);
        }
        assert_eq!(m.face_colors[0], RED, "undo restores the original colour");
        assert_eq!(m.face_colors[2], RED);
        assert_eq!(m.face_colors[1], before[1]);
    }

    #[test]
    fn no_match_records_no_history() {
        let mut m = replace_mesh();
        let absent: [u8; 4] = [1, 2, 3, 255];

        let result = apply_replace_face_color(&mut m, absent, GREEN);

        assert_eq!(result.updated_faces.len(), 0);
        assert_eq!(result.faces, 0);
        assert_eq!(result.regions, 0);
        // A no-op must not create an undo step the user has to step over.
        assert_eq!(m.history.undo_depth(), 0);
    }

    #[test]
    fn baked_neutral_grey_is_a_real_model_colour() {
        let mut model = replace_mesh();
        model.texture_painting = true;
        assert_eq!(
            apply_replace_face_color(&mut model, DEFAULT_FACE_COLOR, GREEN).updated_faces,
            vec![1, 3],
        );
        assert_eq!(model.history.undo_depth(), 1);
    }

    #[test]
    fn replace_keeps_labels_and_segmentation_intact() {
        let mut m = replace_mesh();
        let labels_before = m.segment_labels.clone();

        apply_replace_face_color(&mut m, RED, GREEN);

        assert_eq!(
            m.segment_labels, labels_before,
            "replace is a paint edit, not a segmentation edit"
        );
    }

    #[test]
    fn replace_updates_counts_faces_and_regions() {
        let m = replace_mesh();

        let (faces, regions) = replace_updates(&m, RED);
        assert_eq!((faces.len(), regions as usize), (2, 2));

        // Faces 1 and 3 were never painted, so they still carry the default
        // grey — countable (the guard refuses that replace, not the math).
        let (faces, regions) = replace_updates(&m, DEFAULT_FACE_COLOR);
        assert_eq!((faces.len(), regions as usize), (2, 2));
    }

    #[test]
    fn core_refuses_default_source_and_same_colour() {
        let mut m = replace_mesh();

        // from == to: mechanical no-op, empty result.
        let result = apply_replace_face_color(&mut m, RED, RED);
        assert_eq!(result.faces, 0);

        // Default grey as source: refused by the same contract the UI guard
        // implements — the IPC surface does not trust frontend state.
        let result = apply_replace_face_color(&mut m, DEFAULT_FACE_COLOR, GREEN);
        assert_eq!(result.faces, 0);
        assert_eq!(m.history.undo_depth(), 0);
    }
}

#[cfg(test)]
mod gradient_tests {
    use super::*;
    use crate::mesh::model::MeshModel;

    const RED: [u8; 4] = [255, 0, 0, 255];
    const BLUE: [u8; 4] = [0, 0, 255, 255];
    const STOPS: [[u8; 4]; 2] = [RED, BLUE];

    fn strip() -> MeshModel {
        let mut mesh = MeshModel::new();
        for x in 0..=4 {
            mesh.vertices
                .extend([[x as f32, 0.0, 0.0], [x as f32, 1.0, 0.0]]);
        }
        for i in (0..8).step_by(2) {
            mesh.faces
                .extend([[i, i + 2, i + 1], [i + 1, i + 2, i + 3]]);
        }
        // An intersecting disconnected triangle must stay unpainted.
        let first = mesh.vertices.len() as u32;
        mesh.vertices
            .extend([[1.0, 0.0, 0.1], [2.0, 0.0, 0.1], [1.0, 1.0, 0.1]]);
        mesh.faces.push([first, first + 1, first + 2]);
        mesh.segment_labels = vec![1, 1, 1, 1, 2, 2, 2, 2, 1];
        mesh.rebuild_segments();
        mesh.init_default_colors();
        mesh.compute_bbox();
        mesh.build_kdtree();
        mesh.build_adjacency();
        mesh
    }

    fn hit(mesh: &MeshModel, face_id: u32) -> GradientHit {
        GradientHit {
            face_id,
            point: mesh.face_center(face_id),
        }
    }

    #[test]
    fn gradient_brush_fills_a_sparse_drag_without_painting_another_surface() {
        let mut mesh = strip();
        let points = [hit(&mesh, 0), hit(&mesh, 7)];
        let result = apply_gradient_brush(&mut mesh, &points, &STOPS, 0.6).unwrap();
        assert_eq!(result.updated_faces, (0..8).collect::<Vec<_>>());
        assert_eq!(mesh.face_colors[0], RED);
        assert_eq!(mesh.face_colors[7], BLUE);
        assert_eq!(mesh.face_colors[8], DEFAULT_FACE_COLOR);
        assert_eq!(mesh.gradient_faces.len(), 8);
        assert_eq!(mesh.history.undo_depth(), 1);
    }

    #[test]
    fn gradient_brush_keeps_a_narrow_stroke_continuous_on_coarse_triangles() {
        let mut mesh = strip();
        let points = [
            GradientHit {
                face_id: 0,
                point: [0.1, 0.05, 0.0],
            },
            GradientHit {
                face_id: 7,
                point: [3.99, 0.05, 0.0],
            },
        ];
        let result = apply_gradient_brush(&mut mesh, &points, &STOPS, 0.1).unwrap();
        assert_eq!(result.updated_faces, (0..8).collect::<Vec<_>>());
        assert!(mesh.face_colors[3][0] > mesh.face_colors[4][0]);
        assert_eq!(mesh.face_colors[8], DEFAULT_FACE_COLOR);
    }

    #[test]
    fn gradient_fill_uses_start_segment_and_shift_uses_connected_region() {
        let mut mesh = strip();
        let start = hit(&mesh, 0);
        let end = hit(&mesh, 6);
        let result = apply_gradient_fill(&mut mesh, start, end, &STOPS, 0.6, false).unwrap();
        assert_eq!(result.updated_faces, [0, 1, 2, 3, 8]);
        assert_eq!(mesh.face_colors[2], [170, 0, 85, 255]);
        let mut mesh = strip();
        let result = apply_gradient_fill(&mut mesh, start, end, &STOPS, 0.6, true).unwrap();
        assert_eq!(result.updated_faces, [0, 1, 2, 3]);
    }

    #[test]
    fn gradient_fill_matches_solid_fill_for_a_single_partition_and_local_fallback() {
        let mut mesh = strip();
        mesh.segment_labels.fill(0);
        mesh.rebuild_segments();
        let start = hit(&mesh, 0);
        let end = hit(&mesh, 7);
        let result = apply_gradient_fill(&mut mesh, start, end, &STOPS, 0.6, false).unwrap();
        assert_eq!(result.updated_faces, (0..9).collect::<Vec<_>>());

        let mut mesh = strip();
        mesh.segments.clear();
        let start = hit(&mesh, 0);
        let end = hit(&mesh, 7);
        let result = apply_gradient_fill(&mut mesh, start, end, &STOPS, 0.6, false).unwrap();
        assert_eq!(result.updated_faces, [0, 1]);
    }

    #[test]
    fn gradient_brush_uses_arc_length_on_a_bent_path_with_multiple_stops() {
        let mut mesh = strip();
        const GREEN: [u8; 4] = [0, 255, 0, 255];
        let points = [hit(&mesh, 0), hit(&mesh, 7), hit(&mesh, 6)];
        apply_gradient_brush(&mut mesh, &points, &[RED, GREEN, BLUE], 0.6).unwrap();
        // The blue end is face 6, but face 7 lies at ~88% of the total arc.
        assert_eq!(mesh.face_colors[0], RED);
        assert_eq!(mesh.face_colors[6], BLUE);
        assert_eq!(mesh.face_colors[7], [0, 63, 192, 255]);
        assert_eq!(mesh.gradient_faces[&7].from, GREEN);
        assert_eq!(mesh.gradient_faces[&7].to, BLUE);
    }

    #[test]
    fn gradient_rejects_invalid_hits_and_directions_without_mutation() {
        let mut mesh = strip();
        let start = hit(&mesh, 0);
        let mut off_mesh = hit(&mesh, 2);
        off_mesh.point[2] = 3.0;
        assert!(apply_gradient_brush(&mut mesh, &[start, off_mesh], &STOPS, 0.6).is_err());
        assert!(apply_gradient_fill(&mut mesh, start, start, &STOPS, 0.6, false).is_err());
        assert_eq!(mesh.history.undo_depth(), 0);
        assert!(mesh.gradient_faces.is_empty());
        assert_eq!(mesh.face_colors, vec![DEFAULT_FACE_COLOR; 9]);
    }

    #[test]
    fn gradient_accepts_six_default_stops_and_the_eight_stop_limit() {
        let stops = [
            [235, 65, 75, 255],
            [252, 143, 45, 255],
            [250, 219, 72, 255],
            [83, 197, 105, 255],
            [53, 140, 245, 255],
            [149, 79, 230, 255],
            [128, 0, 255, 255],
            [255, 0, 128, 255],
            [255, 255, 255, 255],
        ];
        let mut mesh = strip();
        let start = hit(&mesh, 0);
        let end = hit(&mesh, 7);
        apply_gradient_brush(&mut mesh, &[start, end], &stops[..6], 0.6).unwrap();
        assert_eq!(mesh.face_colors[7], [149, 79, 230, 255]);
        apply_gradient_fill(&mut mesh, end, start, &stops[..8], 0.6, false).unwrap();
        assert_eq!(mesh.face_colors[7], [235, 65, 75, 255]);
        assert!(apply_gradient_brush(&mut mesh, &[start, end], &stops, 0.6).is_err());
        assert_eq!(mesh.history.undo_depth(), 2);
    }

    #[test]
    fn gradient_project_round_trips_semantic_history_and_same_colour_overwrites() {
        let mut mesh = strip();
        let points = [hit(&mesh, 0), hit(&mesh, 7)];
        apply_gradient_brush(&mut mesh, &points, &STOPS, 0.6).unwrap();
        let gradient = mesh.gradient_faces.clone();
        let colors = mesh.face_colors.clone();

        // Same RGB, but an explicit solid stroke replaces the gradient.
        commit(&mut mesh, None, vec![(0, RED)], Some(RED));
        assert!(!mesh.gradient_faces.contains_key(&0));
        assert_eq!(mesh.history.undo_depth(), 2);

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("gradient.3mf");
        crate::export::threemf::export_3mf(&mesh, &path, None, &|_, _| {}).unwrap();
        let mut loaded = crate::threemf::load_plate(&path, 0, &|_, _| {})
            .unwrap()
            .mesh;
        assert_eq!(loaded.face_colors, colors);
        assert_eq!(loaded.gradient_faces, mesh.gradient_faces);
        let groups = loaded.gradient_paint_groups();
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[STOPS.as_slice()].len(), gradient.len() - 1);
        assert!(!groups[STOPS.as_slice()].contains(&0));

        loaded
            .history
            .undo_with_gradients(
                &mut loaded.face_colors,
                &mut loaded.segment_labels,
                &mut loaded.gradient_faces,
            )
            .unwrap();
        assert_eq!(loaded.gradient_faces, gradient);
        assert_eq!(loaded.gradient_paint_groups()[STOPS.as_slice()].len(), gradient.len());
        loaded
            .history
            .jump_to_with_gradients(
                0,
                &mut loaded.face_colors,
                &mut loaded.segment_labels,
                &mut loaded.gradient_faces,
            )
            .unwrap();
        assert!(loaded.gradient_faces.is_empty());
        assert_eq!(loaded.face_colors, vec![DEFAULT_FACE_COLOR; 9]);

        crate::export::threemf::export_3mf(&loaded, &path, None, &|_, _| {}).unwrap();
        let mut loaded = crate::threemf::load_plate(&path, 0, &|_, _| {})
            .unwrap()
            .mesh;
        loaded
            .history
            .redo_with_gradients(
                &mut loaded.face_colors,
                &mut loaded.segment_labels,
                &mut loaded.gradient_faces,
            )
            .unwrap();
        assert_eq!(loaded.gradient_faces, gradient);
        loaded
            .history
            .redo_with_gradients(
                &mut loaded.face_colors,
                &mut loaded.segment_labels,
                &mut loaded.gradient_faces,
            )
            .unwrap();
        assert_eq!(loaded.gradient_faces, mesh.gradient_faces);
        assert_eq!(loaded.face_colors, colors);
    }

    #[test]
    fn gradient_project_reopens_with_restored_brush_and_redo_fill_context() {
        use crate::export::threemf::{write_project_3mf, ProjectPlate, ProjectWritePurpose};

        let mut mesh = strip();
        let points = [hit(&mesh, 0), hit(&mesh, 7)];
        const GREEN: [u8; 4] = [0, 255, 0, 255];
        apply_gradient_brush(&mut mesh, &points, &[RED, GREEN, BLUE], 0.6).unwrap();
        let fill = apply_gradient_fill(&mut mesh, points[1], points[0], &STOPS, 1.5, true).unwrap();
        mesh.history
            .undo_with_gradients(
                &mut mesh.face_colors,
                &mut mesh.segment_labels,
                &mut mesh.gradient_faces,
            )
            .unwrap();

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("gradient-context.cym.3mf");
        write_project_3mf(
            &[ProjectPlate {
                name: "strip".into(),
                mesh: &mesh,
            }],
            0,
            &path,
            None,
            None,
            ProjectWritePurpose::EngineeringSave,
            &|_, _| {},
        )
        .unwrap();
        let mut loaded = crate::threemf::load_plate(&path, 0, &|_, _| {})
            .unwrap()
            .mesh;

        let groups = loaded.gradient_paint_groups();
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[&[RED, GREEN, BLUE][..]].len(), loaded.gradient_faces.len());
        let brush = serde_json::to_value(loaded.to_dto()).unwrap();
        assert_eq!(
            brush["gradientContext"],
            serde_json::json!({
                "tool": "brush", "stops": [RED, GREEN, BLUE], "radius": 0.6_f32,
                "points": [
                    {"faceId": 0, "point": points[0].point},
                    {"faceId": 7, "point": points[1].point}
                ], "wholeRegion": false
            })
        );
        assert_eq!(
            loaded
                .history
                .entries()
                .iter()
                .map(|entry| entry.kind.key())
                .collect::<Vec<_>>(),
            ["gradientBrush", "gradientFill"]
        );

        loaded
            .history
            .jump_to_with_gradients(
                2,
                &mut loaded.face_colors,
                &mut loaded.segment_labels,
                &mut loaded.gradient_faces,
            )
            .unwrap();
        let all_groups = loaded.gradient_paint_groups();
        assert_eq!(all_groups[STOPS.as_slice()].len(), fill.updated_faces.len());
        assert_eq!(all_groups[&[RED, GREEN, BLUE][..]].len(),
            loaded.gradient_faces.len() - fill.updated_faces.len());
        let fill = serde_json::to_value(loaded.to_dto()).unwrap();
        assert_eq!(
            fill["gradientContext"],
            serde_json::json!({
                "tool": "fill", "stops": STOPS, "radius": 1.5,
                "points": [
                    {"faceId": 7, "point": points[1].point},
                    {"faceId": 0, "point": points[0].point}
                ], "wholeRegion": true
            })
        );
    }

    #[test]
    fn legacy_gradient_history_keeps_semantics_without_inventing_a_gesture() {
        use crate::mesh::history::{History, OpKind};

        let mut mesh = strip();
        let points = [hit(&mesh, 0), hit(&mesh, 7)];
        apply_gradient_brush(&mut mesh, &points, &STOPS, 0.6).unwrap();
        commit(&mut mesh, None, vec![(0, RED)], Some(RED));
        let (mut undo, mut redo) = mesh.history.to_raw();
        for entry in undo.iter_mut().chain(&mut redo) {
            for (_, sample) in &mut entry.gradients {
                sample.ramp_id = 0;
            }
        }
        for sample in mesh.gradient_faces.values_mut() {
            sample.ramp_id = 0;
        }
        mesh.gradient_ramps.clear();
        undo[0].kind = OpKind::Paint;
        undo[0].gradient_context = None;
        mesh.history = History::from_raw(undo, redo);

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("legacy-gradient.cym.3mf");
        crate::export::threemf::export_3mf(&mesh, &path, None, &|_, _| {}).unwrap();
        let loaded = crate::threemf::load_plate(&path, 0, &|_, _| {})
            .unwrap()
            .mesh;

        assert!(loaded.to_dto().has_gradient_paint);
        assert!(loaded.to_dto().gradient_context.is_none());
        assert_eq!(loaded.gradient_faces, mesh.gradient_faces);
        let groups = loaded.gradient_paint_groups();
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[STOPS.as_slice()].len(), mesh.gradient_faces.len());
        assert_eq!(
            loaded
                .history
                .entries()
                .iter()
                .map(|entry| entry.kind.key())
                .collect::<Vec<_>>(),
            ["legacyGradientPaint", "paint"]
        );
    }

    #[test]
    fn whole_ramp_survives_history_eviction_and_project_reopen() {
        let mut mesh = strip();
        let stops = [RED, [0, 255, 0, 255], BLUE];
        let points = [hit(&mesh, 0), hit(&mesh, 7)];
        apply_gradient_brush(&mut mesh, &points, &stops, 0.6).unwrap();
        for i in 0..mesh.history.max_depth() {
            let color = if i % 2 == 0 { RED } else { BLUE };
            commit(&mut mesh, None, vec![(8, color)], Some(color));
        }
        assert!(mesh.history.gradient_context().is_none());
        assert_eq!(mesh.gradient_paint_groups()[stops.as_slice()].len(), 8);

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("evicted-gradient.cym.3mf");
        crate::export::threemf::export_3mf(&mesh, &path, None, &|_, _| {}).unwrap();
        let loaded = crate::threemf::load_plate(&path, 0, &|_, _| {}).unwrap().mesh;
        assert_eq!(loaded.gradient_faces, mesh.gradient_faces);
        assert_eq!(loaded.gradient_paint_groups()[stops.as_slice()].len(), 8);
    }

    #[test]
    #[ignore = "requires the CYM_LEGACY_GRADIENT_PROJECT cylinder file"]
    fn opens_real_legacy_gradient_project() {
        let source = std::env::var_os("CYM_LEGACY_GRADIENT_PROJECT").unwrap();
        let mut mesh = crate::threemf::load_plate(std::path::Path::new(&source), 0, &|_, _| {})
            .unwrap()
            .mesh;
        assert_eq!(mesh.faces.len(), 213_838);
        assert_eq!(mesh.gradient_faces.len(), 74_510);
        assert!(mesh
            .gradient_faces
            .values()
            .all(|s| s.from == [235, 65, 75, 255] && s.to == [0, 176, 240, 255]));
        assert!(mesh.history.gradient_context().is_none());
        assert_eq!(
            mesh.history
                .entries()
                .iter()
                .map(|entry| entry.kind.key())
                .collect::<Vec<_>>(),
            ["paint", "paint", "legacyGradientPaint"]
        );
        let exact = mesh.gradient_faces.clone();
        mesh.history
            .undo_with_gradients(
                &mut mesh.face_colors,
                &mut mesh.segment_labels,
                &mut mesh.gradient_faces,
            )
            .unwrap();
        assert!(mesh.gradient_faces.is_empty());
        mesh.history
            .redo_with_gradients(
                &mut mesh.face_colors,
                &mut mesh.segment_labels,
                &mut mesh.gradient_faces,
            )
            .unwrap();
        assert_eq!(mesh.gradient_faces, exact);
    }
}
