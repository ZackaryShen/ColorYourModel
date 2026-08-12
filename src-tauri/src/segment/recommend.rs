//! Iteration 52: recommended seed points (mesh / point-cloud sampling).
//!
//! The seeded-watershed tool (iteration 50) lets the user drop seeds to define
//! regions, but picking WHERE to drop them on a complex model is trial-and-error.
//! This module SUGGESTS candidate seed locations so each major part tends to get
//! one representative seed, placed in the region INTERIOR (away from creases)
//! rather than on a boundary.
//!
//! Method: weighted farthest-point sampling (FPS) over face centroids.
//!   - The first seed is the face with the LOWEST boundary score (most "interior").
//!   - Each subsequent seed maximizes (min Euclidean distance to existing seeds)
//!     × (1 − boundary_score): it is pushed far from already-chosen seeds AND
//!     away from creases.
//! Euclidean centroid distance is the spread proxy — cheap O(count·F) and good
//! enough for a *suggestion* the user still edits. Geodesic accuracy is a future
//! refinement (would need Dijkstra over face_adjacency) but unnecessary here.

use crate::mesh::model::MeshModel;
use petgraph::visit::EdgeRef;
use serde::{Deserialize, Serialize};

/// One suggested seed: a model-local point plus the face it sits on (so the
/// frontend can accept it by handing the exact point + face_index to the same
/// `SeedInput` the manual click path uses).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SeedSuggestion {
    pub point: [f32; 3],
    pub face_index: u32,
}

/// Per-face boundary score in [0,1]: mean of (1 − |n_i·n_j|) over adjacent faces.
/// 0 = shares a normal with every neighbour (flat interior); 1 = perpendicular
/// normals all around (sharp crease / part boundary). A degenerate mesh with no
/// adjacency, or uncomputed normals, yields all-zero scores → FPS falls back to
/// pure spatial spread (still a valid suggestion, just less crease-aware).
fn face_boundary_scores(mesh: &MeshModel) -> Vec<f32> {
    let n = mesh.faces.len();
    let mut score = vec![0.0f32; n];
    if mesh.normals.len() != n {
        return score; // normals not ready → leave zeros (spread-only fallback)
    }
    for e in mesh.face_adjacency.edge_references() {
        let a = e.source().index();
        let b = e.target().index();
        if a >= n || b >= n {
            continue;
        }
        let na = mesh.normals[a];
        let nb = mesh.normals[b];
        let dot = na[0] * nb[0] + na[1] * nb[1] + na[2] * nb[2];
        let d = 1.0 - dot.abs().min(1.0); // 0 shared normal, 1 perpendicular
        score[a] += d;
        score[b] += d;
    }
    // Average over incident edges so a high-degree vertex doesn't dominate.
    for node in mesh.face_adjacency.node_indices() {
        let deg = mesh.face_adjacency.edges(node).count();
        let i = node.index();
        if deg > 0 {
            score[i] /= deg as f32;
        }
    }
    score
}

/// Return up to `count` suggested seed locations, spread across the mesh and
/// biased toward region interiors. Always succeeds (returns fewer if the mesh
/// has fewer faces than `count`); `count == 0` or an empty mesh yields `[]`.
pub fn recommend_seeds(mesh: &MeshModel, count: usize) -> Vec<SeedSuggestion> {
    let n = mesh.faces.len();
    if n == 0 || count == 0 {
        return Vec::new();
    }
    let centroids = mesh.face_centers();
    if centroids.len() != n {
        return Vec::new();
    }
    let boundary = face_boundary_scores(mesh);
    let target = count.min(n);

    let mut chosen: Vec<usize> = Vec::with_capacity(target);

    // First seed: least boundary-like face (deepest region interior).
    let mut first = 0usize;
    let mut first_b = f32::MAX;
    for i in 0..n {
        if boundary[i] < first_b {
            first_b = boundary[i];
            first = i;
        }
    }
    chosen.push(first);

    // Remaining seeds: weighted farthest-point sampling.
    for _ in 1..target {
        let mut best_i = usize::MAX;
        let mut best_val = f32::NEG_INFINITY;
        for i in 0..n {
            // min squared Euclidean distance to any already-chosen seed
            let mut md = f32::MAX;
            for &c in &chosen {
                let dx = centroids[i][0] - centroids[c][0];
                let dy = centroids[i][1] - centroids[c][1];
                let dz = centroids[i][2] - centroids[c][2];
                let d = dx * dx + dy * dy + dz * dz;
                if d < md {
                    md = d;
                }
            }
            // Spread × interior-ness. (1 − boundary) is 0 on a crease, so creases
            // are never chosen as the maximizing arg even if far from others.
            let val = md * (1.0 - boundary[i]);
            if val > best_val {
                best_val = val;
                best_i = i;
            }
        }
        if best_i == usize::MAX {
            break; // shouldn't happen for target ≤ n, defensive
        }
        chosen.push(best_i);
    }

    chosen
        .into_iter()
        .map(|i| SeedSuggestion {
            point: centroids[i],
            face_index: i as u32,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mesh::model::MeshModel;

    /// A 12-triangle unit cube (2 tris per face).
    fn cube() -> MeshModel {
        let v = vec![
            [0., 0., 0.],
            [1., 0., 0.],
            [1., 1., 0.],
            [0., 1., 0.],
            [0., 0., 1.],
            [1., 0., 1.],
            [1., 1., 1.],
            [0., 1., 1.],
        ];
        let f: Vec<[u32; 3]> = vec![
            [0, 1, 2], [0, 2, 3], // bottom
            [4, 6, 5], [4, 7, 6], // top
            [0, 4, 5], [0, 5, 1], // front
            [1, 5, 6], [1, 6, 2], // right
            [2, 6, 7], [2, 7, 3], // back
            [3, 7, 4], [3, 4, 0], // left
        ];
        let mut m = MeshModel::new();
        m.vertices = v;
        m.faces = f;
        m.compute_normals();
        m.build_adjacency();
        m
    }

    #[test]
    fn recommends_requested_count() {
        let m = cube();
        assert_eq!(recommend_seeds(&m, 4).len(), 4);
    }

    #[test]
    fn recommends_clamped_to_face_count() {
        let m = cube();
        assert_eq!(recommend_seeds(&m, 999).len(), m.faces.len());
    }

    #[test]
    fn no_duplicate_faces() {
        let m = cube();
        let s = recommend_seeds(&m, 6);
        let mut seen = std::collections::HashSet::new();
        for x in &s {
            assert!(seen.insert(x.face_index), "duplicate face {}", x.face_index);
        }
    }

    #[test]
    fn empty_mesh_safe() {
        let m = MeshModel::new();
        assert!(recommend_seeds(&m, 5).is_empty());
    }

    #[test]
    fn zero_count_safe() {
        let m = cube();
        assert!(recommend_seeds(&m, 0).is_empty());
    }
}
