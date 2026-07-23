use std::collections::HashMap;

use tauri::{Emitter, State};
use serde::{Deserialize, Serialize};

use crate::commands::mesh::AppState;
use crate::mesh::loader::ProgressFn;
use crate::mesh::model::Segment;
use crate::segment::dihedral::segment_by_dihedral_angle;

/// Result of auto-segmentation: segment metadata + per-face labels
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SegmentResult {
    pub segments: Vec<Segment>,
    pub segment_labels: Vec<u32>,
}

/// Manual segment label offset — separates manual labels from auto-segment labels.
/// Auto-segment uses labels 0..N; manual labels start at 100000.
/// This prevents conflicts when auto_segment is re-run after manual segmentation.
const MANUAL_SEGMENT_OFFSET: u32 = 100_000;

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

    // Emit completion
    let _ = app.emit(
        "segment-progress",
        serde_json::json!({ "progress": 1.0, "stage": format!("Found {} regions", segments.len()) }),
    );

    log::info!("[cmd:auto_segment] done: {} segments", segments.len());

    Ok(SegmentResult {
        segments,
        segment_labels: mesh.segment_labels.clone(),
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

    // Assign label + color to this face
    mesh.segment_labels[face_id as usize] = label;
    mesh.face_colors[face_id as usize] = color;

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

    // Count faces per label
    let mut label_counts: HashMap<u32, u32> = HashMap::new();
    for &label in &mesh.segment_labels {
        *label_counts.entry(label).or_insert(0) += 1;
    }

    // Generate color for this segment
    let color_seed = segment_label.wrapping_mul(2654435761) >> 24;
    let seg_color: [u8; 4] = [
        ((color_seed * 73) % 200 + 55) as u8,
        ((color_seed * 151) % 200 + 55) as u8,
        ((color_seed * 223) % 200 + 55) as u8,
        255,
    ];

    // Build full segment list
    let mut segments: Vec<Segment> = label_counts
        .iter()
        .map(|(&label, &count)| Segment {
            id: label,
            name: format!(
                "Region {}",
                if label >= MANUAL_SEGMENT_OFFSET {
                    label - MANUAL_SEGMENT_OFFSET + 1
                } else {
                    label + 1
                }
            ),
            color: if label >= MANUAL_SEGMENT_OFFSET {
                Some(seg_color)
            } else {
                None
            },
            face_count: count,
        })
        .collect();
    segments.sort_by_key(|s| s.id);

    // Update mesh.segments as HashMap<u32, Segment>
    mesh.segments = segments.iter().map(|s| (s.id, s.clone())).collect();

    log::info!(
        "[cmd:finalize_segment] label={}, faces={}, total_segments={}",
        segment_label,
        label_counts.get(&segment_label).unwrap_or(&0),
        segments.len()
    );

    Ok(FinalizeSegmentResult {
        segments,
        segment_labels: mesh.segment_labels.clone(),
    })
}
