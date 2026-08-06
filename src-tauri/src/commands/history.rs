//! `undo` / `redo` commands over the unified backend history.
//!
//! These replace three separate frontend-driven mechanisms (a full-snapshot
//! stack in the store, `restore_face_colors`, and `manual_region_undo`). The
//! frontend no longer owns any part of the timeline; it sends a keystroke and
//! receives a patch.

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
        ..
    } = mesh;

    let outcome = match dir {
        Direction::Undo => history.undo(face_colors, segment_labels),
        Direction::Redo => history.redo(face_colors, segment_labels),
    };

    let Some(outcome) = outcome else {
        return Ok(HistoryResult::empty(mesh.history.can_undo(), mesh.history.can_redo()));
    };

    let full = outcome.labels_changed || outcome.faces.len() > MAX_INCREMENTAL_FACES;

    if outcome.labels_changed {
        // Segment metadata is derived from labels, so it has to be recomputed
        // before it is sent. A region whose faces all reverted disappears here.
        mesh.rebuild_segments();
    }

    log::info!(
        "[cmd:history] {} kind={:?} faces={} labels_changed={} full={}",
        match dir {
            Direction::Undo => "undo",
            Direction::Redo => "redo",
        },
        outcome.kind,
        outcome.faces.len(),
        outcome.labels_changed,
        full
    );

    Ok(HistoryResult {
        applied: true,
        full,
        faces: if full { Vec::new() } else { outcome.faces },
        colors: if full {
            Vec::new()
        } else {
            flatten(&outcome.colors)
        },
        segments: full.then(|| mesh.segments.values().cloned().collect()),
        segment_labels: full.then(|| mesh.segment_labels.clone()),
        face_colors: full.then(|| flatten(&mesh.face_colors)),
        can_undo: mesh.history.can_undo(),
        can_redo: mesh.history.can_redo(),
    })
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
