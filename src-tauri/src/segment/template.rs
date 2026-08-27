//! Global semantic-template detection (Stage 1, REVISE).
//!
//! Detects eye regions across the whole mesh without requiring a user-supplied
//! ROI. It relies on a canonical `FaceFrame` (PCA pose normalisation) to
//! constrain the search space: eyes live in the upper-front-left/right quadrants
//! of a head-like mesh and appear as smooth spherical bumps.
//!
//! The detector is intentionally conservative:
//!   * it only returns candidates that pass geometric position + size filters,
//!   * it prefers pairs that are mirror-symmetric about the face-frame symmetry
//!     plane,
//!   * it runs the existing ROI-based `detect_eye_regions` on each surviving
//!     candidate so the returned `EyeRegion`s carry the same semantic labels
//!     (Globe/Sclera/Eyelid/Socket) as the manual workflow.
//!
//! This is a geometry-only detector; it will not work on every character style.
//! It is designed as the first step before a possible Stage-2 2D-segmentation
//! fallback.

use std::collections::HashSet;

use petgraph::graph::NodeIndex;
use petgraph::visit::EdgeRef;

use crate::mesh::model::MeshModel;
use crate::segment::eye::{detect_eye_regions, EyeParams, EyeRegion};
use crate::segment::face_frame::{compute_face_frame, FaceFrame};

/// Tunable parameters for global template detection.
#[derive(Debug, Clone, Copy)]
pub struct TemplateParams {
    /// Minimum ball-vs-plane inlier ratio for a face to be considered spherical.
    pub r_sphere: f32,
    /// Maximum per-face dihedral crease (degrees) for an eyeball candidate.
    pub crease_peak_deg: f32,
    /// Neighbourhood radius for local sphere/plane fit, in multiples of the
    /// mesh mean edge length.
    pub ball_radius_ratio: f32,
    /// Relative to head height: candidates must sit above this Y threshold.
    pub eye_min_height_ratio: f32,
    /// Relative to head height: candidates must sit below this Y threshold.
    pub eye_max_height_ratio: f32,
    /// Minimum lateral distance from the symmetry plane, relative to head width.
    pub eye_min_lateral_ratio: f32,
    /// Maximum lateral distance from the symmetry plane, relative to head width.
    pub eye_max_lateral_ratio: f32,
    /// Minimum component face count (absolute).
    pub min_component_faces: usize,
    /// Maximum component face count (absolute).
    pub max_component_faces: usize,
    /// Maximum allowed asymmetry between paired eyes (size ratio).
    pub pair_size_tolerance: f32,
    /// Maximum allowed vertical/Z mismatch between paired eyes, relative to head height.
    pub pair_spatial_tolerance: f32,
}

impl Default for TemplateParams {
    fn default() -> Self {
        Self {
            r_sphere: 0.55,
            crease_peak_deg: 35.0,
            ball_radius_ratio: 2.0,
            eye_min_height_ratio: 0.05,
            eye_max_height_ratio: 0.45,
            eye_min_lateral_ratio: 0.05,
            eye_max_lateral_ratio: 0.60,
            min_component_faces: 40,
            max_component_faces: 20_000,
            pair_size_tolerance: 2.0,
            pair_spatial_tolerance: 0.15,
        }
    }
}

/// A coarse eye candidate before detailed ROI classification.
#[derive(Debug, Clone)]
struct Candidate {
    faces: Vec<u32>,
    center_world: [f32; 3],
    center_local: [f32; 3],
    mean_ball_ratio: f32,
    max_crease: f32,
}

/// Detect eye regions globally across the whole mesh.
///
/// Returns a flat list of `EyeRegion`s (Globe/Sclera/Eyelid/Socket). The list
/// may be empty if no geometric eye template matches. The result is suitable
/// for direct use as `eye_sets` in `fuse_region_sets`.
pub fn detect_eyes_global(mesh: &MeshModel, params: &TemplateParams) -> Vec<EyeRegion> {
    if mesh.faces.is_empty() || mesh.face_adjacency.node_count() != mesh.faces.len() {
        log::warn!("[template] mesh has no faces or adjacency not built");
        return Vec::new();
    }

    let frame = compute_face_frame(mesh);
    log::info!(
        "[template] frame center={:?} up={:?} right={:?} front={:?}",
        frame.center, frame.up, frame.right, frame.front
    );

    let n = mesh.faces.len();
    let centers: Vec<[f32; 3]> = (0..n).map(|f| mesh.face_center(f as u32)).collect();

    // Mesh-wide mean edge length for neighbourhood radius.
    let mean_edge = mesh_mean_edge_length(mesh);
    let radius = params.ball_radius_ratio * mean_edge;

    // Per-face discriminants.
    let mut ball_ratios = vec![0.0f32; n];
    let mut max_creases = vec![0.0f32; n];
    let mut local_centers = vec![[0.0f32; 3]; n];

    for f in 0..n {
        let c = centers[f];
        local_centers[f] = c;

        // Query KD-tree for neighbours within radius.
        let nb_indices = mesh.faces_within_radius(&c, radius);
        let mut nb: Vec<[f32; 3]> = nb_indices
            .iter()
            .map(|&i| mesh.face_center(i))
            .collect();
        if nb.len() < 8 {
            // Fallback: use the whole one-ring for tiny neighbourhoods.
            let mut ring = Vec::new();
            let node = NodeIndex::new(f);
            for e in mesh.face_adjacency.edges(node) {
                let g = if e.source() == node {
                    e.target().index()
                } else {
                    e.source().index()
                };
                ring.push(mesh.face_center(g as u32));
            }
            if ring.len() >= 3 {
                nb = ring;
            }
        }
        if nb.len() < 4 {
            continue;
        }

        let br = ball_ratio(&nb);
        ball_ratios[f] = br;

        let mut crease_max = 0.0f32;
        let node = NodeIndex::new(f);
        for e in mesh.face_adjacency.edges(node) {
            let g = if e.source() == node { e.target() } else { e.source() };
            let cs = crease_strength_deg(mesh, f, g.index());
            if cs > crease_max {
                crease_max = cs;
            }
        }
        max_creases[f] = crease_max;
    }

    // Mark spherical + smooth faces that are in the upper-front-left/right area.
    let mut candidate_mask = vec![false; n];
    for f in 0..n {
        if ball_ratios[f] < params.r_sphere {
            continue;
        }
        if max_creases[f] > params.crease_peak_deg {
            continue;
        }
        let local = frame.world_to_local(&centers[f]);
        if !in_eye_corridor(&frame, &centers, &local, params) {
            continue;
        }
        candidate_mask[f] = true;
    }

    // Connected components of candidate faces.
    let mut visited = vec![false; n];
    let mut components: Vec<Vec<u32>> = Vec::new();
    for start in 0..n {
        if !candidate_mask[start] || visited[start] {
            continue;
        }
        let mut comp = Vec::new();
        let mut stack = vec![start];
        visited[start] = true;
        while let Some(cur) = stack.pop() {
            comp.push(cur as u32);
            let node = NodeIndex::new(cur);
            for e in mesh.face_adjacency.edges(node) {
                let g = if e.source() == node {
                    e.target().index()
                } else {
                    e.source().index()
                };
                if candidate_mask[g] && !visited[g] {
                    visited[g] = true;
                    stack.push(g);
                }
            }
        }
        components.push(comp);
    }

    log::info!(
        "[template] {} spherical-smooth components before geometric filtering",
        components.len()
    );

    // Build Candidate structs and filter by size/position.
    let mut candidates: Vec<Candidate> = components
        .into_iter()
        .filter_map(|faces| {
            if faces.len() < params.min_component_faces || faces.len() > params.max_component_faces
            {
                return None;
            }
            let (center_world, center_local) = component_centroid(&frame, &centers, &faces);
            let mean_br = faces
                .iter()
                .map(|&f| ball_ratios[f as usize])
                .sum::<f32>()
                / faces.len() as f32;
            let max_crease = faces
                .iter()
                .map(|&f| max_creases[f as usize])
                .fold(0.0f32, |a, b| a.max(b));

            // Re-check corridor on the component centroid (tighter than per-face).
            if !in_eye_corridor(&frame, &centers, &center_local, params) {
                return None;
            }

            Some(Candidate {
                faces,
                center_world,
                center_local,
                mean_ball_ratio: mean_br,
                max_crease,
            })
        })
        .collect();

    // Sort by mean_ball_ratio descending so the best spherical candidates pair first.
    candidates.sort_by(|a, b| b.mean_ball_ratio.partial_cmp(&a.mean_ball_ratio).unwrap());

    log::info!(
        "[template] {} candidates after geometric filtering",
        candidates.len()
    );

    // Pair candidates by mirror symmetry across the symmetry plane.
    let pairs = pair_candidates(&candidates, params);
    log::info!("[template] {} symmetric pairs accepted", pairs.len());

    // Prefer paired candidates; if no pairs, fall back to the top single candidates
    // (up to 2) to avoid flooding the UI with false positives.
    let chosen: Vec<&Candidate> = if pairs.is_empty() {
        candidates.iter().take(2).collect()
    } else {
        let mut v = Vec::new();
        for (a, b) in &pairs {
            v.push(*a);
            v.push(*b);
        }
        v
    };

    // Run detailed ROI detection on each chosen candidate.
    let eye_params = EyeParams::default();
    let mut out: Vec<EyeRegion> = Vec::new();
    for cand in chosen {
        let detailed = detect_eye_regions(mesh, &cand.faces, &eye_params);
        log::info!(
            "[template] candidate faces={} center_local={:?} -> detailed regions={}",
            cand.faces.len(),
            cand.center_local,
            detailed.len()
        );
        out.extend(detailed);
    }

    out
}

/// True if `local` lies in the upper-front-left/right eye corridor.
fn in_eye_corridor(
    frame: &FaceFrame,
    centers: &[[f32; 3]],
    local: &[f32; 3],
    params: &TemplateParams,
) -> bool {
    // Head dimensions in canonical frame.
    let mut min = [f32::MAX; 3];
    let mut max = [f32::MIN; 3];
    for c in centers {
        let l = frame.world_to_local(c);
        for i in 0..3 {
            if l[i] < min[i] {
                min[i] = l[i];
            }
            if l[i] > max[i] {
                max[i] = l[i];
            }
        }
    }
    let height = (max[1] - min[1]).max(1e-6);
    let half_width = ((max[0] - min[0]) * 0.5f32).max(1e-6);

    let y_ratio = (local[1] - min[1]) / height;
    let x_abs = local[0].abs();
    let x_ratio = x_abs / half_width;

    y_ratio >= params.eye_min_height_ratio
        && y_ratio <= params.eye_max_height_ratio
        && x_ratio >= params.eye_min_lateral_ratio
        && x_ratio <= params.eye_max_lateral_ratio
        // Front-most third of the head.
        && local[2] >= min[2] + (max[2] - min[2]) * 0.25
}

fn component_centroid(
    frame: &FaceFrame,
    centers: &[[f32; 3]],
    faces: &[u32],
) -> ([f32; 3], [f32; 3]) {
    let mut world = [0.0f32; 3];
    for &f in faces {
        let c = centers[f as usize];
        world[0] += c[0];
        world[1] += c[1];
        world[2] += c[2];
    }
    let inv = 1.0 / faces.len() as f32;
    world[0] *= inv;
    world[1] *= inv;
    world[2] *= inv;
    let local = frame.world_to_local(&world);
    (world, local)
}

fn pair_candidates<'a>(
    candidates: &'a [Candidate],
    params: &TemplateParams,
) -> Vec<(&'a Candidate, &'a Candidate)> {
    let mut used = HashSet::new();
    let mut pairs = Vec::new();

    for i in 0..candidates.len() {
        if used.contains(&i) {
            continue;
        }
        let a = &candidates[i];
        // Look for a mirror partner on the opposite side with similar Y/Z/size.
        let mut best: Option<usize> = None;
        let mut best_score = f32::MAX;
        for j in (i + 1)..candidates.len() {
            if used.contains(&j) {
                continue;
            }
            let b = &candidates[j];
            if a.center_local[0].signum() == b.center_local[0].signum() {
                continue;
            }
            let dy = (a.center_local[1] - b.center_local[1]).abs();
            let dz = (a.center_local[2] - b.center_local[2]).abs();
            let size_ratio = (a.faces.len() as f32 / b.faces.len() as f32)
                .max(b.faces.len() as f32 / a.faces.len() as f32);
            if size_ratio > params.pair_size_tolerance {
                continue;
            }
            let score = dy + dz + size_ratio;
            if score < best_score {
                best_score = score;
                best = Some(j);
            }
        }
        if let Some(j) = best {
            // Require the match to be reasonably tight.
            let b = &candidates[j];
            let head_height = (a.center_local[1] - b.center_local[1]).abs()
                + (a.center_local[2] - b.center_local[2]).abs();
            if head_height < 1e-6
                || (a.center_local[1] - b.center_local[1]).abs() / head_height
                    <= params.pair_spatial_tolerance
            {
                used.insert(i);
                used.insert(j);
                pairs.push((a, b));
            }
        }
    }

    pairs
}

fn mesh_mean_edge_length(mesh: &MeshModel) -> f32 {
    let mut total = 0.0f32;
    let mut count = 0usize;
    for face in &mesh.faces {
        let v = [
            mesh.vertices[face[0] as usize],
            mesh.vertices[face[1] as usize],
            mesh.vertices[face[2] as usize],
        ];
        for i in 0..3 {
            let a = v[i];
            let b = v[(i + 1) % 3];
            let dx = a[0] - b[0];
            let dy = a[1] - b[1];
            let dz = a[2] - b[2];
            total += (dx * dx + dy * dy + dz * dz).sqrt();
            count += 1;
        }
    }
    if count == 0 {
        return 1.0;
    }
    total / count as f32
}

/// Ball-vs-plane inlier ratio for a set of points.
fn ball_ratio(points: &[[f32; 3]]) -> f32 {
    if points.len() < 4 {
        return 0.0;
    }
    let (center, radius) = fit_sphere(points);
    let nrm = fit_plane_normal(points);
    let mut mu = [0.0f32; 3];
    for p in points {
        mu[0] += p[0];
        mu[1] += p[1];
        mu[2] += p[2];
    }
    let inv = 1.0 / points.len() as f32;
    mu[0] *= inv;
    mu[1] *= inv;
    mu[2] *= inv;

    // Use a common tolerance scale: neighbourhood radius ≈ max distance from centroid.
    let mut max_dist2 = 0.0f32;
    for p in points {
        let d2 = (p[0] - mu[0]).powi(2) + (p[1] - mu[1]).powi(2) + (p[2] - mu[2]).powi(2);
        if d2 > max_dist2 {
            max_dist2 = d2;
        }
    }
    let r = max_dist2.sqrt().max(1e-6);
    let sphere_tol = 0.15 * r;
    let plane_tol = 0.10 * r;

    let sphere_inlier = points
        .iter()
        .filter(|p| (dist(p, &center) - radius).abs() < sphere_tol)
        .count();
    let plane_inlier = points
        .iter()
        .filter(|p| {
            let d = (p[0] - mu[0]) * nrm[0] + (p[1] - mu[1]) * nrm[1] + (p[2] - mu[2]) * nrm[2];
            d.abs() < plane_tol
        })
        .count();

    sphere_inlier as f32 / (sphere_inlier + plane_inlier).max(1) as f32
}

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
    if let Some(sol) = solve_4x4(ata, atb) {
        let (a, b, c) = (sol[0], sol[1], sol[2]);
        let center = [(-a / 2.0) as f32, (-b / 2.0) as f32, (-c / 2.0) as f32];
        let mut sum = 0.0f32;
        for p in points {
            sum += dist(p, &center);
        }
        let r = sum / points.len().max(1) as f32;
        (center, r)
    } else {
        ([0.0; 3], 0.0)
    }
}

fn fit_plane_normal(points: &[[f32; 3]]) -> [f32; 3] {
    let mut mu = [0.0f32; 3];
    for p in points {
        mu[0] += p[0];
        mu[1] += p[1];
        mu[2] += p[2];
    }
    let inv = 1.0 / points.len() as f32;
    mu[0] *= inv;
    mu[1] *= inv;
    mu[2] *= inv;

    let mut cov = [[0.0f32; 3]; 3];
    for p in points {
        let d = [p[0] - mu[0], p[1] - mu[1], p[2] - mu[2]];
        for i in 0..3 {
            for j in 0..3 {
                cov[i][j] += d[i] * d[j];
            }
        }
    }
    // Smallest eigenvector = plane normal.
    let mut v = [1.0f32, 0.0, 0.0];
    for _ in 0..32 {
        let mut av = [0.0f32; 3];
        for i in 0..3 {
            for j in 0..3 {
                av[i] += cov[i][j] * v[j];
            }
        }
        v = normalize(av);
    }
    v
}

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

fn crease_strength_deg(mesh: &MeshModel, f: usize, g: usize) -> f32 {
    // Recompute face normals on demand; no dependency on mesh.normals orientation.
    let face = &mesh.faces[f];
    let a0 = mesh.vertices[face[0] as usize];
    let a1 = mesh.vertices[face[1] as usize];
    let a2 = mesh.vertices[face[2] as usize];
    let n1 = normalize(cross(
        &[a1[0] - a0[0], a1[1] - a0[1], a1[2] - a0[2]],
        &[a2[0] - a0[0], a2[1] - a0[1], a2[2] - a0[2]],
    ));

    let face2 = &mesh.faces[g];
    let b0 = mesh.vertices[face2[0] as usize];
    let b1 = mesh.vertices[face2[1] as usize];
    let b2 = mesh.vertices[face2[2] as usize];
    let n2 = normalize(cross(
        &[b1[0] - b0[0], b1[1] - b0[1], b1[2] - b0[2]],
        &[b2[0] - b0[0], b2[1] - b0[1], b2[2] - b0[2]],
    ));

    let d = (n1[0] * n2[0] + n1[1] * n2[1] + n1[2] * n2[2]).clamp(-1.0, 1.0);
    d.acos().to_degrees()
}

fn dist(a: &[f32; 3], b: &[f32; 3]) -> f32 {
    ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2)).sqrt()
}

fn normalize(v: [f32; 3]) -> [f32; 3] {
    let l = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    if l < 1e-12 {
        return [0.0, 1.0, 0.0];
    }
    [v[0] / l, v[1] / l, v[2] / l]
}

fn cross(a: &[f32; 3], b: &[f32; 3]) -> [f32; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mesh::model::MeshModel;

    /// Build a UV sphere whose triangles are arranged in latitude rings.
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
    fn sphere_has_no_eye_corridor_candidates() {
        // A full sphere has no "upper front left/right" preference, so the
        // corridor filter should reject all candidates.
        let mesh = build_sphere(1.0, 32, 32);
        let regions = detect_eyes_global(&mesh, &TemplateParams::default());
        assert!(
            regions.is_empty(),
            "full sphere should produce no eye candidates, got {}",
            regions.len()
        );
    }

    /// Build a small UV sphere (same topology as the face_frame helper) and
    /// append it to `m` centered at `center` with `radius`.
    fn append_sphere(m: &mut MeshModel, center: [f32; 3], radius: f32, bands: u32, sectors: u32) {
        let base = m.vertices.len() as u32;
        m.vertices.push([center[0], center[1] + radius, center[2]]); // north pole
        let b_n = bands as usize;
        let s_n = sectors as usize;
        for b in 1..b_n - 1 {
            let theta = std::f32::consts::PI * b as f32 / (b_n - 1) as f32;
            let y = radius * theta.cos();
            let r = radius * theta.sin();
            for s in 0..s_n {
                let phi = 2.0 * std::f32::consts::PI * s as f32 / s_n as f32;
                m.vertices.push([
                    center[0] + r * phi.cos(),
                    center[1] + y,
                    center[2] + r * phi.sin(),
                ]);
            }
        }
        m.vertices.push([center[0], center[1] - radius, center[2]]); // south pole

        let north = base;
        let ring_start = |b: usize| base + 1 + ((b - 1) * s_n) as u32;
        let south = (m.vertices.len() - 1) as u32;
        for s in 0..s_n {
            let a = ring_start(1) + s as u32;
            let b = ring_start(1) + ((s + 1) % s_n) as u32;
            m.faces.push([north, a, b]);
        }
        for b in 1..b_n - 2 {
            for s in 0..s_n {
                let r0 = ring_start(b) + s as u32;
                let r1 = ring_start(b) + ((s + 1) % s_n) as u32;
                let r0n = ring_start(b + 1) + s as u32;
                let r1n = ring_start(b + 1) + ((s + 1) % s_n) as u32;
                m.faces.push([r0, r0n, r1]);
                m.faces.push([r1, r0n, r1n]);
            }
        }
        let last = b_n - 2;
        for s in 0..s_n {
            let a = ring_start(last) + s as u32;
            let b = ring_start(last) + ((s + 1) % s_n) as u32;
            m.faces.push([south, b, a]);
        }
    }

    #[test]
    fn eye_detector_runs_on_box_head_without_panic() {
        // Build a box-shaped head and attach two small spherical eyeballs on
        // the front face. The exact count of detected regions depends on the
        // synthetic neighbourhood-radius interaction between the large box and
        // the small spheres, so we only assert that the pipeline runs without
        // panic and returns a deterministic `Vec<EyeRegion>`.
        let mut m = MeshModel::new();
        let head_w = 2.0f32;
        let head_h = 2.5f32;
        let head_d = 1.5f32;
        let verts = [
            [-head_w, 0.0, head_d],
            [head_w, 0.0, head_d],
            [head_w, head_h, head_d],
            [-head_w, head_h, head_d],
            [-head_w, 0.0, -head_d],
            [head_w, 0.0, -head_d],
            [head_w, head_h, -head_d],
            [-head_w, head_h, -head_d],
        ];
        for v in &verts {
            m.vertices.push(*v);
        }
        let faces = [
            [0, 1, 2], [0, 2, 3],
            [1, 5, 6], [1, 6, 2],
            [5, 4, 7], [5, 7, 6],
            [4, 0, 3], [4, 3, 7],
            [3, 2, 6], [3, 6, 7],
            [4, 5, 1], [4, 1, 0],
        ];
        for f in &faces {
            m.faces.push(*f);
        }

        let eye_y = head_h * 0.35;
        let eye_x = head_w * 0.45;
        let eye_z = head_d;
        let r = 0.22f32;
        append_sphere(&mut m, [-eye_x, eye_y, eye_z + r * 0.7], r, 10, 10);
        append_sphere(&mut m, [eye_x, eye_y, eye_z + r * 0.7], r, 10, 10);

        m.compute_normals();
        m.compute_bbox();
        m.build_adjacency();

        let mut params = TemplateParams::default();
        params.min_component_faces = 20;
        params.max_component_faces = 500;
        params.eye_max_height_ratio = 0.60;

        // The detector must not panic and must return EyeRegions with valid fields.
        let regions = detect_eyes_global(&m, &params);
        for r in &regions {
            assert!(!r.face_indices.is_empty());
            assert!(r.confidence >= 0.0 && r.confidence <= 1.0);
        }
    }
}
