use std::collections::VecDeque;

use petgraph::visit::EdgeRef;

use crate::mesh::model::MeshModel;

/// Flood fill from a starting face, returning all connected face indices
/// that share the same segment label (or all connected faces if no segmentation).
pub fn flood_fill(mesh: &MeshModel, start_face: u32) -> Vec<u32> {
    let n_faces = mesh.faces.len();
    let mut visited = vec![false; n_faces];
    let mut result = Vec::new();
    let mut queue = VecDeque::new();

    let segment_id = if !mesh.segment_labels.is_empty() {
        Some(mesh.segment_labels[start_face as usize])
    } else {
        None
    };

    queue.push_back(start_face);
    visited[start_face as usize] = true;

    while let Some(current) = queue.pop_front() {
        result.push(current);

        // Find neighbors in adjacency graph
        let node_idx = petgraph::graph::NodeIndex::new(current as usize);
        for edge in mesh.face_adjacency.edges(node_idx) {
            let neighbor = if edge.source() == node_idx {
                edge.target()
            } else {
                edge.source()
            };
            let ni = neighbor.index() as u32;
            if !visited[ni as usize] {
                // Check segment constraint
                let same_segment = match segment_id {
                    Some(sid) => mesh.segment_labels.get(ni as usize) == Some(&sid),
                    None => true,
                };
                if same_segment {
                    visited[ni as usize] = true;
                    queue.push_back(ni);
                }
            }
        }
    }

    result
}
