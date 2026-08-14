//! Layer 3 of the planar-region fusion study (`docs/09`): the **machine-vision
//! (MultiView 3→2→3) evidence channel**.
//!
//! Strategy (投影 → 2D 连通 → 回投 → 带权 match graph → 可切割):
//!
//!   1. **投影** — render the mesh orthographically from `N` Fibonacci-sphere
//!      view directions. For each view we build a 2D image-plane basis `(u, w)`
//!      ⟂ the view axis `d` and project every vertex to `(pu, pw)`.
//!   2. **2D 连通** — two 3D-adjacent faces are "2D-adjacent" in a view iff
//!      their shared edge projects to a *non-degenerate* segment (so edge-on /
//!      occluded adjacencies are dropped — this is the occlusion-aware step) AND
//!      their average depths along `d` agree. Within a view we then grow regions
//!      by 2D-adjacency + 3D-normal agreement (the "feature / shape" cue),
//!      mirroring the planar detector but in the *projected* arrangement.
//!   3. **回投** — every face collects a per-view label. We back-project by
//!      building a face–face **match graph**: an edge `(a, b)` is weighted by how
//!      many views labelled `a` and `b` identically.
//!   4. **可切割** — union-find over matches with weight ≥ `match_threshold`
//!      yields the final clusters. This is a weighted graph cut over the
//!      multi-view consensus — the "ensemble vote" that makes MultiView more
//!      robust than any single projection.
//!
//! This module is **evidence only**: it never mutates the mesh and its output is
//! purely advisory ghost seeds (same path as Layer 1 / `recommend`), exactly as
//! `docs/09` Layer 3 prescribes. On an unfamiliar model it may disagree with
//! Layer 1 — both are offered to the user, never silently overriding.

use petgraph::unionfind::UnionFind;
use petgraph::visit::EdgeRef;
use serde::{Deserialize, Serialize};

use crate::mesh::model::MeshModel;
use crate::segment::recommend::SeedSuggestion;

/// Tunable knobs for [`detect_multiview_regions`].
#[derive(Debug, Clone, Copy)]
pub struct MultiViewParams {
    /// Number of orthographic views around the Fibonacci sphere. More views =
    /// better coverage of concave/occluded regions, at O(views·edges) cost.
    /// Default 12 (enough to cover all six faces of a cube; raise for thin or
    /// highly concave models).
    pub view_count: usize,
    /// Max angle (degrees) between a candidate face normal and the region's seed
    /// normal for it to join a 2D region. Default 20° (a touch looser than
    /// Layer 1's 15° because the per-view projection already supplies the
    /// spatial-contact constraint).
    pub angle_thr_deg: f32,
    /// Clusters with fewer faces than this are dropped (noise / too small to be
    /// a meaningful patch). Default 2.
    pub min_region_faces: usize,
    /// Minimum number of views in which two adjacent faces must share a label
    /// before they are merged. Default 1 = "agreed in at least one view". Raise
    /// to demand stronger consensus (more conservative, fewer merges).
    pub match_threshold: usize,
}

impl Default for MultiViewParams {
    fn default() -> Self {
        Self {
            view_count: 12,
            angle_thr_deg: 20.0,
            min_region_faces: 2,
            match_threshold: 1,
        }
    }
}

/// One MultiView-detected region (a cluster of faces that voted together across
/// views). Same shape as `PlanarRegion` so the frontend can reuse the exact same
/// ghost-seed → `seed_grow` union path and boundary-outline renderer.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MultiViewRegion {
    /// Number of triangle faces in the cluster.
    pub face_count: u32,
    /// A representative interior seed (the face whose centroid is closest to the
    /// cluster centroid) — ready to hand to `seed_grow` as a ghost suggestion.
    pub seed: SeedSuggestion,
    /// Outline of the cluster as 3D line segments `[[x,y,z],[x,y,z]]`.
    pub boundary_edges: Vec<[[f32; 3]; 2]>,
}

// ─── small vector helpers ────────────────────────────────────────────────
fn sub(a: &[f32; 3], b: &[f32; 3]) -> [f32; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
fn dot(a: &[f32; 3], b: &[f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
fn cross(a: &[f32; 3], b: &[f32; 3]) -> [f32; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}
fn normalize(a: &[f32; 3]) -> [f32; 3] {
    let l = (a[0] * a[0] + a[1] * a[1] + a[2] * a[2]).sqrt().max(1e-12);
    [a[0] / l, a[1] / l, a[2] / l]
}

/// Mean edge length over all triangle edges — drives the 2D-degeneracy
/// tolerance `eps = M * 0.05` (a projected shared edge shorter than this is
/// treated as edge-on / occluded and dropped from the 2D adjacency).
fn mean_edge_length(mesh: &MeshModel) -> f32 {
    let mut sum = 0.0f32;
    let mut count = 0u64;
    for f in &mesh.faces {
        let vs = [
            mesh.vertices[f[0] as usize],
            mesh.vertices[f[1] as usize],
            mesh.vertices[f[2] as usize],
        ];
        for (a, b) in [(vs[0], vs[1]), (vs[1], vs[2]), (vs[2], vs[0])] {
            sum += sub(&a, &b).iter().map(|v| v * v).sum::<f32>().sqrt();
            count += 1;
        }
    }
    if count == 0 {
        1.0
    } else {
        sum / count as f32
    }
}

/// Bounding sphere (center + radius) of the mesh vertices.
fn bounding(mesh: &MeshModel) -> ([f32; 3], f32) {
    if mesh.vertices.is_empty() {
        return ([0.0, 0.0, 0.0], 1.0);
    }
    let mut mn = [f32::MAX; 3];
    let mut mx = [f32::MIN; 3];
    for v in &mesh.vertices {
        for i in 0..3 {
            mn[i] = mn[i].min(v[i]);
            mx[i] = mx[i].max(v[i]);
        }
    }
    let c = [(mn[0] + mx[0]) / 2.0, (mn[1] + mx[1]) / 2.0, (mn[2] + mx[2]) / 2.0];
    let mut r = 0.0f32;
    for v in &mesh.vertices {
        let d = sub(v, &c).iter().map(|x| x * x).sum::<f32>().sqrt();
        r = r.max(d);
    }
    (c, r.max(1e-6))
}

/// `N` evenly-distributed directions on the unit sphere (Fibonacci spiral).
fn fibonacci_sphere(n: usize) -> Vec<[f32; 3]> {
    let mut dirs = Vec::with_capacity(n);
    let golden = std::f32::consts::PI * (3.0 - 5.0_f32.sqrt());
    for i in 0..n {
        let y = 1.0 - (i as f32 + 0.5) / n as f32 * 2.0;
        let rad = (1.0 - y * y).max(0.0).sqrt();
        let theta = golden * i as f32;
        dirs.push([rad * theta.cos(), y, rad * theta.sin()]);
    }
    dirs
}

/// Orthonormal image-plane basis `(u, w)` for a view direction `d`
/// (`u × w = ±d`). `u` is "right", `w` is "up" in the 2D projection.
fn image_basis(d: &[f32; 3]) -> ([f32; 3], [f32; 3]) {
    let up = if d[1].abs() < 0.99 {
        [0.0, 1.0, 0.0]
    } else {
        [1.0, 0.0, 0.0]
    };
    let u = normalize(&cross(&up, d));
    let w = normalize(&cross(d, &u));
    (u, w)
}

/// Average signed depth of a face along view axis `d` (relative to center `c`).
fn face_depth(mesh: &MeshModel, f: u32, d: &[f32; 3], c: &[f32; 3]) -> f32 {
    let tri = mesh.faces[f as usize];
    let mut s = 0.0f32;
    for &v in &tri {
        s += dot(&sub(&mesh.vertices[v as usize], c), d);
    }
    s / 3.0
}

/// The two vertices shared by adjacent faces `a` and `b` (their common edge),
/// or `None` if they do not share exactly two vertices.
fn shared_edge(mesh: &MeshModel, a: u32, b: u32) -> Option<(u32, u32)> {
    let fa = mesh.faces[a as usize];
    let fb = mesh.faces[b as usize];
    let mut out = [0u32; 2];
    let mut k = 0;
    for &va in &fa {
        if fb.contains(&va) && k < 2 {
            out[k] = va;
            k += 1;
        }
    }
    if k == 2 {
        Some((out[0], out[1]))
    } else {
        None
    }
}

/// Detect multi-view regions. See module docs for the algorithm.
///
/// Returns regions sorted by `face_count` descending. Faces that never join a
/// cluster (curved / occluded / too small) are simply absent — this is an
/// *advisory* detector feeding ghost seeds, not a full relabelling; `seed_grow`'s
/// fallback already covers whatever is left.
pub fn detect_multiview_regions(mesh: &MeshModel, params: &MultiViewParams) -> Vec<MultiViewRegion> {
    let n = mesh.faces.len();
    if n == 0 {
        return Vec::new();
    }
    if mesh.normals.len() != n {
        // No normals → cannot judge coplanarity. Advisory no-op (honesty rule:
        // unassessable → empty, not a guess).
        return Vec::new();
    }
    let views = params.view_count.max(1);
    let m = mean_edge_length(mesh);
    let (center, radius) = bounding(mesh);
    let cos_thr = (params.angle_thr_deg as f64).to_radians().cos() as f32;
    let eps = (m * 0.05).max(1e-6);
    let depth_tol = radius * 0.5;

    // Precompute (once) every 3D-adjacent edge together with its shared vertices.
    let mut adj: Vec<(u32, u32, u32, u32)> = Vec::new(); // (a, b, shared_va, shared_vb)
    for e in mesh.face_adjacency.edge_references() {
        let a = e.source().index() as u32;
        let b = e.target().index() as u32;
        if a >= n as u32 || b >= n as u32 {
            continue;
        }
        if let Some((va, vb)) = shared_edge(mesh, a, b) {
            adj.push((a, b, va, vb));
        }
    }

    // Per-face list of (view_index, per-view label).
    let mut per_face_views: Vec<Vec<(usize, usize)>> = vec![Vec::new(); n];

    let dirs = fibonacci_sphere(views);
    for (vi, d) in dirs.iter().enumerate() {
        let (u, w) = image_basis(d);
        // Project every vertex into the 2D image plane.
        let proj: Vec<[f32; 2]> = mesh
            .vertices
            .iter()
            .map(|v| {
                let r = sub(v, &center);
                [dot(&r, &u), dot(&r, &w)]
            })
            .collect();

        // 2D adjacency: shared edge projects non-degenerate AND depths agree.
        let mut view_adj: Vec<Vec<u32>> = vec![Vec::new(); n];
        for &(a, b, va, vb) in &adj {
            let pa = proj[va as usize];
            let pb = proj[vb as usize];
            let dl = ((pa[0] - pb[0]).powi(2) + (pa[1] - pb[1]).powi(2)).sqrt();
            if dl <= eps {
                continue; // edge-on / occluded → drop
            }
            let da = face_depth(mesh, a, d, &center);
            let db = face_depth(mesh, b, d, &center);
            if (da - db).abs() > depth_tol {
                continue; // front/back overlap → drop
            }
            view_adj[a as usize].push(b);
            view_adj[b as usize].push(a);
        }

        // 2D region growing by normal agreement (seed normal fixed per region).
        let mut label = vec![usize::MAX; n];
        let mut next_label = 0usize;
        let mut stack: Vec<u32> = Vec::new();
        for start in 0..n {
            if label[start] != usize::MAX {
                continue;
            }
            let seed_normal = mesh.normals[start];
            label[start] = next_label;
            stack.push(start as u32);
            while let Some(f) = stack.pop() {
                for &g in &view_adj[f as usize] {
                    if label[g as usize] != usize::MAX {
                        continue;
                    }
                    let ng = mesh.normals[g as usize];
                    let dotn = seed_normal[0] * ng[0]
                        + seed_normal[1] * ng[1]
                        + seed_normal[2] * ng[2];
                    if dotn.abs() < cos_thr {
                        continue; // angle too steep → not coplanar
                    }
                    label[g as usize] = next_label;
                    stack.push(g);
                }
            }
            next_label += 1;
        }

        for f in 0..n {
            if label[f] != usize::MAX {
                per_face_views[f].push((vi, label[f]));
            }
        }
    }

    // Build the weighted match graph (face–face, weight = #views in agreement)
    // and cut it with union-find at `match_threshold`.
    let mut uf = UnionFind::new(n);
    for &(a, b, _, _) in &adj {
        let la = &per_face_views[a as usize];
        let lb = &per_face_views[b as usize];
        if la.is_empty() || lb.is_empty() {
            continue;
        }
        let mut agree = 0usize;
        for &(va, lab_a) in la {
            for &(vb, lab_b) in lb {
                if va == vb && lab_a == lab_b {
                    agree += 1;
                    break;
                }
            }
        }
        if agree >= params.match_threshold {
            uf.union(a as usize, b as usize);
        }
    }

    // Group faces by union-find root; only faces that received ≥1 label join.
    let mut groups: std::collections::HashMap<usize, Vec<u32>> =
        std::collections::HashMap::new();
    for f in 0..n {
        if per_face_views[f].is_empty() {
            continue;
        }
        groups.entry(uf.find(f)).or_default().push(f as u32);
    }

    // Build public regions: seed point + boundary edges each.
    let centroids = mesh.face_centers();
    let mut out: Vec<MultiViewRegion> = groups
        .into_iter()
        .map(|(_, region)| {
            // Region centroid = mean of face centroids.
            let mut rc = [0.0f32; 3];
            for &f in &region {
                let c = centroids[f as usize];
                rc[0] += c[0];
                rc[1] += c[1];
                rc[2] += c[2];
            }
            let inv = 1.0 / region.len() as f32;
            rc[0] *= inv;
            rc[1] *= inv;
            rc[2] *= inv;

            // Representative seed = face whose centroid is nearest the region centroid.
            let mut best_f = region[0];
            let mut best_d = f32::INFINITY;
            for &f in &region {
                let c = centroids[f as usize];
                let d = (c[0] - rc[0]).powi(2) + (c[1] - rc[1]).powi(2) + (c[2] - rc[2]).powi(2);
                if d < best_d {
                    best_d = d;
                    best_f = f;
                }
            }
            let seed_point = centroids[best_f as usize];

            // Boundary edges: mesh edges used by exactly one in-region face.
            let mut edge_count: std::collections::HashMap<(u32, u32), u32> =
                std::collections::HashMap::new();
            for &f in &region {
                let tri = mesh.faces[f as usize];
                for (a, b) in [(tri[0], tri[1]), (tri[1], tri[2]), (tri[2], tri[0])] {
                    *edge_count.entry((a.min(b), a.max(b))).or_insert(0) += 1;
                }
            }
            let mut boundary_edges = Vec::new();
            for (&key, &cnt) in &edge_count {
                if cnt == 1 {
                    let pa = mesh.vertices[key.0 as usize];
                    let pb = mesh.vertices[key.1 as usize];
                    boundary_edges.push([pa, pb]);
                }
            }

            MultiViewRegion {
                face_count: region.len() as u32,
                seed: SeedSuggestion {
                    point: seed_point,
                    face_index: best_f,
                },
                boundary_edges,
            }
        })
        .filter(|r| r.face_count as usize >= params.min_region_faces)
        .collect();

    out.sort_by(|a, b| b.face_count.cmp(&a.face_count));
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::segment::metrics::unit_cube;

    fn params_for_cube() -> MultiViewParams {
        // 20 views comfortably cover all six cube faces; default angle/thresholds
        // keep each face's two coplanar triangles together.
        MultiViewParams {
            view_count: 20,
            angle_thr_deg: 20.0,
            min_region_faces: 2,
            match_threshold: 1,
        }
    }

    #[test]
    fn unit_cube_yields_six_multiview_regions() {
        let mesh = unit_cube();
        let regions = detect_multiview_regions(&mesh, &params_for_cube());
        assert_eq!(regions.len(), 6, "a cube has 6 flat faces → 6 regions");
    }

    #[test]
    fn every_cube_region_has_a_closed_boundary() {
        let mesh = unit_cube();
        let regions = detect_multiview_regions(&mesh, &params_for_cube());
        for (i, r) in regions.iter().enumerate() {
            assert_eq!(
                r.boundary_edges.len(),
                4,
                "cube face region {i} should have 4 boundary edges"
            );
        }
    }

    #[test]
    fn all_twelve_faces_are_covered() {
        let mesh = unit_cube();
        let regions = detect_multiview_regions(&mesh, &params_for_cube());
        let total: u32 = regions.iter().map(|r| r.face_count).sum();
        assert_eq!(total, 12, "all 12 cube triangles should belong to a region");
    }

    #[test]
    fn empty_mesh_is_safe() {
        let mesh = MeshModel::new();
        assert!(detect_multiview_regions(&mesh, &MultiViewParams::default()).is_empty());
    }

    #[test]
    fn missing_normals_yields_no_regions() {
        let mut mesh = unit_cube();
        mesh.normals.clear();
        assert!(detect_multiview_regions(&mesh, &params_for_cube()).is_empty());
    }

    #[test]
    fn seed_is_a_valid_face_reference() {
        let mesh = unit_cube();
        let regions = detect_multiview_regions(&mesh, &params_for_cube());
        for r in &regions {
            assert!(
                (r.seed.face_index as usize) < mesh.faces.len(),
                "seed face_index must reference a real face"
            );
            assert!(
                r.seed.point.iter().all(|v| v.is_finite()),
                "seed point must be finite"
            );
        }
    }
}
