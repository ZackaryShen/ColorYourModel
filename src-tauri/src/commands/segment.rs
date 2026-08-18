use tauri::{Emitter, State};
use serde::{Deserialize, Serialize};

use crate::commands::mesh::AppState;
use crate::mesh::loader::ProgressFn;
use crate::mesh::model::{MeshModel, Segment, DEFAULT_FACE_COLOR};
use crate::segment::dihedral::segment_by_dihedral_angle;
use crate::segment::manual::{
    finalize_manual_region as backend_finalize_manual_region, snap_point_to_vertex_on_face,
    MANUAL_SEGMENT_OFFSET,
};
use crate::segment::sdf::segment_by_sdf;
use crate::segment::split::{split_segment as split_segment_impl, SplitMethod, SplitResult};
use crate::segment::resegment::resegment_region as resegment_region_impl;
use crate::segment::recommend::{
    recommend_seeds as backend_recommend_seeds, RecommendWeights, SeedSuggestion,
};
use crate::segment::planar::{
    detect_planar_regions as backend_detect_planar_regions, PlanarParams, PlanarRegion,
};
use crate::segment::eye::{
    detect_eye_regions as backend_detect_eye_regions, EyeRegion,
};
use crate::segment::multiview::{
    detect_multiview_regions as backend_detect_multiview_regions, MultiViewParams, MultiViewRegion,
};
use crate::segment::cross_section::{
    detect_cross_section_features as backend_detect_cross_section_features, CrossSectionRegion,
};
use crate::segment::fuse::fuse_region_sets;
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

/// Wipe the current segmentation AND all face paint, returning the mesh to its
/// freshly-loaded "uncoloured" state. This is the missing "I don't like the
/// auto-segment, let me start over" affordance (REFUTE iteration 57): previously
/// the only escape was re-loading the model, because `auto_segment_v2` clears the
/// undo stack and there was no dedicated reset command.
///
/// Kept as a free function so it is unit-testable without standing up an
/// `AppState`/`Mutex`. The command below is a thin wrapper around it.
pub(crate) fn reset_mesh_state(mesh: &mut MeshModel) {
    let n = mesh.faces.len();
    mesh.segment_labels = vec![0u32; n];
    mesh.segments.clear();
    mesh.segment_names.clear();
    mesh.next_manual_label = MANUAL_SEGMENT_OFFSET;
    mesh.face_colors = vec![DEFAULT_FACE_COLOR; n];
    mesh.history.clear();
}

#[tauri::command]
pub fn reset_segmentation(state: State<AppState>) -> Result<SegmentResult, String> {
    let mut mesh_guard = state.mesh.lock().map_err(|e| e.to_string())?;
    let mesh = mesh_guard.as_mut().ok_or("No mesh loaded")?;
    reset_mesh_state(mesh);
    let labels = mesh.segment_labels.clone();
    let face_colors = flatten_face_colors(&mesh.face_colors);
    Ok(SegmentResult {
        segments: Vec::new(),
        segment_labels: labels,
        face_colors,
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
    curvature: f32,
    concavity: f32,
    state: State<'_, AppState>,
) -> Result<Vec<SeedSuggestion>, String> {
    log::info!(
        "[cmd:recommend_seeds] count={} curvature={} concavity={}",
        count,
        curvature,
        concavity
    );
    let mesh_guard = state.mesh.lock().map_err(|e| e.to_string())?;
    let mesh = mesh_guard.as_ref().ok_or("No mesh loaded")?;
    let weights = RecommendWeights {
        curvature,
        concavity,
    };
    let suggestions = backend_recommend_seeds(mesh, count, weights);
    log::info!("[cmd:recommend_seeds] done: {} suggestions", suggestions.len());
    Ok(suggestions)
}

/// Layer 1 of the planar-region fusion study (`docs/09`): detect the mesh's
/// continuous planar regions and return them as ADVISORY seed suggestions.
///
/// Each returned [`PlanarRegion`] carries a representative interior `seed`
/// (a `SeedSuggestion` the frontend can drop straight into the existing
/// ghost-suggestion → `seed_grow` union path, iter58-64) plus `boundary_edges`
/// so the flat patch can be outlined on the model. Read-only: the mesh is never
/// mutated. Like `recommend_seeds`, a region is purely advisory — the user
/// accepts it (click → becomes a real seed) or ignores it, and nothing reaches
/// `seed_grow` until accepted.
#[tauri::command]
pub fn detect_planar_regions(
    angle_threshold_deg: f32,
    dist_thr_factor: f32,
    min_region_faces: u32,
    state: State<'_, AppState>,
) -> Result<Vec<PlanarRegion>, String> {
    log::info!(
        "[cmd:detect_planar_regions] angle={:.1}° dist_factor={:.4} min_faces={}",
        angle_threshold_deg,
        dist_thr_factor,
        min_region_faces
    );
    let mesh_guard = state.mesh.lock().map_err(|e| e.to_string())?;
    let mesh = mesh_guard.as_ref().ok_or("No mesh loaded")?;
    let params = crate::segment::planar::PlanarParams {
        angle_thr_deg: angle_threshold_deg,
        dist_thr_factor,
        min_region_faces: min_region_faces as usize,
    };
    let regions = backend_detect_planar_regions(mesh, &params);
    log::info!(
        "[cmd:detect_planar_regions] done: {} planar regions",
        regions.len()
    );
    Ok(regions)
}

/// Eye-region semantic detection (docs/10 REVISE): given a user ROI that bounds
/// an eye, classify its faces into socket / eyelid / globe / sclera and return
/// them as advisory regions with boundary edges. Read-only — the mesh is never
/// mutated, mirroring `detect_planar_regions`.
#[tauri::command]
pub fn detect_eye_regions(roi_faces: Vec<u32>, state: State<AppState>) -> Result<Vec<EyeRegion>, String> {
    log::info!("[cmd:detect_eye_regions] roi_faces={}", roi_faces.len());
    let mesh_guard = state.mesh.lock().map_err(|e| e.to_string())?;
    let mesh = mesh_guard.as_ref().ok_or("No mesh loaded")?;
    let regions = backend_detect_eye_regions(mesh, &roi_faces);
    log::info!(
        "[cmd:detect_eye_regions] done: {} eye regions",
        regions.len()
    );
    Ok(regions)
}

/// Layer 3 of the planar-region fusion study (`docs/09`): the **machine-vision
/// (MultiView 3→2→3) evidence channel**. Detects coherent regions by projecting
/// the mesh from multiple views, growing 2D-connected regions per view, then
/// back-projecting and cutting a weighted face–face match graph.
///
/// Each returned [`MultiViewRegion`] carries a representative interior `seed`
/// (a `SeedSuggestion` the frontend can drop straight into the existing
/// ghost-suggestion → `seed_grow` union path, iter58-64) plus `boundary_edges`
/// so the cluster can be outlined on the model. Read-only: the mesh is never
/// mutated. Like `recommend_seeds` / `detect_planar_regions`, a region is purely
/// advisory — the user accepts it (click → becomes a real seed) or ignores it.
#[tauri::command]
pub fn detect_multiview_regions(
    view_count: u32,
    angle_threshold_deg: f32,
    min_region_faces: u32,
    match_threshold: u32,
    state: State<'_, AppState>,
) -> Result<Vec<MultiViewRegion>, String> {
    log::info!(
        "[cmd:detect_multiview_regions] views={} angle={:.1}° min_faces={} match={}",
        view_count,
        angle_threshold_deg,
        min_region_faces,
        match_threshold
    );
    let mesh_guard = state.mesh.lock().map_err(|e| e.to_string())?;
    let mesh = mesh_guard.as_ref().ok_or("No mesh loaded")?;
    let params = crate::segment::multiview::MultiViewParams {
        view_count: view_count as usize,
        angle_thr_deg: angle_threshold_deg,
        min_region_faces: min_region_faces as usize,
        match_threshold: match_threshold as usize,
    };
    let regions = backend_detect_multiview_regions(mesh, &params);
    log::info!(
        "[cmd:detect_multiview_regions] done: {} multiview regions",
        regions.len()
    );
    Ok(regions)
}

/// Layer 2 of the planar-region fusion study (`docs/09`): the **ray / plane
/// enhancement evidence channel**. Marches a slice plane along each principal
/// axis and reports the *feature cross-sections* — positions where the
/// cross-sectional profile changes sharply (the "面积/轮廓对路径的导数超过阈值
/// 处即特征边界" criterion from docs/09 §1.2).
///
/// Each returned [`CrossSectionRegion`] carries the slice plane, its position,
/// the profile metric, a generalized-winding-number inside/outside confidence
/// (Jacobson 2013, stays well-defined on open meshes), and `boundary_edges` —
/// the actual 3D contour of the cross-section, drawn as `lineSegments`. This is
/// **visual-only evidence**: it never mutates the mesh and is NOT folded into
/// `seed_grow` (a slice is a plane, not a face — docs/09 Layer 2 = "不裁决").
#[tauri::command]
pub fn detect_cross_section_features(
    planes_per_axis: u32,
    feature_threshold: f32,
    state: State<'_, AppState>,
) -> Result<Vec<CrossSectionRegion>, String> {
    log::info!(
        "[cmd:detect_cross_section_features] planes/axis={} feature_threshold={:.2}",
        planes_per_axis,
        feature_threshold
    );
    let mesh_guard = state.mesh.lock().map_err(|e| e.to_string())?;
    let mesh = mesh_guard.as_ref().ok_or("No mesh loaded")?;
    let params = crate::segment::cross_section::CrossSectionParams {
        planes_per_axis: planes_per_axis as usize,
        feature_threshold,
    };
    let regions = backend_detect_cross_section_features(mesh, &params);
    log::info!(
        "[cmd:detect_cross_section_features] done: {} feature cross-sections",
        regions.len()
    );
    Ok(regions)
}

/// Layer 4 (docs/09): fuse the planar (Layer 1) and multiview (Layer 3) region
/// *memberships* into one partition via edge-level majority vote, then commit it
/// to the mesh (manual labels + colour + one undo entry) — the same落盘 path as
/// `seed_grow`, but the region boundaries now come from the algorithms' actual
/// face membership instead of a re-grown Voronoi over seed points.
///
/// This is the fix for the user's "the algorithms sketch useful regions but the
/// real partition is still just seeds" complaint. Detection parameters are the
/// same defaults the SeedPanel already uses (planar 15° / M·1/30, multiview 12
/// views / 20° / match 1); only the fusion knobs are exposed:
/// `cut_threshold` (edge cut vote margin, 1 = cut must outvote keep) and
/// `min_region_faces` (post-fusion tiny-region filter, 0 = auto).
#[tauri::command]
pub async fn fuse_segmentation(
    cut_threshold: i32,
    min_region_faces: u32,
    dihedral_deg: f32,
    eye_face_indices: Option<Vec<Vec<u32>>>,
    app: tauri::AppHandle,
    state: State<'_, AppState>,
) -> Result<SegmentResult, String> {
    log::info!(
        "[cmd:fuse_segmentation] cut_threshold={} min_region_faces={} dihedral_deg={:.1} eye_sets={}",
        cut_threshold,
        min_region_faces,
        dihedral_deg,
        eye_face_indices.as_ref().map(|v| v.len()).unwrap_or(0)
    );

    // Take the mesh out for the whole compute (mirror auto_segment_v2) so a
    // concurrent sync command cannot stall on the lock.
    let mut mesh_guard = state.mesh.lock().map_err(|e| e.to_string())?;
    let mut mesh = mesh_guard.take().ok_or("No mesh loaded")?;
    drop(mesh_guard);

    // No-op progress callback: the detectors we call here are read-only and the
    // heavy commit happens in `fuse_region_sets` (which emits its own done
    // event below).
    let progress_cb: Box<ProgressFn> = Box::new(|_, _| {});

    let n = mesh.faces.len();
    let detect_min = (n / 500).max(2);

    // Layer 1: geometric backbone (裁决源).
    let planar_regions = backend_detect_planar_regions(
        &mesh,
        &PlanarParams {
            angle_thr_deg: 15.0,
            dist_thr_factor: 1.0 / 30.0,
            min_region_faces: detect_min,
        },
    );
    // Layer 3: machine-vision evidence channel.
    let multiview_regions = backend_detect_multiview_regions(
        &mesh,
        &MultiViewParams {
            view_count: 12,
            angle_thr_deg: 20.0,
            min_region_faces: detect_min,
            match_threshold: 1,
        },
    );

    // Layer 0: geometry backbone — dihedral crease vote. On smooth / single-
    // colour / organic meshes planar+multiview return little or nothing, so
    // without this the fused partition collapses to one region (cat-model
    // diagnosis: planar=0, multiview=125 disconnected islands → 1 fused
    // region). Dihedral creases (leg–body, ear–head, tail–base) cut regardless
    // of colour/flatness, giving the edge vote real signal. Computed
    // transiently: we read the per-face labels into region sets, then reset the
    // mesh's labels so `fuse_region_sets` re-labels everything itself.
    let dihedral_sets: Vec<Vec<u32>> = {
        let _segs = segment_by_dihedral_angle(&mut mesh, dihedral_deg, &*progress_cb);
        let mut map: std::collections::HashMap<u32, Vec<u32>> = std::collections::HashMap::new();
        for (i, &l) in mesh.segment_labels.iter().enumerate() {
            map.entry(l).or_default().push(i as u32);
        }
        let sets: Vec<Vec<u32>> = map.into_values().collect();
        let nfaces = mesh.segment_labels.len();
        mesh.segment_labels = vec![0u32; nfaces];
        mesh.segments.clear();
        mesh.segment_names.clear();
        sets
    };

    let planar_sets: Vec<Vec<u32>> = planar_regions
        .iter()
        .map(|r| r.face_indices.clone())
        .collect();
    let multiview_sets: Vec<Vec<u32>> = multiview_regions
        .iter()
        .map(|r| r.face_indices.clone())
        .collect();
    // Layer 4.5: user-confirmed semantic eye regions (`detect_eye_regions`).
    // The frontend passes the regions it has on screen; each is a per-semantic
    // list of face indices (globe / sclera / eyelid / socket). Empty / missing
    // means "no eye regions in scope" — the fuse stays a 3-channel vote.
    let eye_sets: Vec<Vec<u32>> = eye_face_indices.unwrap_or_default();

    let result = fuse_region_sets(
        &mut mesh,
        &planar_sets,
        &multiview_sets,
        &dihedral_sets,
        &eye_sets,
        cut_threshold,
        min_region_faces as usize,
    )
    .map_err(|e| e.to_string())?;

    let _ = app.emit(
        "segment-progress",
        serde_json::json!({ "progress": 1.0, "stage": "done" }),
    );
    log::info!(
        "[cmd:fuse_segmentation] done: {} regions ({} faces moved)",
        result.region_count,
        result.moved_faces
    );
    // Surface a structured breakdown to the UI so the user can read *why* the
    // button produced e.g. 535 regions on a smooth model and tweak the knobs.
    // Also written to the log; emitted on `fused-debug` (camelCase) for the
    // frontend to pick up.
    let _ = app.emit(
        "fuse-debug",
        serde_json::json!({
            "stage": "fuse-done",
            "channels": {
                "planar": planar_sets.len(),
                "multiview": multiview_sets.len(),
                "dihedral": dihedral_sets.len(),
                "eye": eye_sets.len(),
            },
            "cutThreshold": cut_threshold,
            "minRegionFaces": min_region_faces,
            "rawComponents": result.region_count,
            "edgeTotal": result.edge_total.unwrap_or(0),
            "edgeCut": result.edge_cut.unwrap_or(0),
            "regionSizeMin": result.region_size_min,
            "regionSizeMax": result.region_size_max,
            "regionSizeMedian": result.region_size_median,
        }),
    );

    let labels = result.segment_labels.clone();
    let face_colors = flatten_face_colors(&mesh.face_colors);
    let mut mesh_guard = state.mesh.lock().map_err(|e| e.to_string())?;
    *mesh_guard = Some(mesh);

    Ok(SegmentResult {
        segments: result.segments,
        segment_labels: labels,
        face_colors,
    })
}

#[cfg(test)]
mod reset_tests {
    use super::*;
    use crate::mesh::model::MeshModel;

    fn tiny_mesh() -> MeshModel {
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
        m
    }

    #[test]
    fn reset_mesh_state_neutralises_everything() {
        let mut m = tiny_mesh();
        // Simulate a fully painted, multi-region state.
        m.face_colors = vec![[255, 0, 0, 255]; 4];
        m.segment_labels = vec![1, 2, 2, 1];
        m.segments.insert(
            1,
            Segment {
                id: 1,
                name: "Left arm".to_string(),
                color: None,
                face_count: 2,
            },
        );
        m.segment_names.insert(1, "Left arm".to_string());

        reset_mesh_state(&mut m);

        assert!(
            m.face_colors.iter().all(|c| *c == DEFAULT_FACE_COLOR),
            "face colours must return to neutral grey"
        );
        assert!(
            m.segment_labels.iter().all(|&l| l == 0),
            "all labels must be zeroed"
        );
        assert!(m.segments.is_empty(), "segments map must be cleared");
        assert!(m.segment_names.is_empty(), "segment names must be cleared");
    }
}
