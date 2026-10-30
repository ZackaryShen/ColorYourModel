//! `undo` / `redo` commands over the unified backend history.
//!
//! These replaced three separate frontend-driven mechanisms (a full-snapshot
//! stack in the store, a `restore_face_colors` command and a `manual_region_undo`
//! command), all removed in S2. The frontend no longer owns any part of the
//! timeline; it sends a keystroke and receives a patch.

use serde::{Deserialize, Serialize};
use tauri::State;

use crate::commands::mesh::AppState;
use crate::mesh::model::Segment;

/// Above this many faces the incremental path stops being worth it.
///
/// Not a GPU-upload argument — a sparse patch is still cheaper to upload than a
/// whole buffer. The binding cost is the IPC hop: Tauri serialises the payload
/// as JSON, so 50 000 faces plus their colours is already several megabytes of
/// text to encode, ship and parse. Past that, sending the buffers the frontend
/// would rebuild anyway is both simpler and faster.
///
/// Known limitation: below the threshold the frontend patches with a single
/// `addUpdateRange` spanning `[min, max]`, so a two-face delta at opposite ends
/// of the model still re-uploads everything between them. Chunking that range
/// is frontend work (S2) and does not change the contract here.
const MAX_INCREMENTAL_FACES: usize = 50_000;

/// The result of an undo or redo.
///
/// Two shapes in one struct, selected by `full`:
///
/// - `full == false` — colour-only patch. `faces` + `colors` carry the delta;
///   the frontend writes them into the existing buffer.
/// - `full == true` — the frontend must replace `segments`, `segmentLabels` and
///   `faceColors` wholesale.
///
/// The full form is mandatory whenever labels moved. The segment view memoises
/// on the mesh-data object identity, so patching labels in place would restore
/// the data correctly and then fail to repaint anything.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HistoryResult {
    /// `false` when the stack was empty and nothing happened.
    pub applied: bool,
    /// Which of the two payload shapes below is populated.
    pub full: bool,

    /// Incremental form: faces whose colour changed.
    pub faces: Vec<u32>,
    /// Incremental form: flat RGBA, four bytes per entry in `faces`.
    pub colors: Vec<u8>,

    /// Full form: rebuilt segment metadata.
    pub segments: Option<Vec<Segment>>,
    /// Full form: every per-face label.
    pub segment_labels: Option<Vec<u32>>,
    /// Full form: every per-face colour, flat RGBA.
    pub face_colors: Option<Vec<u8>>,

    /// 0.2.0-P2 transform form: the post-apply vertices, flat xyz per vertex.
    /// `Some` ⟺ the applied entry was a whole-model transform; the frontend
    /// must rebuild its geometry from these (the incremental colour patch
    /// cannot express vertex movement).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vertices: Option<Vec<f32>>,
    /// Transform form: the recomputed bounding box, paired with `vertices`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bbox: Option<crate::mesh::model::BoundingBox>,

    pub can_undo: bool,
    pub can_redo: bool,
}

impl HistoryResult {
    /// Nothing to undo or redo. Still reports the stack state so the toolbar
    /// buttons converge even if a previous response was dropped.
    fn empty(can_undo: bool, can_redo: bool) -> Self {
        Self {
            applied: false,
            full: false,
            faces: Vec::new(),
            colors: Vec::new(),
            segments: None,
            segment_labels: None,
            face_colors: None,
            vertices: None,
            bbox: None,
            can_undo,
            can_redo,
        }
    }
}

fn flatten(colors: &[[u8; 4]]) -> Vec<u8> {
    colors.iter().flat_map(|c| c.iter().copied()).collect()
}

/// `Direction::Undo` and `Direction::Redo` differ only in which stack they pop,
/// so the whole body is shared.
#[derive(Debug, Clone, Copy)]
enum Direction {
    Undo,
    Redo,
}

fn step(state: State<AppState>, dir: Direction) -> Result<HistoryResult, String> {
    let mut mesh_guard = state.mesh.lock().map_err(|e| e.to_string())?;
    let mesh = mesh_guard.as_mut().ok_or("No mesh loaded")?;

    // Split the borrow: the history and the buffers it mutates are separate
    // fields of the same struct, which the borrow checker cannot see through a
    // method call.
    let crate::mesh::model::MeshModel {
        history,
        face_colors,
        segment_labels,
        vertices,
        ..
    } = mesh;

    let outcome = match dir {
        Direction::Undo => history.undo(face_colors, segment_labels, vertices),
        Direction::Redo => history.redo(face_colors, segment_labels, vertices),
    };

    let Some(outcome) = outcome else {
        return Ok(HistoryResult::empty(mesh.history.can_undo(), mesh.history.can_redo()));
    };

    // Gradient membership repair: undo can move faces OFF a gradient line
    // (they leave the merged swatch), redo moves them back ON. Scoped to the
    // faces the history step actually touched, so a hand-painted line colour
    // is never mistaken for gradient provenance. Safe to call through `mesh`
    // — the field borrows split above ended with the undo/redo call.
    mesh.sync_gradient_faces(&outcome.faces);

    // 0.2.0-P2: a transform entry moves VERTICES — after applying it, the
    // derived geometry state must be rebuilt and the new vertices shipped to
    // the frontend (the incremental colour patch cannot express this).
    let transformed = outcome.transform.is_some();
    if transformed {
        mesh.compute_normals();
        mesh.compute_bbox();
        mesh.build_kdtree();
        mesh.build_vertex_kdtree();
        mesh.build_adjacency();
    }

    Ok(history_result(
        mesh,
        match dir {
            Direction::Undo => "undo",
            Direction::Redo => "redo",
        },
        outcome.kind,
        outcome.faces,
        outcome.colors,
        outcome.labels_changed,
        transformed,
    ))
}

/// Assemble the wire result for a timeline move. Undo, redo and jump differ
/// only in WHICH entries were applied — the payload semantics are shared:
/// `labels_changed` recomputes segment metadata, `transformed` ships the
/// post-apply vertices (callers must have rebuilt the derived geometry state
/// beforehand), and past [`MAX_INCREMENTAL_FACES`] the whole-buffer form wins
/// on IPC cost, as do both of the first two.
fn history_result(
    mesh: &mut crate::mesh::model::MeshModel,
    what: &str,
    kind: crate::mesh::history::OpKind,
    faces: Vec<u32>,
    colors: Vec<[u8; 4]>,
    labels_changed: bool,
    transformed: bool,
) -> HistoryResult {
    let full = transformed || labels_changed || faces.len() > MAX_INCREMENTAL_FACES;

    if labels_changed {
        // Segment metadata is derived from labels, so it has to be recomputed
        // before it is sent. A region whose faces all reverted disappears here.
        mesh.rebuild_segments();
    }

    log::info!(
        "[cmd:history] {} kind={:?} faces={} labels_changed={} full={}",
        what,
        kind,
        faces.len(),
        labels_changed,
        full
    );

    HistoryResult {
        applied: true,
        full,
        faces: if full || transformed { Vec::new() } else { faces },
        colors: if full || transformed {
            Vec::new()
        } else {
            flatten(&colors)
        },
        segments: full.then(|| mesh.sorted_segments()),
        segment_labels: full.then(|| mesh.segment_labels.clone()),
        face_colors: full.then(|| flatten(&mesh.face_colors)),
        vertices: transformed.then(|| {
            mesh.vertices
                .iter()
                .flat_map(|v| v.iter().copied())
                .collect()
        }),
        bbox: transformed.then(|| mesh.bbox.clone()),
        can_undo: mesh.history.can_undo(),
        can_redo: mesh.history.can_redo(),
    }
}

#[tauri::command]
pub fn undo(state: State<AppState>) -> Result<HistoryResult, String> {
    step(state, Direction::Undo)
}

#[tauri::command]
pub fn redo(state: State<AppState>) -> Result<HistoryResult, String> {
    step(state, Direction::Redo)
}

/// Stack state without touching it, for enabling the toolbar buttons on load.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HistoryState {
    pub can_undo: bool,
    pub can_redo: bool,
    pub undo_depth: usize,
    pub redo_depth: usize,
    pub bytes: usize,
}

#[tauri::command]
pub fn history_state(state: State<AppState>) -> Result<HistoryState, String> {
    let mesh_guard = state.mesh.lock().map_err(|e| e.to_string())?;
    let mesh = mesh_guard.as_ref().ok_or("No mesh loaded")?;
    Ok(HistoryState {
        can_undo: mesh.history.can_undo(),
        can_redo: mesh.history.can_redo(),
        undo_depth: mesh.history.undo_depth(),
        redo_depth: mesh.history.redo_depth(),
        bytes: mesh.history.bytes(),
    })
}

// ─── History panel (issue #22) ─────────────────────────────────────────────

/// One row of the history panel: what the operation was and how much it
/// touched. `kind` is the stable wire key from `OpKind::key()`; the frontend
/// maps it to icon + localized label.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HistoryEntryDto {
    pub kind: String,
    pub stroke_id: Option<u64>,
    pub face_count: usize,
    pub bytes: usize,
}

/// Everything the panel renders, in one read-only call. `current` is the
/// jump index of the live state; `max_depth` and `bytes` feed the header so
/// the eviction ceiling (`DEFAULT_MAX_DEPTH` / `DEFAULT_BYTE_BUDGET`) stops
/// being invisible.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HistoryEntriesResult {
    pub current: usize,
    pub max_depth: usize,
    pub bytes: usize,
    /// Applied entries, oldest first; depth k = the state after `entries[k-1]`.
    pub entries: Vec<HistoryEntryDto>,
}

#[tauri::command]
pub fn history_entries(state: State<AppState>) -> Result<HistoryEntriesResult, String> {
    let mesh_guard = state.mesh.lock().map_err(|e| e.to_string())?;
    let mesh = mesh_guard.as_ref().ok_or("No mesh loaded")?;
    Ok(HistoryEntriesResult {
        current: mesh.history.undo_depth(),
        max_depth: mesh.history.max_depth(),
        bytes: mesh.history.bytes(),
        entries: mesh
            .history
            .entries()
            .into_iter()
            .map(|e| HistoryEntryDto {
                kind: e.kind.key().to_string(),
                stroke_id: e.stroke_id,
                face_count: e.face_count,
                bytes: e.bytes,
            })
            .collect(),
    })
}

/// Jump to state `target` (issue #22): applies every skipped step in ONE
/// call and returns the patch for the final state — the union of all touched
/// faces with their final colours, full-payload whenever labels moved or a
/// transform ran, exactly like a single undo/redo would produce.
#[tauri::command]
pub fn history_jump(target: usize, state: State<AppState>) -> Result<HistoryResult, String> {
    let mut mesh_guard = state.mesh.lock().map_err(|e| e.to_string())?;
    let mesh = mesh_guard.as_mut().ok_or("No mesh loaded")?;

    let crate::mesh::model::MeshModel {
        history,
        face_colors,
        segment_labels,
        vertices,
        ..
    } = mesh;

    let Some(out) = history.jump_to(target, face_colors, segment_labels, vertices) else {
        return Ok(HistoryResult::empty(mesh.history.can_undo(), mesh.history.can_redo()));
    };

    // Gradient membership repair, same contract as undo/redo above: the
    // jump's union of touched faces is re-judged against the gradient lines
    // at the destination state.
    mesh.sync_gradient_faces(&out.faces);

    // A transform step moved VERTICES: the derived geometry state must be
    // rebuilt once here, at the jump's destination, before the payload reads it.
    if out.transformed {
        mesh.compute_normals();
        mesh.compute_bbox();
        mesh.build_kdtree();
        mesh.build_vertex_kdtree();
        mesh.build_adjacency();
    }

    Ok(history_result(
        mesh,
        "jump",
        out.kind,
        out.faces,
        out.colors,
        out.labels_changed,
        out.transformed,
    ))
}
