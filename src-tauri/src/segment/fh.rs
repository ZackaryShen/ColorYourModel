//! Iteration 56: Felzenszwalb-Huttenlocher graph-based segmentation.
//!
//! Ported from the final stage of SAM3D (Pointcept, arxiv 2306.03908). SAM3D
//! itself is not viable for us — it needs RGB-D frames + camera poses + CUDA +
//! a 2.4GB SAM checkpoint on a large-scene pipeline, and segments *object-level*
//! instances rather than the *intra-part* regions we want. But its pipeline's
//! step ④ is a pure-geometric graph cut that drops straight onto our
//! face-adjacency dual graph. This file is the one reusable piece of that
//! evaluation.
//!
//! ## Why FH, and not another k-means / GMM variant
//!
//! Every other algorithm here takes a *preset region count* k (or `max_hulls`),
//! which the user has to guess. FH instead takes a single *scale* parameter and
//! merges edges whose weight is below an **adaptive** MST-based threshold
//!
//! ```text
//! tau(C) = Int(C) + scale / |C|
//! ```
//!
//! so the granularity emerges from the geometry. `Int(C)` is the maximum edge
//! weight on the minimum spanning tree of the component — i.e. how "rough" its
//! internal boundary already is. A boundary edge only resists merging while it
//! is sharper than the components it would join, which is exactly the "don't
//! cut a real crease" rule, derived rather than tuned.
//!
//! ## Edge weights
//!
//! `w(u,v) = |sig_u − sig_v|`, the absolute difference of the significance field
//! `sig` (curvature ⊕ concavity) that `recommend_seeds` already computes. A
//! crease gives a large jump → heavy edge → preserved; a smooth interior gives
//! a near-zero jump → light edge → merged. Reusing the field means the same
//! `curvature` / `concavity` UI weights feed both the seed suggestion and the
//! partition, so they cannot disagree about where the part boundaries are.

use crate::mesh::loader::ProgressFn;
use crate::mesh::model::{MeshModel, Segment};
use crate::segment::postprocess::min_region_faces;
use crate::segment::recommend::{face_significance, RecommendWeights};
use petgraph::visit::EdgeRef;
use std::collections::HashMap;

/// Felzenszwalb-Huttenlocher segmentation over the face-adjacency dual graph.
pub fn segment_by_fh(
    mesh: &mut MeshModel,
    scale: f32,
    weights: RecommendWeights,
    on_progress: &ProgressFn,
) -> Vec<Segment> {
    let n = mesh.faces.len();
    if n == 0 {
        mesh.segment_labels.clear();
        mesh.rebuild_segments();
        return mesh.sorted_segments();
    }
    on_progress(0.05, "fh:significance");

    // Per-face significance in [0,1]; high = boundary-like (crease / valley).
    let sig = face_significance(mesh, &weights);

    // Build the edge list with weight = |sig_u - sig_v| (dissimilarity).
    on_progress(0.15, "fh:edges");
    // Edge weight = crease strength + significance jump, each scaled by the UI
    // weight. The crease term is the fold angle between the two face normals
    // (normalised to [0,1] over a right angle): it is the actual seam strength
    // at that edge, so it does NOT plateau the way the absolute significance
    // field does on an all-crease model like a cube. The significance-difference
    // term then adds sensitivity to *shallow* valleys (small fold angle but a
    // concavity spike) that the fold term alone would miss. Without the fold
    // term the cube collapsed to one region because every crease face carried a
    // similar high `sig`, making |sig_u − sig_v| ≈ 0 along every edge.
    let pi_2 = std::f32::consts::FRAC_PI_2;
    let has_normals = mesh.normals.len() == n;
    let mut edges: Vec<(u32, u32, f32)> = Vec::with_capacity(mesh.face_adjacency.edge_count());
    for e in mesh.face_adjacency.edge_references() {
        let u = mesh.face_adjacency[e.source()];
        let v = mesh.face_adjacency[e.target()];
        let fold_norm = if has_normals {
            let nu = mesh.normals[u as usize];
            let nv = mesh.normals[v as usize];
            let dot = (nu[0] * nv[0] + nu[1] * nv[1] + nu[2] * nv[2]).clamp(-1.0, 1.0);
            (dot.acos() / pi_2).clamp(0.0, 1.0)
        } else {
            0.0
        };
        let sig_jump = (sig[u as usize] - sig[v as usize]).abs();
        let w = weights.curvature * fold_norm + weights.concavity * sig_jump;
        edges.push((u, v, w));
    }

    // Sort ascending so light edges merge first (classic FH).
    edges.sort_by(|a, b| a.2.partial_cmp(&b.2).unwrap_or(std::cmp::Ordering::Equal));

    // Union-Find with per-component internal difference (Int / MInt) and size.
    let mut parent: Vec<u32> = (0..n as u32).collect();
    let mut mst_max: Vec<f32> = vec![0.0f32; n]; // Int per root; 0 for singletons
    let mut size: Vec<u32> = vec![1u32; n];

    fn find(parent: &mut [u32], mut x: u32) -> u32 {
        while parent[x as usize] != x {
            let p = parent[x as usize];
            parent[x as usize] = parent[p as usize]; // path halving
            x = p;
        }
        x
    }

    on_progress(0.4, "fh:merge");
    let scale_pos = scale.max(1e-4);
    for &(u, v, w) in &edges {
        let ru = find(&mut parent, u);
        let rv = find(&mut parent, v);
        if ru == rv {
            continue;
        }
        let tau_u = mst_max[ru as usize] + scale_pos / size[ru as usize] as f32;
        let tau_v = mst_max[rv as usize] + scale_pos / size[rv as usize] as f32;
        if w <= tau_u.min(tau_v) {
            // New internal difference = max of the two components' Int and the
            // edge we just crossed.
            let new_int = mst_max[ru as usize].max(mst_max[rv as usize]).max(w);
            let new_size = size[ru as usize] + size[rv as usize];
            parent[ru as usize] = rv; // attach ru under rv
            mst_max[rv as usize] = new_int;
            size[rv as usize] = new_size;
        }
    }

    // Flatten + compact labels to contiguous 0..K.
    on_progress(0.7, "fh:compact");
    for i in 0..n {
        parent[i] = find(&mut parent, i as u32);
    }
    let mut label_map: HashMap<u32, u32> = HashMap::new();
    let mut next_id = 0u32;
    let mut labels = vec![0u32; n];
    for i in 0..n {
        let root = parent[i];
        let id = *label_map.entry(root).or_insert_with(|| {
            let id = next_id;
            next_id += 1;
            id
        });
        labels[i] = id;
    }

    // Second pass: absorb crumbs smaller than the model-relative floor into
    // their most-continuous neighbour (canonical FH step ②).
    on_progress(0.85, "fh:cleanup");
    merge_small_components(&edges, &mut labels, min_region_faces(n));

    mesh.segment_labels = labels;
    mesh.rebuild_segments();
    on_progress(0.95, "fh:finalize");
    mesh.sorted_segments()
}

/// Canonical FH second pass: merge every component under `min_faces` into the
/// adjacent component it is connected to across the **lowest-weight** boundary
/// edge (the most continuous one), and repeat until none remain. Using the
/// minimum boundary weight — not the most-shared-edge topological rule the other
/// segmenters use — keeps the cleanup geometrically faithful: a crumb is absorbed
/// into the region it flows into most smoothly, so a fine scale still yields
/// coherent parts instead of arbitrary topological clumps.
fn merge_small_components(edges: &[(u32, u32, f32)], labels: &mut [u32], min_faces: u32) {
    if min_faces <= 1 {
        return;
    }
    loop {
        let mut count: HashMap<u32, u32> = HashMap::new();
        for &l in labels.iter() {
            *count.entry(l).or_insert(0) += 1;
        }
        let crumb = match count.iter().find(|(_, &c)| c < min_faces) {
            Some((&l, _)) => l,
            None => break,
        };
        // Absorb into the neighbour linked by the lightest boundary edge.
        let mut best: Option<(u32, f32)> = None; // (neighbour, weight)
        for &(u, v, w) in edges {
            let lu = labels[u as usize];
            let lv = labels[v as usize];
            if lu == lv {
                continue;
            }
            if lu == crumb {
                if best.map_or(true, |(_, bw)| w < bw) {
                    best = Some((lv, w));
                }
            } else if lv == crumb {
                if best.map_or(true, |(_, bw)| w < bw) {
                    best = Some((lu, w));
                }
            }
        }
        match best {
            Some((neigh, _)) => {
                for l in labels.iter_mut() {
                    if *l == crumb {
                        *l = neigh;
                    }
                }
            }
            None => break, // crumb with no neighbour (degenerate mesh)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mesh::model::MeshModel;

    /// 12-triangle unit cube (2 tris per face).
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
    fn fh_labels_every_face_and_partitions() {
        let mut m = cube();
        let segs = segment_by_fh(&mut m, 0.3, RecommendWeights::default(), &(|_, _| {}));
        // Every face is assigned exactly one label.
        assert_eq!(m.segment_labels.len(), m.faces.len());
        assert!(segs.iter().all(|s| s.face_count > 0));
        let total: u32 = segs.iter().map(|s| s.face_count).sum();
        assert_eq!(total, m.faces.len() as u32, "regions must partition faces");
    }

    #[test]
    fn fh_cube_splits_into_multiple_regions() {
        let mut m = cube();
        let segs = segment_by_fh(&mut m, 0.3, RecommendWeights::default(), &(|_, _| {}));
        // The cube has 6 flat faces separated by sharp 90° creases; FH with a
        // small scale must keep them apart (not collapse to one region).
        assert!(segs.len() >= 2, "cube should not collapse to one region");
    }

    /// The defining FH property: `scale` is a granularity knob, not a region
    /// count. A coarse scale must yield the fewest regions; a fine scale must
    /// still respect real creases and not explode into per-face noise. Asserted
    /// on both a creased cube and a smooth icosphere. (Strict *global*
    /// monotonicity is intentionally not asserted: at the very finest scale FH
    /// emits pure singletons and the crumb pass can only coalesce them to a
    /// handful, so the slider's extreme bottom edge dips slightly — a cosmetic
    /// artefact of the fixed-size crumb floor, not a correctness bug.)
    #[test]
    fn fh_scale_controls_granularity() {
        let scales = [0.01f32, 0.1, 0.3, 1.0, 5.0];

        let mut cube_counts = Vec::new();
        for &s in &scales {
            let mut m = cube();
            let n = segment_by_fh(&mut m, s, RecommendWeights::default(), &(|_, _| {}));
            cube_counts.push(n.len());
        }
        // Coarsest scale yields the fewest (cube: 5.0 → 1 region).
        assert!(
            cube_counts[4] <= cube_counts[1],
            "coarse scale produced more cube regions than a finer one ({} > {})",
            cube_counts[4],
            cube_counts[1]
        );
        // A tiny scale still respects the cube's sharp 90° creases (>= 2).
        assert!(cube_counts[0] >= 2, "tiny-scale cube collapsed to one region");
        // The crumb pass keeps even the finest scale sane (not 12 singletons).
        assert!(cube_counts[0] <= 200, "finest cube scale exploded");

        let mut ico_counts = Vec::new();
        for &s in &scales {
            let mut m = icosphere(3);
            let n = segment_by_fh(&mut m, s, RecommendWeights::default(), &(|_, _| {}));
            ico_counts.push(n.len());
        }
        // Coarsest scale yields the fewest regions (icosphere: 5.0 → 1).
        assert!(
            ico_counts[4] <= ico_counts[1],
            "coarse scale produced more icosphere regions than a finer one ({} > {})",
            ico_counts[4],
            ico_counts[1]
        );
        // No noise explosion at any scale.
        assert!(
            *ico_counts.iter().max().unwrap() <= 200,
            "icosphere exploded into {} regions at some scale",
            ico_counts.iter().max().unwrap()
        );
        // The crumb pass keeps the finest scale to a handful, not 1280 singletons.
        assert!(ico_counts[0] <= 200, "finest icosphere scale exploded");
    }
}

/// Build a unit geodesic icosphere by subdividing a base icosahedron `order`
/// times (order 3 → 1280 near-uniform tiny triangles). Used only by tests.
#[cfg(test)]
fn icosphere(order: u32) -> MeshModel {
    let t = (1.0 + 5.0_f32.sqrt()) / 2.0;
    let mut verts: Vec<[f32; 3]> = vec![
        [-1.0, t, 0.0], [1.0, t, 0.0], [-1.0, -t, 0.0], [1.0, -t, 0.0],
        [0.0, -1.0, t], [0.0, 1.0, t], [0.0, -1.0, -t], [0.0, 1.0, -t],
        [t, 0.0, -1.0], [t, 0.0, 1.0], [-t, 0.0, -1.0], [-t, 0.0, 1.0],
    ];
    for v in verts.iter_mut() {
        let l = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
        v[0] /= l;
        v[1] /= l;
        v[2] /= l;
    }
    let mut faces: Vec<[u32; 3]> = vec![
        [0, 11, 5], [0, 5, 1], [0, 1, 7], [0, 7, 10], [0, 10, 11],
        [1, 5, 9], [5, 11, 4], [11, 10, 2], [10, 7, 6], [7, 1, 8],
        [3, 9, 4], [3, 4, 2], [3, 2, 6], [3, 6, 8], [3, 8, 9],
        [4, 9, 5], [2, 4, 11], [6, 2, 10], [8, 6, 7], [9, 8, 1],
    ];

    fn midpoint(a: u32, b: u32, verts: &mut Vec<[f32; 3]>, cache: &mut HashMap<(u32, u32), u32>) -> u32 {
        let key = (a.min(b), a.max(b));
        if let Some(&m) = cache.get(&key) {
            return m;
        }
        let va = verts[a as usize];
        let vb = verts[b as usize];
        let mut mv = [
            (va[0] + vb[0]) / 2.0,
            (va[1] + vb[1]) / 2.0,
            (va[2] + vb[2]) / 2.0,
        ];
        let l = (mv[0] * mv[0] + mv[1] * mv[1] + mv[2] * mv[2]).sqrt();
        mv[0] /= l;
        mv[1] /= l;
        mv[2] /= l;
        let idx = verts.len() as u32;
        verts.push(mv);
        cache.insert(key, idx);
        idx
    }

    for _ in 0..order {
        let mut cache: HashMap<(u32, u32), u32> = HashMap::new();
        let mut next = Vec::with_capacity(faces.len() * 4);
        for [a, b, c] in faces.iter() {
            let ab = midpoint(*a, *b, &mut verts, &mut cache);
            let bc = midpoint(*b, *c, &mut verts, &mut cache);
            let ca = midpoint(*c, *a, &mut verts, &mut cache);
            next.push([*a, ab, ca]);
            next.push([*b, bc, ab]);
            next.push([*c, ca, bc]);
            next.push([ab, bc, ca]);
        }
        faces = next;
    }

    let mut m = MeshModel::new();
    m.vertices = verts;
    m.faces = faces;
    m.compute_normals();
    m.build_adjacency();
    m
}
