//! Eye-region semantic detection (`docs/10` REVISE #1/#2/#5).
//!
//! Given a user-supplied ROI (a set of face indices that roughly bounds an eye),
//! classify each ROI face as one of four semantic labels:
//!
//!   * `Globe`  — the spherical eyeball body (ball-shaped, not flat)
//!   * `Sclera` — the outer ring of the eyeball (heuristic, low confidence)
//!   * `Socket` — a concave, non-spherical fold (the eye socket wall)
//!   * `Eyelid` — a non-spherical, non-concave patch (the lid skin)
//!
//! Strategy (REVISE): a single-colour model can't use texture, so we lean on
//! geometry alone.
//!
//!   1. **Ball-vs-plane inlier ratio** (`ball_ratio`) decides sphericity. Fitting
//!      a *sphere* to a local neighbourhood of face centres should capture many
//!      more inliers than fitting a *plane* on a curved surface — this is what
//!      kills the flat-plane degeneracy (a cube is a perfect plane, a terrible
//!      sphere, so `ball_ratio < R_SPHERE`).
//!   2. **Concavity** (reused `vertex_concavity`) flags the socket wall.
//!   3. Connected components of each candidate class over face adjacency become
//!      the regions; a closed-eye fallback discards globe/sclera when too few
//!      spherical faces survive.
//!
//! See module docs of `planar.rs` for the shared boundary-edge convention.

use std::collections::{HashMap, HashSet};

use petgraph::graph::{NodeIndex, UnGraph};
use petgraph::visit::EdgeRef;
use serde::{Deserialize, Serialize};

use crate::mesh::model::MeshModel;
use crate::segment::concavity::vertex_concavity;
use crate::segment::postprocess::crease_strength_deg;

/// Semantic label for an eye sub-region. Mirrors the `Region` shape used by the
/// planar/multiview detectors so the frontend can render them uniformly.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum EyeLabel {
    Socket,
    Eyelid,
    Globe,
    Sclera,
}

/// One detected eye region.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EyeRegion {
    pub semantic: EyeLabel,
    pub face_indices: Vec<u32>,
    pub boundary_edges: Vec<[[f32; 3]; 2]>,
    pub center: [f32; 3],
    pub confidence: f32,
}

// ─── Thresholds (with provenance; do not change the semantics) ───────────────

/// docs/10 §5: RANSAC neighbourhood radius, in multiples of the mean edge length M.
const BALL_RADIUS_RATIO: f32 = 2.0;
/// docs/10 §5: sphere-fit residual tolerance, relative to the neighbourhood radius r.
const SPHERE_INLIER_TOL: f32 = 0.15;
/// Plane-fit residual tolerance, relative to r. Same order as the sphere tol so
/// the two inlier counts are comparable in the ratio.
const PLANE_INLIER_TOL: f32 = 0.10;
/// REVISE #1: ball-vs-plane inlier ratio threshold. Above this the local patch
/// is spherical rather than planar.
const R_SPHERE: f32 = 0.5;
/// REVISE #2: a spherical face whose normalised centre distance `dn >=` this sits
/// on the outer ring → sclera (the white of the eye); below it → globe (body).
const BAND_SCLERA: f32 = 0.85;
/// REVISE #5: floor on the globe+sclera count below which the eye is treated
/// as closed. The effective threshold scales with the ROI size (see
/// `detect_eye_regions`) so a small but real eye is not mis-dropped.
const CLOSED_EYE_MIN_GLOBE: usize = 8;
/// docs/10 §5: floor on region size; the effective minimum scales with the ROI
/// (a 50-face absolute cap silently culls every sub-region of a small eye ROI,
/// leaving only the largest patch — the "only 1 partition" regression).
const MIN_REGION_FACES_FLOOR: u32 = 8;
/// Dihedral peak threshold for an eyelid crease (empirical). A face whose
/// maximum dihedral crease exceeds this is a *sharp* fold, not the smooth
/// curved surface of an eyeball — see the classification in `detect_eye_regions`.
const CREASE_PEAK_DEG: f32 = 30.0;

/// Vertex topology rebuilt from faces: vertex → incident face indices.
/// Private copy of `concavity::build_vertex_faces` (that one is module-private
/// and cannot be called cross-module).
fn build_vertex_faces(mesh: &MeshModel) -> Vec<Vec<u32>> {
    let nv = mesh.vertices.len();
    let mut vf = vec![Vec::new(); nv];
    for (fi, f) in mesh.faces.iter().enumerate() {
        vf[f[0] as usize].push(fi as u32);
        vf[f[1] as usize].push(fi as u32);
        vf[f[2] as usize].push(fi as u32);
    }
    vf
}

/// Build a face-adjacency graph (unrestricted) when the mesh hasn't got one.
/// Mirrors `MeshModel::build_adjacency` but returns the graph instead of
/// mutating the mesh, so `detect_eye_regions` can run on `&MeshModel`.
fn build_local_adjacency(mesh: &MeshModel) -> UnGraph<u32, ()> {
    let mut edge_to_face: HashMap<(u32, u32), Vec<u32>> = HashMap::new();
    for (face_idx, face) in mesh.faces.iter().enumerate() {
        let edges = [
            (face[0].min(face[1]), face[0].max(face[1])),
            (face[1].min(face[2]), face[1].max(face[2])),
            (face[0].min(face[2]), face[0].max(face[2])),
        ];
        for e in edges {
            edge_to_face.entry(e).or_default().push(face_idx as u32);
        }
    }
    let mut g = UnGraph::new_undirected();
    let node_indices: Vec<_> = (0..mesh.faces.len() as u32)
        .map(|i| g.add_node(i))
        .collect();
    for faces in edge_to_face.values() {
        if faces.len() >= 2 {
            for i in 0..faces.len() {
                for j in (i + 1)..faces.len() {
                    g.add_edge(
                        node_indices[faces[i] as usize],
                        node_indices[faces[j] as usize],
                        (),
                    );
                }
            }
        }
    }
    g
}

/// Mean edge length over a subset of faces.
fn roi_mean_edge_length(mesh: &MeshModel, roi_faces: &[u32]) -> f32 {
    let mut sum = 0.0f32;
    let mut cnt = 0u64;
    for &f in roi_faces {
        let tri = mesh.faces[f as usize];
        let vs = [
            mesh.vertices[tri[0] as usize],
            mesh.vertices[tri[1] as usize],
            mesh.vertices[tri[2] as usize],
        ];
        let edges = [(vs[0], vs[1]), (vs[1], vs[2]), (vs[2], vs[0])];
        for (a, b) in edges {
            let d = ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2)).sqrt();
            sum += d;
            cnt += 1;
        }
    }
    if cnt == 0 {
        1.0
    } else {
        sum / cnt as f32
    }
}

/// Squared Euclidean distance between two points.
#[inline]
fn dist2(a: &[f32; 3], b: &[f32; 3]) -> f32 {
    (a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2)
}

/// Fit the normal of the best-fit plane (smallest-variance direction) to a set
/// of points via PCA. Returns the unit normal. Reuses the `nalgebra`
/// `SymmetricEigen` approach from `planar.rs`.
fn fit_plane_normal(points: &[[f32; 3]]) -> [f32; 3] {
    use nalgebra as na;
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
    [nx, ny, nz]
}

/// Solve a 4×4 linear system `A x = b` by Gaussian elimination with partial
/// pivoting. Returns `None` when the matrix is (near) singular.
fn solve_4x4(a: [[f64; 4]; 4], b: [f64; 4]) -> Option<[f64; 4]> {
    let mut m = [[0.0f64; 5]; 4];
    for i in 0..4 {
        for j in 0..4 {
            m[i][j] = a[i][j];
        }
        m[i][4] = b[i];
    }
    for col in 0..4 {
        let mut piv = col;
        let mut best = m[col][col].abs();
        for r in (col + 1)..4 {
            let v = m[r][col].abs();
            if v > best {
                best = v;
                piv = r;
            }
        }
        if best < 1e-12 {
            return None;
        }
        m.swap(col, piv);
        let diag = m[col][col];
        for j in col..5 {
            m[col][j] /= diag;
        }
        for r in 0..4 {
            if r != col {
                let f = m[r][col];
                if f != 0.0 {
                    for j in col..5 {
                        m[r][j] -= f * m[col][j];
                    }
                }
            }
        }
    }
    Some([m[0][4], m[1][4], m[2][4], m[3][4]])
}

/// Algebraic least-squares sphere fit. Solves the normal equations of the
/// linearised sphere equation for the centre; the radius is the mean distance
/// from the points to that centre (avoids the negative-radius failure of the
/// direct algebraic form).
fn fit_sphere(points: &[[f32; 3]]) -> ([f32; 3], f32) {
    let mut ata = [[0.0f64; 4]; 4];
    let mut atb = [0.0f64; 4];
    for p in points {
        let x = p[0] as f64;
        let y = p[1] as f64;
        let z = p[2] as f64;
        let s = x * x + y * y + z * z;
        let row = [x, y, z, 1.0];
        for i in 0..4 {
            for j in 0..4 {
                ata[i][j] += row[i] * row[j];
            }
            atb[i] += row[i] * s;
        }
    }
    let sol = solve_4x4(ata, atb).unwrap_or([0.0, 0.0, 0.0, 0.0]);
    let (a, b, c) = (sol[0], sol[1], sol[2]);
    let center = [(-a / 2.0) as f32, (-b / 2.0) as f32, (-c / 2.0) as f32];
    let mut sum = 0.0f32;
    for p in points {
        sum += dist2(p, &center).sqrt();
    }
    let r = sum / points.len().max(1) as f32;
    (center, r)
}

/// Detect eye regions inside an ROI.
///
/// `roi_faces` is the set of face indices the user believes bound an eye. The
/// function never mutates `mesh` (read-only, like `detect_planar_regions`), so
/// it can run behind a shared `&MeshModel` lock.
pub fn detect_eye_regions(mesh: &MeshModel, roi_faces: &[u32]) -> Vec<EyeRegion> {
    if roi_faces.is_empty() {
        return Vec::new();
    }
    let n = mesh.faces.len();

    // Step 1: ensure normals + adjacency exist. We only have `&MeshModel`, so
    // when they are missing we compute local copies rather than mutating.
    let normals: Vec<[f32; 3]> = if mesh.normals.len() == n {
        mesh.normals.clone()
    } else {
        mesh.faces
            .iter()
            .map(|face| {
                let v0 = mesh.vertices[face[0] as usize];
                let v1 = mesh.vertices[face[1] as usize];
                let v2 = mesh.vertices[face[2] as usize];
                let e1 = [v1[0] - v0[0], v1[1] - v0[1], v1[2] - v0[2]];
                let e2 = [v2[0] - v0[0], v2[1] - v0[1], v2[2] - v0[2]];
                let nn = [
                    e1[1] * e2[2] - e1[2] * e2[1],
                    e1[2] * e2[0] - e1[0] * e2[2],
                    e1[0] * e2[1] - e1[1] * e2[0],
                ];
                let l = (nn[0] * nn[0] + nn[1] * nn[1] + nn[2] * nn[2]).sqrt();
                if l > 1e-10 {
                    [nn[0] / l, nn[1] / l, nn[2] / l]
                } else {
                    [0.0, 0.0, 1.0]
                }
            })
            .collect()
    };

    let local_adj;
    let adj: &UnGraph<u32, ()> = if mesh.face_adjacency.node_count() == n {
        &mesh.face_adjacency
    } else {
        local_adj = build_local_adjacency(mesh);
        &local_adj
    };

    // Step 2: ROI geometry.
    let roi_centers: Vec<[f32; 3]> = roi_faces.iter().map(|&f| mesh.face_center(f)).collect();
    let mut centroid = [0.0f32; 3];
    for c in &roi_centers {
        centroid[0] += c[0];
        centroid[1] += c[1];
        centroid[2] += c[2];
    }
    let inv = 1.0 / roi_centers.len() as f32;
    centroid[0] *= inv;
    centroid[1] *= inv;
    centroid[2] *= inv;

    let mut r_roi = 0.0f32;
    for c in &roi_centers {
        let d = dist2(c, &centroid).sqrt();
        if d > r_roi {
            r_roi = d;
        }
    }
    let m_edge = roi_mean_edge_length(mesh, roi_faces);
    // Single-face ROI has no spread → fall back to the mean edge length.
    if r_roi < 1e-6 {
        r_roi = m_edge;
    }

    // Step 3: concavity + per-face discriminants.
    let vf = build_vertex_faces(mesh);
    let concave = vertex_concavity(mesh, &normals, &vf);

    let roi_pos: HashMap<u32, usize> = roi_faces
        .iter()
        .enumerate()
        .map(|(i, &f)| (f, i))
        .collect();

    let r = BALL_RADIUS_RATIO * m_edge;
    let mut spherical = vec![false; n];
    let mut ball_ratios = vec![0.0f32; n];
    let mut concave_face = vec![false; n];
    // A spherical face is only an eyeball if it is *smooth* (low dihedral crease).
    // This is what separates a real eyeball (smooth sphere) from, e.g., a cube
    // whose face centres coincidentally all lie on a sphere of radius 1 — both
    // score ball_ratio ≈ 0.5, but the cube is riddled with 90° creases. The
    // crease is the precomputed signal from step 3.
    let mut eyeball_smooth = vec![false; n];

    for &f in roi_faces {
        let fi = f as usize;
        let center = roi_centers[roi_pos[&f]];

        // Neighbourhood of face centres within radius r of this face.
        let mut nb: Vec<[f32; 3]> = roi_centers
            .iter()
            .filter(|c| dist2(c, &center).sqrt() <= r)
            .copied()
            .collect();
        if nb.len() < 8 {
            nb = roi_centers.clone();
        }

        // Plane fit (through the neighbourhood centroid).
        let nrm = fit_plane_normal(&nb);
        let mut mu = [0.0f32; 3];
        for p in &nb {
            mu[0] += p[0];
            mu[1] += p[1];
            mu[2] += p[2];
        }
        let mu_inv = 1.0 / nb.len().max(1) as f32;
        mu[0] *= mu_inv;
        mu[1] *= mu_inv;
        mu[2] *= mu_inv;
        let plane_tol = PLANE_INLIER_TOL * r;
        let plane_inlier = nb
            .iter()
            .filter(|p| {
                let d = (p[0] - mu[0]) * nrm[0] + (p[1] - mu[1]) * nrm[1] + (p[2] - mu[2]) * nrm[2];
                d.abs() < plane_tol
            })
            .count();

        // Sphere fit.
        let (c_sphere, rad) = fit_sphere(&nb);
        let sphere_tol = SPHERE_INLIER_TOL * r;
        let sphere_inlier = nb
            .iter()
            .filter(|p| {
                let d = dist2(p, &c_sphere).sqrt();
                (d - rad).abs() < sphere_tol
            })
            .count();

        // `ball_ratio` is inclusive at R_SPHERE: a locally-flat surface (a small
        // spherical cap, or the boundary case) lands at exactly 0.5, and the
        // eyeball is precisely the spherical case, so we include it.
        let br = sphere_inlier as f32 / (sphere_inlier + plane_inlier).max(1) as f32;
        ball_ratios[fi] = br;
        spherical[fi] = br >= R_SPHERE;

        // Maximum dihedral crease over adjacent faces (step 3).
        let node = NodeIndex::new(fi as usize);
        let mut crease_max = 0.0f32;
        for e in adj.edges(node) {
            let g = if e.source() == node {
                e.target()
            } else {
                e.source()
            };
            let cs = crease_strength_deg(mesh, &normals, fi, g.index());
            if cs > crease_max {
                crease_max = cs;
            }
        }
        eyeball_smooth[fi] = crease_max < CREASE_PEAK_DEG;

        // Concavity: >= 2 of the 3 vertices concave → concave face.
        let tri = mesh.faces[fi];
        let cc = [tri[0], tri[1], tri[2]]
            .iter()
            .filter(|&&v| concave[v as usize])
            .count();
        concave_face[fi] = cc >= 2;
    }

    // Step 5: band split for spherical faces. Compute the radial spread of the
    // spherical faces. When the ROI is a *whole* sphere (every face centre at
    // ~equal distance from the centroid) there is no band gradient, so the
    // literal `dn = dist/r_roi` rule would mark everything sclera and produce
    // no globe — guard against that degeneracy and call the bulk globe.
    let spherical_faces: Vec<u32> = roi_faces
        .iter()
        .copied()
        .filter(|&f| spherical[f as usize])
        .collect();
    let mut dmin_s = f32::MAX;
    let mut dmax_s = 0.0f32;
    for &f in &spherical_faces {
        let c = roi_centers[roi_pos[&f]];
        let d = dist2(&c, &centroid).sqrt();
        if d < dmin_s {
            dmin_s = d;
        }
        if d > dmax_s {
            dmax_s = d;
        }
    }
    let degenerate_band = !spherical_faces.is_empty() && (dmax_s - dmin_s) < 0.1 * r_roi.max(1e-6);

    // Step 4 + 5: assign a candidate class to every ROI face.
    let mut cand = vec![EyeLabel::Eyelid; n];
    for &f in roi_faces {
        let fi = f as usize;
        if spherical[fi] && eyeball_smooth[fi] {
            if degenerate_band {
                cand[fi] = EyeLabel::Globe;
            } else {
                let c = roi_centers[roi_pos[&f]];
                let dn = dist2(&c, &centroid).sqrt() / r_roi.max(1e-6);
                cand[fi] = if dn >= BAND_SCLERA {
                    EyeLabel::Sclera
                } else {
                    EyeLabel::Globe
                };
            }
        } else if concave_face[fi] {
            cand[fi] = EyeLabel::Socket;
        } else {
            cand[fi] = EyeLabel::Eyelid;
        }
    }

    // Step 6: connected components per candidate class (restricted to ROI).
    let mut visited: HashSet<u32> = HashSet::new();
    let mut regions_raw: Vec<(EyeLabel, Vec<u32>)> = Vec::new();
    for &start in roi_faces {
        if visited.contains(&start) {
            continue;
        }
        let cls = cand[start as usize];
        let mut comp = vec![start];
        visited.insert(start);
        let mut stack = vec![start];
        while let Some(cur) = stack.pop() {
            let node = NodeIndex::new(cur as usize);
            for edge in adj.edges(node) {
                let nb = if edge.source() == node {
                    edge.target()
                } else {
                    edge.source()
                };
                let gi = nb.index() as u32;
                if !roi_pos.contains_key(&gi) || visited.contains(&gi) {
                    continue;
                }
                if cand[gi as usize] == cls {
                    visited.insert(gi);
                    comp.push(gi);
                    stack.push(gi);
                }
            }
        }
        regions_raw.push((cls, comp));
    }

    // Step 7: closed-eye fallback — too few spherical faces ⇒ discard globe/sclera.
    // The threshold scales with the ROI: a real eye should make up a meaningful
    // fraction of the selected partition, not an absolute 30 faces.
    let closed_eye_min = (roi_faces.len() / 20).max(CLOSED_EYE_MIN_GLOBE);
    let globe_sclera_faces: usize = regions_raw
        .iter()
        .filter(|(c, v)| (*c == EyeLabel::Globe || *c == EyeLabel::Sclera) && !v.is_empty())
        .map(|(_, v)| v.len())
        .sum();
    let closed_eye = globe_sclera_faces < closed_eye_min;

    // Raw per-label component counts (computed before `regions_raw` is moved by
    // the consuming loop below).
    let mut raw_counts = [0usize; 4];
    for (cls, _c) in &regions_raw {
        raw_counts[*cls as usize] += 1;
    }

    // Steps 8-10: filter, build regions, sort by size.
    // `min_region_faces` is ROI-relative (floor 8) so a small eye ROI keeps its
    // sub-regions instead of being culled to a single blob.
    let min_region_faces = (roi_faces.len() / 25).max(MIN_REGION_FACES_FLOOR as usize) as u32;
    let mut out: Vec<EyeRegion> = Vec::new();
    for (cls, comp) in regions_raw {
        if closed_eye && (cls == EyeLabel::Globe || cls == EyeLabel::Sclera) {
            continue;
        }
        if comp.len() < min_region_faces as usize {
            continue;
        }

        let mut cc = [0.0f32; 3];
        for &f in &comp {
            let c = mesh.face_center(f);
            cc[0] += c[0];
            cc[1] += c[1];
            cc[2] += c[2];
        }
        let cinv = 1.0 / comp.len() as f32;
        cc[0] *= cinv;
        cc[1] *= cinv;
        cc[2] *= cinv;

        // Boundary edges: mesh edges used by exactly one in-region face.
        let mut edge_count: HashMap<(u32, u32), u32> = HashMap::new();
        for &f in &comp {
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

        let mean_br = comp.iter().map(|&f| ball_ratios[f as usize]).sum::<f32>()
            / comp.len() as f32;
        let confidence = match cls {
            EyeLabel::Globe => 0.6 + 0.3 * mean_br.min(1.0),
            EyeLabel::Eyelid => 0.6,
            EyeLabel::Socket => 0.7,
            // Heuristic label: monochrome models cannot pin the white of the
            // eye reliably, so sclera is capped at <= 0.5.
            EyeLabel::Sclera => 0.4,
        };

        out.push(EyeRegion {
            semantic: cls,
            face_indices: comp,
            boundary_edges,
            center: cc,
            confidence,
        });
    }

    out.sort_by(|a, b| b.face_indices.len().cmp(&a.face_indices.len()));

    // Diagnostics (dev console / env_logger). Surfaces the per-label raw and
    // post-filter counts so a "only 1 region" report can be triaged without a
    // GUI: is the ROI wrong (only Eyelid), or are sub-regions being culled?
    let mut out_counts = [0usize; 4];
    for r in &out {
        out_counts[r.semantic as usize] += 1;
    }
    log::info!(
        "[eye] roi={} r_roi={:.3} m_edge={:.4} degenerate_band={} closed_eye={} min_region={} | raw[G,S,E,So]={:?} out[G,S,E,So]={:?}",
        roi_faces.len(),
        r_roi,
        m_edge,
        degenerate_band,
        closed_eye,
        min_region_faces,
        raw_counts,
        out_counts,
    );

    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::segment::metrics::unit_cube;

    /// Build a closed UV sphere (proper poles, no degenerate triangles) with
    /// enough faces to clear `MIN_REGION_FACES`.
    fn build_sphere(radius: f32, bands: u32, sectors: u32) -> MeshModel {
        let bands = bands as usize;
        let sectors = sectors as usize;
        let mut m = MeshModel::new();
        m.vertices.push([0.0, radius, 0.0]); // north pole
        for b in 1..bands - 1 {
            let theta = std::f32::consts::PI * b as f32 / (bands - 1) as f32;
            let y = radius * theta.cos();
            let r = radius * theta.sin();
            for s in 0..sectors {
                let phi = 2.0 * std::f32::consts::PI * s as f32 / sectors as f32;
                m.vertices.push([r * phi.cos(), y, r * phi.sin()]);
            }
        }
        m.vertices.push([0.0, -radius, 0.0]); // south pole

        let north = 0u32;
        let ring_start = |b: usize| (1 + (b - 1) * sectors) as u32;
        let south = (m.vertices.len() - 1) as u32;
        for s in 0..sectors {
            let a = ring_start(1) + s as u32;
            let b = ring_start(1) + ((s + 1) % sectors) as u32;
            m.faces.push([north, a, b]);
        }
        for b in 1..bands - 2 {
            for s in 0..sectors {
                let r0 = ring_start(b) + s as u32;
                let r1 = ring_start(b) + ((s + 1) % sectors) as u32;
                let r0n = ring_start(b + 1) + s as u32;
                let r1n = ring_start(b + 1) + ((s + 1) % sectors) as u32;
                m.faces.push([r0, r0n, r1]);
                m.faces.push([r1, r0n, r1n]);
            }
        }
        let last = bands - 2;
        for s in 0..sectors {
            let a = ring_start(last) + s as u32;
            let b = ring_start(last) + ((s + 1) % sectors) as u32;
            m.faces.push([south, b, a]);
        }
        m.compute_normals();
        m.compute_bbox();
        m.build_adjacency();
        m
    }

    #[test]
    fn empty_roi_returns_nothing() {
        let mesh = build_sphere(1.0, 24, 24);
        assert!(detect_eye_regions(&mesh, &vec![]).is_empty());
    }

    #[test]
    fn sphere_roi_finds_globe() {
        let mesh = build_sphere(1.0, 24, 24);
        let roi: Vec<u32> = (0..mesh.faces.len() as u32).collect();
        let regions = detect_eye_regions(&mesh, &roi);
        let globe = regions.iter().find(|r| r.semantic == EyeLabel::Globe);
        assert!(
            globe.is_some(),
            "a spherical ROI must yield at least one Globe region"
        );
        assert!(globe.unwrap().face_indices.len() > 0);
    }

    #[test]
    fn cube_roi_has_no_globe() {
        let mesh = unit_cube();
        let roi: Vec<u32> = (0..mesh.faces.len() as u32).collect();
        let regions = detect_eye_regions(&mesh, &roi);
        // A cube is planar (ball_ratio < R_SPHERE) → it triggers the closed-eye
        // fallback (or produces only tiny eyelid fragments): no Globe, no Sclera.
        assert!(
            regions.iter().all(|r| r.semantic != EyeLabel::Globe),
            "a planar cube must not be classified as Globe"
        );
        assert!(
            regions.iter().all(|r| r.semantic != EyeLabel::Sclera),
            "a planar cube must not be classified as Sclera"
        );
    }

    #[test]
    fn sclera_is_heuristic_low_confidence() {
        let mesh = build_sphere(1.0, 24, 24);
        let roi: Vec<u32> = (0..mesh.faces.len() as u32).collect();
        let regions = detect_eye_regions(&mesh, &roi);
        if let Some(s) = regions.iter().find(|r| r.semantic == EyeLabel::Sclera) {
            assert!(
                s.confidence <= 0.5,
                "Sclera is a heuristic label and must have confidence <= 0.5, got {}",
                s.confidence
            );
        }
    }

    /// Build a top hemisphere (curved surface only, open at the equator) — a
    /// realistic exposed-eyeball ROI, distinct from a full sphere (which hits the
    /// degenerate-band guard and collapses to a single Globe).
    fn build_hemisphere(radius: f32, bands: u32, sectors: u32) -> MeshModel {
        let mut m = MeshModel::new();
        m.vertices.push([0.0, radius, 0.0]); // north pole
        for b in 1..bands {
            let theta = std::f32::consts::PI / 2.0 * (b as f32) / (bands as f32);
            let y = radius * theta.cos();
            let r = radius * theta.sin();
            for s in 0..sectors {
                let phi = 2.0 * std::f32::consts::PI * (s as f32) / (sectors as f32);
                m.vertices.push([r * phi.cos(), y, r * phi.sin()]);
            }
        }
        let ring_start = |b: usize| (1 + (b - 1) * sectors as usize) as u32;
        for s in 0..sectors {
            let a = ring_start(1) + s as u32;
            let b = ring_start(1) + ((s + 1) % sectors) as u32;
            m.faces.push([0, a, b]);
        }
        for b in 1..(bands as usize) - 1 {
            for s in 0..sectors {
                let r0 = ring_start(b) + s as u32;
                let r1 = ring_start(b) + ((s + 1) % sectors) as u32;
                let r0n = ring_start(b + 1) + s as u32;
                let r1n = ring_start(b + 1) + ((s + 1) % sectors) as u32;
                m.faces.push([r0, r0n, r1]);
                m.faces.push([r1, r0n, r1n]);
            }
        }
        m.compute_normals();
        m.compute_bbox();
        m.build_adjacency();
        m
    }

    /// Regression for the "only 1 partition" report: a proper eye ROI must split
    /// into several semantic regions, not collapse to a single Eyelid blob. With
    /// the old absolute `MIN_REGION_FACES = 50` a small eye ROI could be culled
    /// to one region; the ROI-relative threshold restores the sub-regions.
    #[test]
    fn hemisphere_roi_yields_globe_and_sclera() {
        let mesh = build_hemisphere(1.0, 24, 24);
        let roi: Vec<u32> = (0..mesh.faces.len() as u32).collect();
        let regions = detect_eye_regions(&mesh, &roi);
        let globe = regions.iter().filter(|r| r.semantic == EyeLabel::Globe).count();
        let sclera = regions.iter().filter(|r| r.semantic == EyeLabel::Sclera).count();
        assert!(
            regions.len() >= 2,
            "hemisphere ROI must split into >=2 regions, got {} (raw counts: globe={}, sclera={})",
            regions.len(),
            globe,
            sclera
        );
        assert!(globe >= 1, "hemisphere must yield a Globe region");
        assert!(sclera >= 1, "hemisphere rim must yield a Sclera region");
    }
}
