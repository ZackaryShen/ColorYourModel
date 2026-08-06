//! Face selection for the fill tool.
//!
//! These used to select *and* write the colour. Writing moved out to
//! `MeshModel::apply_paint` so that every colour mutation in the backend goes
//! through one place and therefore lands in the undo history — a second write
//! path is exactly how an operation ends up being unundoable.

use crate::mesh::model::MeshModel;
use crate::segment::flood_fill::flood_fill;

/// Faces reachable from `start_face` by flood fill (one connected region).
pub fn region_faces(mesh: &MeshModel, start_face: u32) -> Vec<u32> {
    flood_fill(mesh, start_face)
}

/// Faces belonging to a specific segment.
pub fn segment_faces(mesh: &MeshModel, segment_id: u32) -> Vec<u32> {
    mesh.segment_labels
        .iter()
        .enumerate()
        .filter(|(_, &label)| label == segment_id)
        .map(|(i, _)| i as u32)
        .collect()
}
