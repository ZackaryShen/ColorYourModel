//! Iteration 52 (enhanced): recommended seed points for the seeded-watershed tool.
//!
//! The seeded-watershed tool (iteration 50) lets the user drop seeds to define
//! regions, but picking WHERE to drop them on a complex model is trial-and-error.
//! This module SUGGESTS candidate seed locations so each major part tends to get
//! one representative seed, placed in the region INTERIOR (away from creases)
//! rather than on a boundary.
//!
//! ## Spread metric: geodesic (graph) distance, not straight-line
//!
//! The first cut (iteration 52) used Euclidean centroid distance as the spread
//! proxy — cheap O(count·F) but wrong for thin / elongated parts, where two
//! points on opposite sides of a limb can be close in 3D yet far across the
//! surface. This version runs a **multi-source Dijkstra over the face-adjacency
//! dual graph** (edge weight = geodesic distance between adjacent face
//! centroids), so "far from existing seeds" means far *along the mesh*, which is
//! what matters for region coverage. See [`geodesic_min_dist`].
//!
//! ## Significance field: curvature ⊕ concavity, weights exposed to the UI
//!
//! A seed should sit in a region interior, i.e. away from both sharp creases
//! (high normal difference) and concave valleys. We combine two per-face terms
//! into a single significance score `sig ∈ [0,1]` (higher = more
//! boundary-like):
//!
//! ```text
//! sig = w_curv · curv_norm + w_conc · conc_norm
//! ```
//!
//! `w_curv` / `w_conc` were previously hard-coded constants; they are now
//! command parameters so the UI can bias the suggestion toward either sharp
//! creases or concave valleys. The interior-ness used by FPS is `1 − sig`.

use crate::mesh::model::MeshModel;
use crate::segment::concavity::vertex_concavity;
use petgraph::graph::NodeIndex;
use petgraph::visit::EdgeRef;
use serde::{Deserialize, Serialize};
use std::collections::BinaryHeap;

/// One suggested seed: a model-local point plus the face it sits on (so the
/// frontend can accept it by handing the exact point + face_index to the same
/// `SeedInput` the manual click path uses).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SeedSuggestion {
    pub point: [f32; 3],
    pub face_index: u32,
}

/// Weights for the significance field. Both default to 1.0; the UI lets the user
/// push the suggestion toward creases (`curvature`) or valleys (`concavity`).
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecommendWeights {
    pub curvature: f32,
    pub concavity: f32,
}

impl Default for RecommendWeights {
    fn default() -> Self {
        RecommendWeights {
            curvature: 1.0,
            concavity: 1.0,
        }
    }
}

/// Build the vertex→faces incidence list `vertex_concavity` needs.
fn vertex_faces(mesh: &MeshModel) -> Vec<Vec<u32>> {
    let nv = mesh.vertices.len();
    let mut vf = vec![Vec::new(); nv];
    for (fi, f) in mesh.faces.iter().enumerate() {
        for &v in f {
            vf[v as usize].push(fi as u32);
        }
    }
    vf
}

/// Per-face curvature term in [0,1]: mean of (1 − |n_i·n_j|) over adjacent faces.
/// 0 = shares a normal with every neighbour (flat interior); 1 = perpendicular
/// normals all around (sharp crease). A degenerate mesh with no adjacency, or
/// uncomputed normals, yields all-zero scores → FPS falls back to pure spread.
fn face_curvature_scores(mesh: &MeshModel) -> Vec<f32> {
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

/// Per-face concavity term in [0,1]: fraction of the face's three vertices that
/// the concavity analysis flags as concave (a valley, not a ridge). Reuses the
/// same signed-dihedral test `segment_by_concavity` uses, so the suggestion
/// agrees with the manual partitioner about where the real part boundaries are.
fn face_concavity_scores(mesh: &MeshModel) -> Vec<f32> {
    let n = mesh.faces.len();
    let mut scores = vec![0.0f32; n];
    if mesh.normals.len() != n || mesh.vertices.is_empty() {
        return scores;
    }
    let vf = vertex_faces(mesh);
    let concave = vertex_concavity(mesh, &mesh.normals, &vf);
    for (fi, f) in mesh.faces.iter().enumerate() {
        let c = f.iter().filter(|&&v| concave[v as usize]).count() as f32 / 3.0;
        scores[fi] = c;
    }
    scores
}

/// Combine curvature + concavity into a single significance score `sig ∈ [0,1]`
/// (higher = more boundary-like). Each term is normalised to its own max so the
/// two weights are comparable regardless of absolute magnitudes.
///
/// `pub(crate)` so the FH graph segmenter (iter 56) reuses the exact same field
/// instead of duplicating the curvature/concavity normalisation — if the two
/// ever drifted, the seed suggestion and the partition would disagree about
/// where the real part boundaries are.
pub(crate) fn face_significance(mesh: &MeshModel, weights: &RecommendWeights) -> Vec<f32> {
    let n = mesh.faces.len();
    let curv = face_curvature_scores(mesh);
    let conc = face_concavity_scores(mesh);
    let curv_max = curv.iter().cloned().fold(0.0f32, f32::max).max(1e-6);
    let conc_max = conc.iter().cloned().fold(0.0f32, f32::max).max(1e-6);
    (0..n)
        .map(|i| {
            let curv_n = curv[i] / curv_max;
            let conc_n = conc[i] / conc_max;
            let s = weights.curvature * curv_n + weights.concavity * conc_n;
            s.clamp(0.0, 1.0)
        })
        .collect()
}

/// Fixed-point scale for Dijkstra distances (f32 has no `Ord`, so we run the
/// priority queue on scaled `i64` keys).
const GEO_SCALE: i64 = 1_000_000;

/// Multi-source geodesic distance over the face-adjacency dual graph.
///
/// Edge weight between adjacent faces = Euclidean distance between their
/// centroids — a surface-length approximation of the true geodesic that is exact
/// enough for seed spread. `sources` are seeded at distance 0; `out` receives the
/// shortest distance from each face to its nearest source (or `f32::MAX` if
/// unreachable, which only happens on a disconnected mesh).
fn geodesic_min_dist(
    mesh: &MeshModel,
    sources: &[usize],
    centroids: &[[f32; 3]],
    out: &mut [f32],
) {
    let n = mesh.faces.len();
    let mut dsc: Vec<i64> = vec![i64::MAX; n];
    // Max-heap of (-distance, node) so the smallest distance pops first.
    let mut pq: BinaryHeap<(i64, usize)> = BinaryHeap::new();
    for &s in sources {
        if s < n {
            dsc[s] = 0;
            pq.push((0, s));
        }
    }
    while let Some((neg, u)) = pq.pop() {
        let du = -neg;
        if du > dsc[u] {
            continue; // stale entry
        }
        for e in mesh.face_adjacency.edges(NodeIndex::new(u)) {
            let v = e.target().index();
            if v >= n {
                continue;
            }
            let cu = centroids[u];
            let cv = centroids[v];
            let w = ((cu[0] - cv[0]).powi(2) + (cu[1] - cv[1]).powi(2) + (cu[2] - cv[2]).powi(2))
                .sqrt();
            let wsc = (w * GEO_SCALE as f32) as i64;
            let nv = du + wsc;
            if nv < dsc[v] {
                dsc[v] = nv;
                pq.push((-nv, v));
            }
        }
    }
    for i in 0..n {
        out[i] = if dsc[i] == i64::MAX {
            f32::MAX
        } else {
            dsc[i] as f32 / GEO_SCALE as f32
        };
    }
}

/// Return up to `count` suggested seed locations, spread geodesically across the
/// mesh and biased toward region interiors (low significance). Always succeeds
/// (returns fewer if the mesh has fewer faces than `count`); `count == 0` or an
/// empty mesh yields `[]`.
pub fn recommend_seeds(
    mesh: &MeshModel,
    count: usize,
    weights: RecommendWeights,
) -> Vec<SeedSuggestion> {
    let n = mesh.faces.len();
    if n == 0 || count == 0 {
        return Vec::new();
    }
    let centroids = mesh.face_centers();
    if centroids.len() != n {
        return Vec::new();
    }
    let sig = face_significance(mesh, &weights);
    let target = count.min(n);

    let mut chosen: Vec<usize> = Vec::with_capacity(target);
    let mut chosen_flag = vec![false; n];

    // First seed: least significant face (deepest region interior).
    let mut first = 0usize;
    let mut first_s = f32::MAX;
    for i in 0..n {
        if sig[i] < first_s {
            first_s = sig[i];
            first = i;
        }
    }
    chosen.push(first);
    chosen_flag[first] = true;

    // Remaining seeds: geodesic farthest-point sampling. `min_dist` holds the
    // geodesic distance to the nearest already-chosen seed; each new seed
    // maximises (geodesic distance × interior-ness).
    let mut min_dist = vec![f32::MAX; n];
    for _ in 1..target {
        geodesic_min_dist(mesh, &chosen, &centroids, &mut min_dist);
        let mut best_i = None;
        let mut best_val = f32::NEG_INFINITY;
        for i in 0..n {
            if chosen_flag[i] {
                continue;
            }
            let interior = 1.0 - sig[i];
            let val = min_dist[i] * interior;
            if val > best_val {
                best_val = val;
                best_i = Some(i);
            }
        }
        match best_i {
            Some(b) => {
                chosen.push(b);
                chosen_flag[b] = true;
            }
            None => break, // shouldn't happen for target ≤ n, defensive
        }
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

    fn w() -> RecommendWeights {
        RecommendWeights::default()
    }

    #[test]
    fn recommends_requested_count() {
        let m = cube();
        assert_eq!(recommend_seeds(&m, 4, w()).len(), 4);
    }

    #[test]
    fn recommends_clamped_to_face_count() {
        let m = cube();
        assert_eq!(recommend_seeds(&m, 999, w()).len(), m.faces.len());
    }

    #[test]
    fn no_duplicate_faces() {
        let m = cube();
        let s = recommend_seeds(&m, 6, w());
        let mut seen = std::collections::HashSet::new();
        for x in &s {
            assert!(seen.insert(x.face_index), "duplicate face {}", x.face_index);
        }
    }

    #[test]
    fn empty_mesh_safe() {
        let m = MeshModel::new();
        assert!(recommend_seeds(&m, 5, w()).is_empty());
    }

    #[test]
    fn zero_count_safe() {
        let m = cube();
        assert!(recommend_seeds(&m, 0, w()).is_empty());
    }

    /// Geodesic spread on a long thin bar: two seeds must land far apart *along
    /// the surface* even if their straight-line distance is small. With a pure
    /// Euclidean metric both could sit on the same end; with geodesic FPS the
    /// second seed is pulled to the opposite end.
    #[test]
    fn geodesic_spread_covers_a_thin_bar() {
        // 20 segments along X, each a quad (2 tris) → 40 faces.
        let seg = 20usize;
        let mut v = Vec::new();
        for i in 0..=seg {
            let x = i as f32;
            v.push([x, 0.0, 0.0]);
            v.push([x, 1.0, 0.0]);
        }
        let mut f = Vec::new();
        for i in 0..seg {
            let a = (i * 2) as u32;
            let b = (i * 2 + 1) as u32;
            let c = (i * 2 + 2) as u32;
            let d = (i * 2 + 3) as u32;
            f.push([a, b, c]);
            f.push([b, d, c]);
        }
        let mut m = MeshModel::new();
        m.vertices = v;
        m.faces = f;
        m.compute_normals();
        m.build_adjacency();

        let s = recommend_seeds(&m, 2, w());
        assert_eq!(s.len(), 2);
        // The two chosen faces must be on opposite ends: face 0 is at x=0,
        // face (seg-1)*2 is at x=seg. They should not both be at the same end.
        let min_f = s.iter().map(|x| x.face_index).min().unwrap();
        let max_f = s.iter().map(|x| x.face_index).max().unwrap();
        assert!(max_f - min_f >= (seg as u32) / 2, "seeds clustered at one end");
    }
}
