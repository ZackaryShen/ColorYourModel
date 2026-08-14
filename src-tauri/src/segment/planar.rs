//! Layer 1 of the planar-region fusion study (`docs/09`): the **geometric
//! backbone** that turns a triangle mesh into a set of *continuous planar
//! regions* plus their *planar boundaries*.
//!
//! This is the "continuous plane + plane boundary" deliverable the user asked
//! for. It upgrades the existing dihedral-only thinking (which splits on creases
//! but never *fits* a plane and never *outlines* a flat patch) into a real
//! planar-region detector:
//!
//!   1. **seed by flatness** — faces whose normal agrees with their neighbours
//!      (low curvature → high planarity) are seeded first. This reuses the same
//!      "flat interior" intuition `recommend.rs` already uses, but here it only
//!      *orders* the seeds; the region is defined by plane fit, not by curvature
//!      alone.
//!   2. **grow by plane fit + refit** — from a seed face we fit a PCA plane,
//!      then add neighbouring faces whose (a) normal is within `angle_thr` of the
//!      plane normal and (b) all three vertices lie within `dist_thr` of the
//!      plane. After each growth pass the plane is **refit** from every vertex in
//!      the region, so a slightly bent patch converges to its best-fit plane
//!      instead of drifting along curvature. Thresholds are deliberately tight
//!      (defaults `angle_thr = 15°` ≈ π/12, `dist_thr = M/30` where M is the mean
//!      edge length, both from *Make It Flat* / 深圳大学 2024 — see docs/09 §1.1)
//!      so we **over-segment rather than miss** a plane (REFUTE honesty rule:
//!      false-negative > false-positive for a partition tool).
//!   3. **boundary extraction** — a region's outline is every mesh edge that
//!      exactly one in-region face uses. That is exactly the geometric region
//!      boundary (no fragile alpha-shape needed for v1; alpha-shape is deferred
//!      in docs/09 §6 backlog as an optional concave-hull refinement).
//!
//! Output: one `PlanarRegion` per flat patch that survives `min_region_faces`.
//! Each carries a `seed` (a representative interior face) so the frontend can
//! drop it straight into the existing ghost-suggestion → `seed_grow` union path
//! (iter58-64), plus `boundary_edges` so the patch can be outlined. This is the
//! "automatic suggested seeds" hook from docs/09 §4.

use nalgebra as na;
use petgraph::visit::EdgeRef;
use serde::{Deserialize, Serialize};

use crate::mesh::model::MeshModel;
use crate::segment::recommend::SeedSuggestion;

/// Tunable knobs for [`detect_planar_regions`]. Defaults mirror the cited
/// literature (docs/09 §1.1) and stay symmetric with the existing Dihedral
/// default of 30° in `docs/08`.
#[derive(Debug, Clone, Copy)]
pub struct PlanarParams {
    /// Max angle (degrees) between a candidate face normal and the region plane
    /// normal for the face to join. Default 15 (π/12).
    pub angle_thr_deg: f32,
    /// Plane-distance tolerance as a fraction of the mean edge length M. Default
    /// 1/30 — a face's vertices must lie within `M/30` of the fit plane.
    pub dist_thr_factor: f32,
    /// Regions with fewer faces than this are dropped (they are noise / too
    /// small to be a meaningful "plane"). Default derived from mesh size in the
    /// command layer, but exposed so callers/tests can tighten it.
    pub min_region_faces: usize,
}

impl Default for PlanarParams {
    fn default() -> Self {
        Self {
            angle_thr_deg: 15.0,
            dist_thr_factor: 1.0 / 30.0,
            min_region_faces: 2,
        }
    }
}

/// One detected continuous planar region.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanarRegion {
    /// Fit plane as `[a, b, c, d]` with `a·x + b·y + c·z + d = 0`, normalised.
    pub plane: [f32; 4],
    /// Number of triangle faces in the region.
    pub face_count: u32,
    /// A representative interior seed (the face whose centroid is closest to the
    /// region centroid) — ready to hand to `seed_grow` as a ghost suggestion.
    pub seed: SeedSuggestion,
    /// Outline of the region as 3D line segments: each entry is
    /// `[[x,y,z],[x,y,z]]` (an edge's two endpoints). Drawn as `lineSegments`.
    pub boundary_edges: Vec<[[f32; 3]; 2]>,
}

/// Mean edge length over all triangle edges — drives the absolute distance
/// tolerance `dist_thr = M * dist_thr_factor`.
fn mean_edge_length(mesh: &MeshModel) -> f32 {
    let mut sum = 0.0f32;
    let mut count = 0u64;
    for f in &mesh.faces {
        let vs = [
            mesh.vertices[f[0] as usize],
            mesh.vertices[f[1] as usize],
            mesh.vertices[f[2] as usize],
        ];
        let edges = [
            (vs[0], vs[1]),
            (vs[1], vs[2]),
            (vs[2], vs[0]),
        ];
        for (a, b) in edges {
            let d = ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2)).sqrt();
            sum += d;
            count += 1;
        }
    }
    if count == 0 {
        1.0
    } else {
        sum / count as f32
    }
}

/// Per-face flatness in [0,1]: 1 = shares a normal with every neighbour (flat
/// interior), 0 = perpendicular normals all around (sharp crease). Defined as
/// `1 − mean over neighbours of (1 − |n_f·n_g|)`, the same cheap proxy
/// `recommend.rs` uses for "interior-ness" — no k-ring PCA needed just to *order*
/// seeds (docs/09 notes k-ring PCA is the "proper" planarity but the curvature
/// proxy is a strong, O(1)-per-face stand-in that is plenty for seeding).
fn face_flatness(mesh: &MeshModel) -> Vec<f32> {
    let n = mesh.faces.len();
    let mut score = vec![0.0f32; n];
    if mesh.normals.len() != n {
        return score;
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
        let d = 1.0 - dot.abs().min(1.0);
        score[a] += d;
        score[b] += d;
    }
    for node in mesh.face_adjacency.node_indices() {
        let deg = mesh.face_adjacency.edges(node).count();
        let i = node.index();
        if deg > 0 {
            score[i] /= deg as f32;
        }
    }
    score.into_iter().map(|s| 1.0 - s).collect()
}

/// Fit a plane `[a,b,c,d]` (normalised) to a set of points via PCA: the normal
/// is the eigenvector of the covariance matrix with the **smallest** eigenvalue
/// (the direction of least variance = the plane normal). `d = −n·centroid`.
fn fit_plane(points: &[[f32; 3]]) -> [f32; 4] {
    let m = points.len().max(1) as f32;
    let mut mu = [0.0f32; 3];
    for p in points {
        mu[0] += p[0];
        mu[1] += p[1];
        mu[2] += p[2];
    }
    mu[0] /= m;
    mu[1] /= m;
    mu[2] /= m;

    let mut cov = na::Matrix3::<f32>::zeros();
    for p in points {
        let d = [p[0] - mu[0], p[1] - mu[1], p[2] - mu[2]];
        for i in 0..3 {
            for j in 0..3 {
                cov[(i, j)] += d[i] * d[j];
            }
        }
    }
    cov /= m;

    // Smallest-eigenvalue eigenvector = plane normal.
    let eig = na::linalg::SymmetricEigen::new(cov);
    let mut mini = 0usize;
    let mut minv = eig.eigenvalues[0];
    for k in 1..3 {
        if eig.eigenvalues[k] < minv {
            minv = eig.eigenvalues[k];
            mini = k;
        }
    }
    let col = eig.eigenvectors.column(mini);
    let (mut nx, mut ny, mut nz) = (col[0], col[1], col[2]);
    let len = (nx * nx + ny * ny + nz * nz).sqrt().max(1e-12);
    nx /= len;
    ny /= len;
    nz /= len;
    let d = -(nx * mu[0] + ny * mu[1] + nz * mu[2]);
    [nx, ny, nz, d]
}

/// Fit the initial plane from a single triangle (its 3 vertices).
fn fit_plane_from_face(mesh: &MeshModel, f: u32) -> [f32; 4] {
    let tri = mesh.faces[f as usize];
    let pts = [
        mesh.vertices[tri[0] as usize],
        mesh.vertices[tri[1] as usize],
        mesh.vertices[tri[2] as usize],
    ];
    fit_plane(&pts)
}

/// Signed distance from a point to a normalised plane.
#[inline]
fn plane_dist(plane: &[f32; 4], p: &[f32; 3]) -> f32 {
    plane[0] * p[0] + plane[1] * p[1] + plane[2] * p[2] + plane[3]
}

/// Detect continuous planar regions. See module docs for the algorithm.
///
/// Returns regions sorted by `face_count` descending (biggest flat patches
/// first). Faces that never join a region (curved surfaces, tiny artefacts) are
/// simply absent — this is an *advisory* detector feeding ghost seeds, not a
/// full relabelling; `seed_grow`'s fallback already covers whatever is left.
pub fn detect_planar_regions(mesh: &MeshModel, params: &PlanarParams) -> Vec<PlanarRegion> {
    let n = mesh.faces.len();
    if n == 0 {
        return Vec::new();
    }
    if mesh.normals.len() != n {
        // No normals → cannot judge coplanarity. Advisory no-op rather than a
        // wrong answer (honesty rule: unassessable → empty, not a guess).
        return Vec::new();
    }

    let m = mean_edge_length(mesh);
    let dist_thr = (m * params.dist_thr_factor).max(1e-6);
    let cos_thr = (params.angle_thr_deg as f64).to_radians().cos() as f32;

    let flatness = face_flatness(mesh);
    // Seed order: flattest faces first.
    let mut order: Vec<usize> = (0..n).collect();
    order.sort_by(|&a, &b| flatness[b].partial_cmp(&flatness[a]).unwrap_or(std::cmp::Ordering::Equal));

    let mut assigned = vec![false; n];
    let mut raw_regions: Vec<Vec<u32>> = Vec::new();

    for &seed in &order {
        if assigned[seed] {
            continue;
        }
        // Grow one region from this seed face.
        let mut region: Vec<u32> = vec![seed as u32];
        assigned[seed] = true;
        let mut plane = fit_plane_from_face(mesh, seed as u32);

        let mut frontier: Vec<u32> = Vec::new();
        loop {
            frontier.clear();
            for &f in &region {
                for e in mesh.face_adjacency.edges(petgraph::graph::NodeIndex::new(f as usize)) {
                    let g = e.target().index() as u32;
                    if g < n as u32 && !assigned[g as usize] {
                        frontier.push(g);
                    }
                }
            }
            if frontier.is_empty() {
                break;
            }
            let mut grew = false;
            for &g in &frontier {
                if assigned[g as usize] {
                    continue;
                }
                // (a) normal agreement with the region plane.
                let ng = mesh.normals[g as usize];
                let dot = plane[0] * ng[0] + plane[1] * ng[1] + plane[2] * ng[2];
                if dot.abs() < cos_thr {
                    continue; // angle too steep → not coplanar
                }
                // (b) every vertex within dist_thr of the plane.
                let tri = mesh.faces[g as usize];
                let mut ok = true;
                for &v in &tri {
                    let p = mesh.vertices[v as usize];
                    if plane_dist(&plane, &p).abs() > dist_thr {
                        ok = false;
                        break;
                    }
                }
                if !ok {
                    continue;
                }
                assigned[g as usize] = true;
                region.push(g);
                grew = true;
            }
            if !grew {
                break;
            }
            // Refit the plane from every vertex in the region.
            let mut pts: Vec<[f32; 3]> = Vec::with_capacity(region.len() * 3);
            for &f in &region {
                let tri = mesh.faces[f as usize];
                for &v in &tri {
                    pts.push(mesh.vertices[v as usize]);
                }
            }
            plane = fit_plane(&pts);
        }

        if region.len() >= params.min_region_faces {
            raw_regions.push(region);
        }
        // Faces below min_region_faces stay `assigned` (so they aren't reseeded
        // into another region) but are simply dropped from the output — they
        // remain the responsibility of `seed_grow`'s fallback.
    }

    // Build the public regions: seed point + boundary edges each.
    let centroids = mesh.face_centers();
    let mut out: Vec<PlanarRegion> = raw_regions
        .into_iter()
        .map(|region| {
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
                let edges = [(tri[0], tri[1]), (tri[1], tri[2]), (tri[2], tri[0])];
                for (a, b) in edges {
                    let key = (a.min(b), a.max(b));
                    *edge_count.entry(key).or_insert(0) += 1;
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

            // Plane from the whole region (final fit).
            let mut pts: Vec<[f32; 3]> = Vec::with_capacity(region.len() * 3);
            for &f in &region {
                let tri = mesh.faces[f as usize];
                for &v in &tri {
                    pts.push(mesh.vertices[v as usize]);
                }
            }
            let plane = fit_plane(&pts);

            PlanarRegion {
                plane,
                face_count: region.len() as u32,
                seed: SeedSuggestion {
                    point: seed_point,
                    face_index: best_f,
                },
                boundary_edges,
            }
        })
        .collect();

    out.sort_by(|a, b| b.face_count.cmp(&a.face_count));
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::segment::metrics::unit_cube;

    fn params_for_cube() -> PlanarParams {
        // The unit cube has 2 triangles per face; a region of 2 faces must pass.
        PlanarParams {
            angle_thr_deg: 15.0,
            dist_thr_factor: 1.0 / 30.0,
            min_region_faces: 2,
        }
    }

    #[test]
    fn unit_cube_yields_six_planar_regions() {
        let mesh = unit_cube();
        let regions = detect_planar_regions(&mesh, &params_for_cube());
        assert_eq!(regions.len(), 6, "a cube has 6 flat faces → 6 regions");
    }

    #[test]
    fn every_cube_region_has_a_closed_boundary() {
        let mesh = unit_cube();
        let regions = detect_planar_regions(&mesh, &params_for_cube());
        // Each face is a 2-triangle square → 4 boundary edges forming a loop.
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
        let regions = detect_planar_regions(&mesh, &params_for_cube());
        let total: u32 = regions.iter().map(|r| r.face_count).sum();
        assert_eq!(total, 12, "all 12 cube triangles should belong to a region");
    }

    #[test]
    fn empty_mesh_is_safe() {
        let mesh = MeshModel::new();
        assert!(detect_planar_regions(&mesh, &PlanarParams::default()).is_empty());
    }

    #[test]
    fn missing_normals_yields_no_regions() {
        let mut mesh = unit_cube();
        mesh.normals.clear();
        assert!(detect_planar_regions(&mesh, &params_for_cube()).is_empty());
    }

    #[test]
    fn seed_is_a_valid_face_reference() {
        let mesh = unit_cube();
        let regions = detect_planar_regions(&mesh, &params_for_cube());
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
