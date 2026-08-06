use tauri::{Emitter, State};
use serde::{Deserialize, Serialize};

use crate::commands::mesh::AppState;
use crate::mesh::loader::ProgressFn;
use crate::mesh::model::Segment;
use crate::segment::dihedral::segment_by_dihedral_angle;
use crate::segment::manual::{
    finalize_manual_region as backend_finalize_manual_region, undo_last_manual_region,
    snap_point_to_vertex_on_face, MANUAL_SEGMENT_OFFSET,
};
use crate::segment::sdf::segment_by_sdf;

/// Flatten per-face `[[r,g,b,a]; N]` into a flat `Vec<u8>` matching `MeshDataDto.faceColors`.
fn flatten_face_colors(colors: &[[u8; 4]]) -> Vec<u8> {
    colors.iter().flat_map(|c| c.iter().copied()).collect()
}

/// Result of auto-segmentation: segment metadata + per-face labels + face colors.
/// `face_colors` is returned so the frontend can repaint newly-touched faces
/// (and preserve existing paint on re-segment) without a full reload.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SegmentResult {
    pub segments: Vec<Segment>,
    pub segment_labels: Vec<u32>,
    pub face_colors: Vec<u8>,
}

/// Manual segment label offset — imported from segment::manual so it stays
/// the single source of truth (auto_segment_* also reuse it).

#[tauri::command]
pub fn auto_segment(
    angle_threshold: f32,
    app: tauri::AppHandle,
    state: State<AppState>,
) -> Result<SegmentResult, String> {
    log::info!("[cmd:auto_segment] angle_threshold={}", angle_threshold);

    let mut mesh_guard = state.mesh.lock().map_err(|e| e.to_string())?;
    let mesh = mesh_guard.as_mut().ok_or("No mesh loaded")?;

    // Build progress callback that emits Tauri events
    let app_for_progress = app.clone();
    let progress_cb: Box<ProgressFn> = Box::new(move |fraction: f32, stage: &str| {
        let _ = app_for_progress.emit(
            "segment-progress",
            serde_json::json!({ "progress": fraction, "stage": stage }),
        );
    });

    let segments = segment_by_dihedral_angle(mesh, angle_threshold, &*progress_cb);
    // Auto segmentation rewrites all labels; drop stale history so undo cannot
    // restore pre-auto labels onto the new result. Both stacks, because both
    // hold diffs recorded against the labels that were just replaced.
    mesh.manual_region_history.clear();
    mesh.history.clear();

    // Emit completion
    let _ = app.emit(
        "segment-progress",
        serde_json::json!({ "progress": 1.0, "stage": format!("Found {} regions", segments.len()) }),
    );

    log::info!("[cmd:auto_segment] done: {} segments", segments.len());

    Ok(SegmentResult {
        segments,
        segment_labels: mesh.segment_labels.clone(),
        face_colors: flatten_face_colors(&mesh.face_colors),
    })
}

/// Result of painting a single face into a manual segment (incremental).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PaintSegmentFaceResult {
    pub face_id: u32,
    pub color: [u8; 4],
    pub segment_label: u32,
}

/// Paint a single face into a manual segment.
///
/// Called during drag: user drags segment brush across faces.
/// - First call: segment_label = None → creates new label
/// - Subsequent calls: segment_label = Some(label) → extends that segment
///
/// Returns the face color for immediate incremental GPU update.
#[tauri::command]
pub fn paint_segment_face(
    state: State<AppState>,
    face_id: u32,
    segment_label: Option<u32>,
    stroke_id: Option<u64>,
) -> Result<PaintSegmentFaceResult, String> {
    let mut mesh_guard = state.mesh.lock().map_err(|e| e.to_string())?;
    let mesh = mesh_guard.as_mut().ok_or("No mesh loaded")?;

    let face_count = mesh.faces.len();
    if face_id as usize >= face_count {
        return Err(format!("face_id {} out of range ({})", face_id, face_count));
    }

    // Determine label
    let label = match segment_label {
        Some(l) if l >= MANUAL_SEGMENT_OFFSET => l,
        _ => {
            let max_existing = mesh.segment_labels.iter().copied().max().unwrap_or(0);
            std::cmp::max(MANUAL_SEGMENT_OFFSET, max_existing + 1)
        }
    };

    // Generate color from label hash (deterministic per label)
    let color_seed = label.wrapping_mul(2654435761) >> 24;
    let color: [u8; 4] = [
        ((color_seed * 73) % 200 + 55) as u8,
        ((color_seed * 151) % 200 + 55) as u8,
        ((color_seed * 223) % 200 + 55) as u8,
        255,
    ];

    // Assign label + color to this face, recording both for undo. The drag
    // fires this once per face, so `stroke_id` is what collapses a whole
    // segment-brush gesture into one undo level.
    mesh.apply_segment_paint(stroke_id, face_id, label, color);

    Ok(PaintSegmentFaceResult {
        face_id,
        color,
        segment_label: label,
    })
}

/// Result of finalizing a manual segment: updated segment metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FinalizeSegmentResult {
    pub segments: Vec<Segment>,
    pub segment_labels: Vec<u32>,
}

/// Finalize a manual segment after the user finishes painting.
///
/// Rebuilds segment metadata (face counts, names) from current labels.
/// Called once on pointer-up after a segment paint drag.
#[tauri::command]
pub fn finalize_segment(
    state: State<AppState>,
    segment_label: u32,
) -> Result<FinalizeSegmentResult, String> {
    let mut mesh_guard = state.mesh.lock().map_err(|e| e.to_string())?;
    let mesh = mesh_guard.as_mut().ok_or("No mesh loaded")?;

    if segment_label < MANUAL_SEGMENT_OFFSET {
        return Err(format!("Invalid manual segment label: {}", segment_label));
    }

    // Rebuild full segment metadata from current labels (single source of truth).
    mesh.rebuild_segments();
    let mut segments: Vec<Segment> = mesh.segments.values().cloned().collect();
    segments.sort_by_key(|s| s.id);

    log::info!(
        "[cmd:finalize_segment] label={}, total_segments={}",
        segment_label,
        segments.len()
    );

    Ok(FinalizeSegmentResult {
        segments,
        segment_labels: mesh.segment_labels.clone(),
    })
}

/// Result of snapping a clicked 3D point to the nearest mesh vertex.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ManualPointResult {
    pub vertex_index: u32,
    pub face_id: u32,
    pub snapped: [f32; 3],
}

/// Snap a clicked 3D point (model-local coordinates) to the nearest mesh vertex.
/// `face_index` is the triangle hit by the raycaster; it lets the backend snap
/// to the SAME-SIDE vertex (front shell) instead of a global nearest that could
/// land on a back-face vertex through a thin mesh — see
/// `segment::manual::snap_point_to_vertex_on_face`.
#[tauri::command]
pub fn manual_region_add_point(
    point: [f32; 3],
    face_index: u32,
    state: State<AppState>,
) -> Result<ManualPointResult, String> {
    let mut mesh_guard = state.mesh.lock().map_err(|e| e.to_string())?;
    let mesh = mesh_guard.as_mut().ok_or("No mesh loaded")?;

    let (vi, snapped) = snap_point_to_vertex_on_face(mesh, &point, face_index)
        .ok_or_else(|| "No vertex found near point".to_string())?;
    // Anchor face: the first face containing this vertex
    let face_id = mesh
        .face_of_vertex(vi)
        .ok_or_else(|| format!("vertex {} has no incident face", vi))?;

    Ok(ManualPointResult {
        vertex_index: vi,
        face_id,
        snapped,
    })
}

/// Finalize a manual region from an ordered list of clicked 3D points.
/// Closes the loop (last point connects back to first), builds the enclosed
/// region via geodesic surface paths + bounded flood fill, and assigns a fresh
/// manual segment label. See segment::manual::finalize_manual_region.
#[tauri::command]
pub fn finalize_manual_region(
    points: Vec<[f32; 3]>,
    face_indices: Vec<u32>,
    app: tauri::AppHandle,
    state: State<AppState>,
) -> Result<SegmentResult, String> {
    let mut mesh_guard = state.mesh.lock().map_err(|e| e.to_string())?;
    let mesh = mesh_guard.as_mut().ok_or("No mesh loaded")?;

    let (_label, region) = backend_finalize_manual_region(mesh, &points, &face_indices)?;

    // Emit completion
    let _ = app.emit(
        "segment-progress",
        serde_json::json!({ "progress": 1.0, "stage": format!("Manual region: {} faces", region.len()) }),
    );

    Ok(SegmentResult {
        segments: mesh.segments.values().cloned().collect(),
        segment_labels: mesh.segment_labels.clone(),
        face_colors: flatten_face_colors(&mesh.face_colors),
    })
}

/// Undo the last finalized manual (lasso) region.
///
/// Restores affected faces to their pre-finalize state (which may itself be an
/// earlier manual region) and returns the updated segment metadata so the
/// frontend can repaint. Returns an error when there is nothing to undo.
#[tauri::command]
pub fn manual_region_undo(state: State<AppState>) -> Result<SegmentResult, String> {
    log::info!("[cmd:manual_region_undo] invoked");
    let mut mesh_guard = state.mesh.lock().map_err(|e| e.to_string())?;
    let mesh = mesh_guard.as_mut().ok_or("No mesh loaded")?;

    match undo_last_manual_region(mesh) {
        Some(label) => {
            mesh.rebuild_segments();
            log::info!("[cmd:manual_region_undo] reverted label={}", label);
            Ok(SegmentResult {
                segments: mesh.segments.values().cloned().collect(),
                segment_labels: mesh.segment_labels.clone(),
                face_colors: flatten_face_colors(&mesh.face_colors),
            })
        }
        None => Err("No manual region to undo".into()),
    }
}

/// Restore a previously snapshotted paint state (full per-face colors + labels).
///
/// Frontend undo/redo of painting ships the pre-stroke snapshot (faceColors +
/// segmentLabels) captured before a stroke. We validate lengths against the
/// live mesh, restore both arrays via `MeshModel::restore_paint_state` (which
/// also rebuilds segment metadata so a reverted just-created region disappears),
/// and return the updated result for a single repaint.
#[tauri::command]
pub fn restore_face_colors(
    state: State<AppState>,
    face_colors: Vec<u8>,
    segment_labels: Vec<u32>,
) -> Result<SegmentResult, String> {
    let mut mesh_guard = state.mesh.lock().map_err(|e| e.to_string())?;
    let mesh = mesh_guard.as_mut().ok_or("No mesh loaded")?;

    if face_colors.len() != mesh.face_colors.len() * 4 {
        return Err(format!(
            "face_colors length {} != expected {}",
            face_colors.len(),
            mesh.face_colors.len() * 4
        ));
    }
    if segment_labels.len() != mesh.segment_labels.len() {
        return Err(format!(
            "segment_labels length {} != expected {}",
            segment_labels.len(),
            mesh.segment_labels.len()
        ));
    }

    // Reassemble [u8;4] from the flattened Vec<u8>.
    let mut restored: Vec<[u8; 4]> = Vec::with_capacity(face_colors.len() / 4);
    for chunk in face_colors.chunks_exact(4) {
        restored.push([chunk[0], chunk[1], chunk[2], chunk[3]]);
    }
    mesh.restore_paint_state(&restored, &segment_labels);

    log::info!(
        "[cmd:restore_face_colors] restored, segments={}",
        mesh.segments.len()
    );

    Ok(SegmentResult {
        segments: mesh.segments.values().cloned().collect(),
        segment_labels: mesh.segment_labels.clone(),
        face_colors: flatten_face_colors(&mesh.face_colors),
    })
}

/// Smart auto-segmentation via Shape Diameter Function (Tier 0).
/// Produces semantic "parts" (thin vs thick) instead of the dihedral
/// normal-only clusters. `k = 0` auto-estimates cluster count from SDF peaks.
#[tauri::command]
pub fn auto_segment_smart(
    k: u32,
    app: tauri::AppHandle,
    state: State<AppState>,
) -> Result<SegmentResult, String> {
    log::info!("[cmd:auto_segment_smart] k={}", k);
    let mut mesh_guard = state.mesh.lock().map_err(|e| e.to_string())?;
    let mesh = mesh_guard.as_mut().ok_or("No mesh loaded")?;

    let _ = app.emit(
        "segment-progress",
        serde_json::json!({ "progress": 0.1, "stage": "Computing SDF..." }),
    );
    let segments = segment_by_sdf(mesh, k);
    // Auto segmentation rewrites all labels; drop stale history (see auto_segment).
    mesh.manual_region_history.clear();
    mesh.history.clear();

    let _ = app.emit(
        "segment-progress",
        serde_json::json!({ "progress": 1.0, "stage": format!("Found {} parts", segments.len()) }),
    );
    log::info!("[cmd:auto_segment_smart] done: {} parts", segments.len());

    Ok(SegmentResult {
        segments,
        segment_labels: mesh.segment_labels.clone(),
        face_colors: flatten_face_colors(&mesh.face_colors),
    })
}
