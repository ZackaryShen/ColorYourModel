//! CurvatureKMeans — intrinsic feature-clustering segmentation (REFUTE-revised).
//!
//! Per-face feature vector is INTRINSIC only: local curvature (mean |dihedral|
//! to neighbours) + thickness (|SDF|, optional). We deliberately exclude any
//! normal-direction component: PCA principal-axis projection has a
//! non-deterministic sign under eigen-degeneracy (sphere/cube), so `normal ·
//! principalAxis` would cluster faces by *orientation*, not by *part*.
//!
//! Pipeline (corrected per adversarial review):
//!   1. Build intrinsic features per face (absolute scales — see
//!      [`crate::segment::postprocess::log_normalize`] for why min-max is a trap).
//!   2. Deterministic farthest-point-seeded k-means (no `rand` dependency).
//!   3. Connectivity split → small-region merge → contour merge, all shared with
//!      the other algorithms in [`crate::segment::postprocess`].
//!
//! Final label count is bounded well below `MANUAL_SEGMENT_OFFSET` (100_000),
//! so the auto/manual label namespaces never collide.

use petgraph::visit::EdgeRef;

use crate::mesh::loader::ProgressFn;
use crate::mesh::model::{MeshModel, Segment};
use crate::segment::postprocess::{
    assemble_features, face_curvature, finalize_segments, log_normalize, refine_regions, Feature,
    DEFAULT_CREASE_DEG,
};
use crate::segment::sdf::{compute_sdf, oriented_normals};

/// Hard ceiling on cluster count to keep the label space safely under the manual
/// offset and the k-means tractable.
const MAX_K: usize = 24;

/// Light Laplacian smoothing of the normal field (noise robustness). New normal
/// for a face = normalized sum of its own + neighbours' normals.
fn smooth_normals(mesh: &MeshModel, normals: &[[f32; 3]]) -> Vec<[f32; 3]> {
    let n = mesh.faces.len();
    let mut out = vec![[0.0f32; 3]; n];
    for fi in 0..n {
        let node = petgraph::graph::NodeIndex::new(fi);
        let mut s = normals[fi];
        for edge in mesh.face_adjacency.edges(node) {
            let nb = if edge.source() == node {
                edge.target()
            } else {
                edge.source()
            };
            let ni = nb.index();
            s[0] += normals[ni][0];
            s[1] += normals[ni][1];
            s[2] += normals[ni][2];
        }
        let len = (s[0] * s[0] + s[1] * s[1] + s[2] * s[2]).sqrt();
        out[fi] = if len > 1e-10 {
            [s[0] / len, s[1] / len, s[2] / len]
        } else {
            normals[fi]
        };
    }
    out
}

/// Build per-face intrinsic features: curvature = mean |dihedral| to neighbours
/// on a 90° full-scale; thickness = |SDF| (log-normalized) when `use_sdf`.
fn build_features(mesh: &MeshModel, normals: &[[f32; 3]], use_sdf: bool) -> Vec<Feature> {
    let curv = face_curvature(mesh, normals);
    if use_sdf {
        let thick = log_normalize(&compute_sdf(mesh));
        assemble_features(&curv, Some(&thick))
    } else {
        assemble_features(&curv, None)
    }
}

/// Deterministic farthest-point-seeded k-means on 2-D features. No RNG.
fn deterministic_kmeans(feats: &[Feature], k: usize) -> Vec<u32> {
    let n = feats.len();
    if n == 0 {
        return Vec::new();
    }
    let k = k.clamp(2, MAX_K).min(n);

    // Seed: first = max-norm point; rest = farthest from the current seed set.
    let mut seeds: Vec<usize> = Vec::with_capacity(k);
    let first = (0..n)
        .max_by(|&a, &b| {
            let na = feats[a].curv + feats[a].thick;
            let nb = feats[b].curv + feats[b].thick;
            na.partial_cmp(&nb).unwrap()
        })
        .unwrap();
    seeds.push(first);
    while seeds.len() < k {
        let mut best = 0usize;
        let mut best_d = -1.0f32;
        for i in 0..n {
            if seeds.contains(&i) {
                continue;
            }
            let mut md = f32::INFINITY;
            for &s in &seeds {
                let dx = feats[i].curv - feats[s].curv;
                let dy = feats[i].thick - feats[s].thick;
                md = md.min(dx * dx + dy * dy);
            }
            if md > best_d {
                best_d = md;
                best = i;
            }
        }
        seeds.push(best);
    }

    let mut centroids: Vec<(f32, f32)> = seeds
        .iter()
        .map(|&s| (feats[s].curv, feats[s].thick))
        .collect();
    let mut labels = vec![0u32; n];
    for _ in 0..20 {
        let mut changed = false;
        for (i, f) in feats.iter().enumerate() {
            let mut best = 0usize;
            let mut best_d = f32::INFINITY;
            for (c, cen) in centroids.iter().enumerate() {
                let dx = f.curv - cen.0;
                let dy = f.thick - cen.1;
                let d = dx * dx + dy * dy;
                if d < best_d {
                    best_d = d;
                    best = c;
                }
            }
            if labels[i] != best as u32 {
                labels[i] = best as u32;
                changed = true;
            }
        }
        let mut sums = vec![(0.0f32, 0.0f32, 0u32); k];
        for (i, f) in feats.iter().enumerate() {
            let c = labels[i] as usize;
            sums[c].0 += f.curv;
            sums[c].1 += f.thick;
            sums[c].2 += 1;
        }
        for c in 0..k {
            if sums[c].2 > 0 {
                centroids[c] = (sums[c].0 / sums[c].2 as f32, sums[c].1 / sums[c].2 as f32);
            }
        }
        if !changed {
            break;
        }
    }
    labels
}

/// Segment by intrinsic-feature clustering. See module docs for the corrected
/// pipeline. Returns segment metadata; per-face labels are written to
/// `mesh.segment_labels` and `mesh.segments` is rebuilt.
pub fn segment_by_curvature_kmeans(
    mesh: &mut MeshModel,
    k_user: u32,
    smoothing_iters: u32,
    use_sdf: bool,
    crease_threshold_deg: f32,
    on_progress: &ProgressFn,
) -> Vec<Segment> {
    let n = mesh.faces.len();
    if n == 0 {
        // kdtree queries and k-means seeding both assume ≥1 face; an empty mesh
        // reaches here only via a malformed import, and panicking inside a Tauri
        // command would take the whole app down.
        mesh.segment_labels.clear();
        mesh.segments.clear();
        on_progress(1.0, "curv: empty mesh");
        return Vec::new();
    }
    on_progress(0.0, "curv: building features");
    let mut normals = oriented_normals(mesh);
    for _ in 0..smoothing_iters {
        normals = smooth_normals(mesh, &normals);
    }
    let feats = build_features(mesh, &normals, use_sdf);
    on_progress(0.3, "curv: k-means");

    let k = if k_user == 0 { 6 } else { k_user as usize };
    let km_labels = deterministic_kmeans(&feats, k);
    on_progress(0.55, "curv: connectivity split + contour merge");
    let crease = if crease_threshold_deg > 0.0 {
        crease_threshold_deg
    } else {
        DEFAULT_CREASE_DEG
    };
    let final_labels = refine_regions(mesh, &km_labels, &feats, &normals, Some(crease));

    let segments = finalize_segments(mesh, final_labels);
    on_progress(1.0, &format!("curv: {} regions", segments.len()));
    segments
}
