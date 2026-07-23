use crate::mesh::model::MeshModel;
use crate::segment::flood_fill::flood_fill;

/// Fill an entire segment or flood-fill region with a target color
pub fn fill_region(mesh: &mut MeshModel, start_face: u32, color: [u8; 4]) -> Vec<u32> {
    let faces = flood_fill(mesh, start_face);
    for &fid in &faces {
        mesh.face_colors[fid as usize] = color;
    }
    faces
}

/// Fill all faces of a specific segment
pub fn fill_segment(mesh: &mut MeshModel, segment_id: u32, color: [u8; 4]) -> Vec<u32> {
    let faces: Vec<u32> = mesh
        .segment_labels
        .iter()
        .enumerate()
        .filter(|(_, &label)| label == segment_id)
        .map(|(i, _)| i as u32)
        .collect();
    for &fid in &faces {
        mesh.face_colors[fid as usize] = color;
    }
    faces
}
