use tauri::{Emitter, State};
use serde::{Deserialize, Serialize};

use crate::commands::mesh::AppState;
use crate::mesh::loader::ProgressFn;
use crate::mesh::model::Segment;
use crate::segment::dihedral::segment_by_dihedral_angle;
use crate::segment::manual::{
    finalize_manual_region as backend_finalize_manual_region, snap_point_to_vertex_on_face,
    MANUAL_SEGMENT_OFFSET,
};
use crate::segment::sdf::segment_by_sdf;
use crate::segment::split::{split_segment as split_segment_impl, SplitMethod, SplitResult};
use crate::segment::resegment::resegment_region as resegment_region_impl;
use crate::segment::recommend::{recommend_seeds as backend_recommend_seeds, SeedSuggestion};
use crate::segment::seeded::{seed_grow as backend_seed_grow, SeedGrowParams, SeedInput};
use crate::segment::{run_segmentation, SegmentationAlgorithm};

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
    // restore pre-auto labels onto the new result — every recorded diff was
    // taken against the labels that were just replaced.
    mesh.history.clear();

    // Emit completion
    let _ = app.emit(
        "segment-progress",
        serde_json::json!({ "progress": 1.0, "stage": "done" }),
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

    // Continue the stroke's existing segment, or reserve a fresh one on the
    // first face of the drag. Allocation never recycles a retired number —
    // see MeshModel::alloc_manual_label.
    let label = match segment_label {
        Some(l) if l >= MANUAL_SEGMENT_OFFSET => l,
        _ => mesh.alloc_manual_label(),
    };
    let color = crate::mesh::model::MeshModel::manual_label_color(label);

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
    let segments: Vec<Segment> = mesh.sorted_segments();

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

    let (_label, _region) = backend_finalize_manual_region(mesh, &points, &face_indices)?;

    // Emit completion
    let _ = app.emit(
        "segment-progress",
        serde_json::json!({ "progress": 1.0, "stage": "done" }),
    );

    Ok(SegmentResult {
        segments: mesh.sorted_segments(),
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

    let app_for_progress = app.clone();
    let progress_cb: Box<ProgressFn> = Box::new(move |fraction: f32, stage: &str| {
        let _ = app_for_progress.emit(
            "segment-progress",
            serde_json::json!({ "progress": fraction, "stage": stage }),
        );
    });
    let segments = segment_by_sdf(mesh, k, &*progress_cb);
    // Auto segmentation rewrites all labels; drop stale history (see auto_segment).
    mesh.history.clear();

    let _ = app.emit(
        "segment-progress",
        serde_json::json!({ "progress": 1.0, "stage": "done" }),
    );
    log::info!("[cmd:auto_segment_smart] done: {} parts", segments.len());

    Ok(SegmentResult {
        segments,
        segment_labels: mesh.segment_labels.clone(),
        face_colors: flatten_face_colors(&mesh.face_colors),
    })
}

/// What the frontend has to replace after a merge.
///
/// No `face_colors`: a merge moves labels and leaves the paint alone, so
/// shipping the whole colour buffer back would be several megabytes of JSON
/// describing a buffer that did not change.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MergeResult {
    pub segments: Vec<Segment>,
    pub segment_labels: Vec<u32>,
    pub moved_faces: usize,
}

/// Absorb `source_ids` into `target_id`, so they become one region.
///
/// The target is chosen by the caller rather than inferred (largest region,
/// lowest label, …): every inference rule is wrong for some selection, and the
/// panel already knows which row the user anchored the selection on.
#[tauri::command]
pub fn merge_segments(
    target_id: u32,
    source_ids: Vec<u32>,
    state: State<AppState>,
) -> Result<MergeResult, String> {
    log::info!(
        "[cmd:merge_segments] target={} sources={:?}",
        target_id,
        source_ids
    );
    let mut mesh_guard = state.mesh.lock().map_err(|e| e.to_string())?;
    let mesh = mesh_guard.as_mut().ok_or("No mesh loaded")?;
    let moved_faces = mesh.merge_segments(target_id, &source_ids)?;
    Ok(MergeResult {
        segments: mesh.sorted_segments(),
        segment_labels: mesh.segment_labels.clone(),
        moved_faces,
    })
}

/// Divide one region into sub-regions along its internal creases.
///
/// Mirrors `merge_segments`: it returns fresh `segments` + `segment_labels`
/// (no `face_colors` — a split moves labels and leaves the paint alone, so
/// shipping the colour buffer back would describe a buffer that did not change).
/// `method` is the wire-tagged [`SplitMethod`]; today only `crease` is wired,
/// `plane` returns an explicit "not implemented" error. History is label-only via
/// `OpKind::Split`, so the split is undoable like a merge.
#[tauri::command]
pub fn split_segment(
    label: u32,
    method: SplitMethod,
    state: State<AppState>,
) -> Result<SplitResult, String> {
    log::info!("[cmd:split_segment] label={} method={:?}", label, method);
    let mut mesh_guard = state.mesh.lock().map_err(|e| e.to_string())?;
    let mesh = mesh_guard.as_mut().ok_or("No mesh loaded")?;
    let result = split_segment_impl(mesh, label, &method)?;
    log::info!(
        "[cmd:split_segment] moved {} faces; kept_label={} new_label={}",
        result.moved_faces,
        result.kept_label,
        result.new_label
    );
    Ok(result)
}

/// Give a region a user-facing name, or clear it back to "Region N".
///
/// Returns the whole segment list rather than an acknowledgement: the panel
/// renders straight off that vector, and handing back one patched row would
/// mean the frontend has to splice it into two separate store slices
/// (`meshData.segments` and the top-level `segments`) and keep them in sync.
///
/// Deliberately **not** recorded in the undo history. The history is a face
/// diff — it stores `(face, colour)` and `(face, label)` pairs and replays them
/// by swapping buffer entries — and a rename touches zero faces. Threading it
/// through would mean a third payload shape on every entry plus a name-aware
/// `HistoryOutcome`, all to undo an edit the user can reverse by typing. The
/// separation is also what keeps an unrelated Ctrl+Z from silently reverting a
/// name the user set five strokes ago.
#[tauri::command]
pub fn rename_segment(
    segment_id: u32,
    name: String,
    state: State<AppState>,
) -> Result<Vec<Segment>, String> {
    log::info!("[cmd:rename_segment] id={} name={:?}", segment_id, name);
    let mut mesh_guard = state.mesh.lock().map_err(|e| e.to_string())?;
    let mesh = mesh_guard.as_mut().ok_or("No mesh loaded")?;
    mesh.rename_segment(segment_id, &name)?;
    Ok(mesh.sorted_segments())
}

/// Unified multi-algorithm auto-segmentation entry point. The UI sends a single
/// `algorithm` enum (dihedral | shapeDiameter | curvatureKMeans) with its
/// parameters; the backend dispatches via `run_segmentation`. One interface
/// replaces the previous per-algorithm commands (REFUTE: avoid N near-identical
/// commands and the configuration drift that caused).
/// `preserve_manual` is optional and defaults to **true**: omitting it must not
/// silently pick the destructive branch. A caller that forgets the argument
/// (or an older frontend bundle) keeps the user's hand-drawn regions; wiping
/// them stays something you have to ask for explicitly.
#[tauri::command]
pub async fn auto_segment_v2(
    algorithm: SegmentationAlgorithm,
    preserve_manual: Option<bool>,
    app: tauri::AppHandle,
    state: State<'_, AppState>,
) -> Result<SegmentResult, String> {
    let preserve_manual = preserve_manual.unwrap_or(true);
    log::info!(
        "[cmd:auto_segment_v2] algorithm={:?} preserve_manual={}",
        algorithm,
        preserve_manual
    );

    let app_for_progress = app.clone();
    let progress_cb: Box<ProgressFn> = Box::new(move |fraction: f32, stage: &str| {
        let _ = app_for_progress.emit(
            "segment-progress",
            serde_json::json!({ "progress": fraction, "stage": stage }),
        );
    });

    // Take the mesh OUT of the shared Mutex for the whole (potentially very long)
    // segmentation so a concurrent *synchronous* command on the main thread
    // cannot freeze waiting on this lock (REFUTE major-1: a held MutexGuard on a
    // worker thread makes every main-thread sync command block on `lock()`).
    // Tauri v2 runs this async command on its multi-threaded tokio runtime, so
    // the heavy work no longer blocks the webview thread (the "(未响应)" freeze).
    // The mesh is restored afterwards; any command that races in during compute
    // sees "No mesh loaded" instead of stalling the UI.
    let mut mesh_guard = state.mesh.lock().map_err(|e| e.to_string())?;
    let mut mesh = mesh_guard.take().ok_or("No mesh loaded")?;
    drop(mesh_guard);

    let segments = run_segmentation(&mut mesh, &algorithm, preserve_manual, &*progress_cb);
    // Auto segmentation rewrites all labels; drop stale history (see auto_segment).
    mesh.history.clear();

    let _ = app.emit(
        "segment-progress",
        serde_json::json!({ "progress": 1.0, "stage": "done" }),
    );
    log::info!("[cmd:auto_segment_v2] done: {} segments", segments.len());

    // Auto segmentation never mutates face colours (only labels), so shipping the
    // full colour buffer back would be several megabytes of identical JSON that
    // also forces a full front-end repaint (REFUTE major-5). Mirror
    // MergeResult/SplitResult: return labels + metadata only.
    let labels = mesh.segment_labels.clone();
    let mut mesh_guard = state.mesh.lock().map_err(|e| e.to_string())?;
    *mesh_guard = Some(mesh);

    Ok(SegmentResult {
        segments,
        segment_labels: labels,
        face_colors: Vec::new(),
    })
}

/// Re-run any segmentation algorithm on a single existing region, replacing it
/// with the sub-regions the algorithm finds inside it. See
/// `segment::resegment::resegment_region` — this command only wraps it with the
/// same take/restore-mesh + progress-emit pattern as `auto_segment_v2` so the
/// heavy work runs off the main thread and does not freeze the webview.
#[tauri::command]
pub async fn resegment_region(
    label: u32,
    algorithm: SegmentationAlgorithm,
    app: tauri::AppHandle,
    state: State<'_, AppState>,
) -> Result<SegmentResult, String> {
    log::info!(
        "[cmd:resegment_region] label={} algorithm={:?}",
        label,
        algorithm
    );

    let app_for_progress = app.clone();
    let progress_cb: Box<ProgressFn> = Box::new(move |fraction: f32, stage: &str| {
        let _ = app_for_progress.emit(
            "segment-progress",
            serde_json::json!({ "progress": fraction, "stage": stage }),
        );
    });

    // Take the mesh out of the shared Mutex for the (potentially long) sub-mesh
    // segmentation — mirror auto_segment_v2's rationale.
    let mut mesh_guard = state.mesh.lock().map_err(|e| e.to_string())?;
    let mut mesh = mesh_guard.take().ok_or("No mesh loaded")?;
    drop(mesh_guard);

    let inner = resegment_region_impl(&mut mesh, label, &algorithm, &*progress_cb)
        .map_err(|e| e.to_string())?;

    let _ = app.emit(
        "segment-progress",
        serde_json::json!({ "progress": 1.0, "stage": "done" }),
    );
    log::info!(
        "[cmd:resegment_region] done: {} sub-regions ({} faces moved)",
        inner.region_count,
        inner.moved_faces
    );

    // Re-segmentation is a local edit, so unlike auto_segment_v2 it must NOT
    // clear history — `resegment_region_impl` already recorded an `OpKind::Split`
    // undo op, letting the user undo the refine just like a split.
    let labels = mesh.segment_labels.clone();
    let mut mesh_guard = state.mesh.lock().map_err(|e| e.to_string())?;
    *mesh_guard = Some(mesh);

    Ok(SegmentResult {
        segments: inner.segments,
        segment_labels: labels,
        face_colors: Vec::new(),
    })
}

/// Seeded watershed segmentation (iteration 50): the user drops a few seed points
/// (one per region) and the algorithm grows each into a region using feature-edge
/// barriers + geodesic nearest-seed Voronoi, with a fallback that fills any
/// unseeded patch from the geodesic-nearest seed across barriers.
#[tauri::command]
pub fn seed_grow(
    seeds: Vec<SeedInput>,
    barrier_deg: f32,
    optimizer: bool,
    app: tauri::AppHandle,
    state: State<'_, AppState>,
) -> Result<SegmentResult, String> {
    log::info!(
        "[cmd:seed_grow] seeds={} barrier_deg={:.1} optimizer={}",
        seeds.len(),
        barrier_deg,
        optimizer
    );

    let params = SeedGrowParams {
        barrier_deg,
        optimizer,
    };

    let mut mesh_guard = state.mesh.lock().map_err(|e| e.to_string())?;
    let mesh = mesh_guard.as_mut().ok_or("No mesh loaded")?;

    let inner = backend_seed_grow(mesh, &seeds, &params).map_err(|e| e.to_string())?;

    let _ = app.emit(
        "segment-progress",
        serde_json::json!({ "progress": 1.0, "stage": "done" }),
    );
    log::info!(
        "[cmd:seed_grow] done: {} regions ({} faces moved)",
        inner.region_count,
        inner.moved_faces
    );

    Ok(SegmentResult {
        segments: inner.segments,
        segment_labels: inner.segment_labels,
        face_colors: flatten_face_colors(&mesh.face_colors),
    })
}

/// Suggest seed locations for the seeded-watershed tool (iteration 52). Returns
/// up to `count` candidate points spread across the mesh and biased toward
/// region interiors (weighted farthest-point sampling over face centroids — see
/// `segment::recommend`). These are ADVISORY: the frontend shows them as ghost
/// markers the user accepts (click → becomes a real seed) or ignores. They do
/// not participate in `seed_grow` until accepted. Read-only: the mesh is never
/// mutated, so no history interaction is needed.
#[tauri::command]
pub fn recommend_seeds(
    count: usize,
    state: State<'_, AppState>,
) -> Result<Vec<SeedSuggestion>, String> {
    log::info!("[cmd:recommend_seeds] count={}", count);
    let mesh_guard = state.mesh.lock().map_err(|e| e.to_string())?;
    let mesh = mesh_guard.as_ref().ok_or("No mesh loaded")?;
    let suggestions = backend_recommend_seeds(mesh, count);
    log::info!("[cmd:recommend_seeds] done: {} suggestions", suggestions.len());
    Ok(suggestions)
}
