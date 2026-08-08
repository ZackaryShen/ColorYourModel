//! Segmentation accuracy metrics (PSB-style) + synthetic golden-sample harness.
//!
//! Acceptance philosophy (REFUTE blocker #7): we do NOT self-certify. Thresholds
//! come from external public PSB baselines, never from self-measured numbers; the
//! synthetic samples below are functional *smoke* tests (topological facts like
//! "a single smooth sphere is one connected region"), and they print live metric
//! numbers for the user to judge — but they never assert a fake "pass ≥ baseline".
//! Real PSB .off meshes drop into `examples/golden/` (see `golden_eval`) for a
//! genuine cross-check once an OFF loader lands.

use std::collections::HashSet;

use petgraph::visit::EdgeRef;

use crate::mesh::model::MeshModel;

/// Pair-counting Rand Index between two labelings. 1.0 = identical partition.
pub fn rand_index(a: &[u32], b: &[u32]) -> f64 {
    let n = a.len();
    if n != b.len() || n < 2 {
        return 0.0;
    }
    let mut a_same = 0u64;
    let mut b_same = 0u64;
    let mut both = 0u64;
    for i in 0..n {
        for j in (i + 1)..n {
            let sa = a[i] == a[j];
            let sb = b[i] == b[j];
            if sa {
                a_same += 1;
            }
            if sb {
                b_same += 1;
            }
            if sa == sb {
                both += 1;
            }
        }
    }
    if a_same + b_same == 0 {
        return 1.0;
    }
    both as f64 / (n as u64 * (n as u64 - 1) / 2) as f64
}

/// Set of undirected boundary edges (face-index pairs) where labels differ.
fn boundary_edge_set(mesh: &MeshModel, labels: &[u32]) -> HashSet<(u32, u32)> {
    let mut set = HashSet::new();
    for edge in mesh.face_adjacency.edge_references() {
        let fi = mesh.face_adjacency[edge.source()];
        let fj = mesh.face_adjacency[edge.target()];
        if labels[fi as usize] != labels[fj as usize] {
            let (a, b) = if fi <= fj { (fi, fj) } else { (fj, fi) };
            set.insert((a, b));
        }
    }
    set
}

/// Boundary precision / recall / F1 between auto labels `a` and reference `b`.
/// Measures agreement on *where* the cuts are, the most human-meaningful metric.
pub fn boundary_f_score(a: &[u32], b: &[u32], mesh: &MeshModel) -> (f64, f64, f64) {
    let ea = boundary_edge_set(mesh, a);
    let eb = boundary_edge_set(mesh, b);
    if ea.is_empty() && eb.is_empty() {
        return (1.0, 1.0, 1.0);
    }
    let inter = ea.intersection(&eb).count() as f64;
    let prec = if ea.is_empty() { 1.0 } else { inter / ea.len() as f64 };
    let rec = if eb.is_empty() { 1.0 } else { inter / eb.len() as f64 };
    let f1 = if prec + rec > 0.0 {
        2.0 * prec * rec / (prec + rec)
    } else {
        0.0
    };
    (prec, rec, f1)
}

/// Direct Hamming distance (fraction of faces with differing labels). Cheap
/// sanity proxy; sensitive to label permutation, so pair with Rand for context.
pub fn hamming_distance(a: &[u32], b: &[u32]) -> f64 {
    if a.len() != b.len() || a.is_empty() {
        return 1.0;
    }
    let diff = a.iter().zip(b).filter(|(x, y)| x != y).count();
    diff as f64 / a.len() as f64
}

/// True iff every label forms a single connected component over face adjacency.
/// Directly validates REFUTE blocker #2 (merging never creates connectivity).
pub fn all_labels_connected(mesh: &MeshModel, labels: &[u32]) -> bool {
    let ids: HashSet<u32> = labels.iter().copied().collect();
    for &lab in &ids {
        if !is_connected(mesh, labels, lab) {
            return false;
        }
    }
    true
}

fn is_connected(mesh: &MeshModel, labels: &[u32], label: u32) -> bool {
    let start = match labels.iter().position(|&l| l == label) {
        Some(p) => p as u32,
        None => return true,
    };
    let mut visited = vec![false; labels.len()];
    let mut stack = vec![start];
    visited[start as usize] = true;
    let mut seen = 0u32;
    while let Some(cur) = stack.pop() {
        seen += 1;
        let node = petgraph::graph::NodeIndex::new(cur as usize);
        for edge in mesh.face_adjacency.edges(node) {
            let nb = if edge.source() == node {
                edge.target()
            } else {
                edge.source()
            };
            let ni = nb.index();
            if labels[ni] == label && !visited[ni] {
                visited[ni] = true;
                stack.push(ni as u32);
            }
        }
    }
    let total = labels.iter().filter(|&&l| l == label).count() as u32;
    seen == total
}

// ─── Synthetic golden samples (deterministic, no external download) ──────────
// Test-only: the shipped binary never builds synthetic geometry, so gating the
// whole block keeps it out of the release artifact (and out of dead_code noise).

/// Closed UV sphere with welded seam and proper (non-degenerate) poles.
#[cfg(test)]
fn uv_sphere(radius: f32, c: [f32; 3], bands: usize, sectors: usize) -> (Vec<[f32; 3]>, Vec<u32>) {
    assert!(bands >= 3);
    let mut verts = Vec::new();
    verts.push([c[0], c[1] + radius, c[2]]); // north pole
    for b in 1..bands - 1 {
        let theta = std::f32::consts::PI * b as f32 / (bands - 1) as f32;
        let y = c[1] + radius * theta.cos();
        let r = radius * theta.sin();
        for s in 0..sectors {
            let phi = 2.0 * std::f32::consts::PI * s as f32 / sectors as f32;
            verts.push([c[0] + r * phi.cos(), y, c[2] + r * phi.sin()]);
        }
    }
    verts.push([c[0], c[1] - radius, c[2]]); // south pole
    let north = 0u32;
    // Indices are u32 to match `MeshModel::faces`; keeping the closure's return
    // type in the same domain avoids a dozen usize/u32 casts at every call site.
    let ring_start = |b: usize| (1 + (b - 1) * sectors) as u32;
    let south = (verts.len() - 1) as u32;
    let mut faces = Vec::new();
    for s in 0..sectors {
        let a = ring_start(1) + s as u32;
        let b = ring_start(1) + ((s + 1) % sectors) as u32;
        faces.push(north);
        faces.push(a);
        faces.push(b);
    }
    for b in 1..bands - 2 {
        for s in 0..sectors {
            let r0 = ring_start(b) + s as u32;
            let r1 = ring_start(b) + ((s + 1) % sectors) as u32;
            let r0n = ring_start(b + 1) + s as u32;
            let r1n = ring_start(b + 1) + ((s + 1) % sectors) as u32;
            faces.push(r0);
            faces.push(r0n);
            faces.push(r1);
            faces.push(r1);
            faces.push(r0n);
            faces.push(r1n);
        }
    }
    let last = bands - 2;
    for s in 0..sectors {
        let a = ring_start(last) + s as u32;
        let b = ring_start(last) + ((s + 1) % sectors) as u32;
        faces.push(south);
        faces.push(b);
        faces.push(a);
    }
    (verts, faces)
}

/// Fully prepare a MeshModel (normals, bbox, kdtrees, adjacency, default labels).
#[cfg(test)]
fn build_mesh(verts: &[[f32; 3]], faces_flat: &[u32]) -> MeshModel {
    let mut m = MeshModel::new();
    m.vertices = verts.to_vec();
    m.faces = faces_flat
        .chunks(3)
        .map(|c| [c[0], c[1], c[2]])
        .collect();
    m.compute_normals();
    m.compute_bbox();
    m.build_kdtree();
    m.build_vertex_kdtree();
    m.build_adjacency();
    let n = m.faces.len();
    m.segment_labels = vec![0u32; n];
    m.face_colors = vec![[138, 138, 138, 255]; n];
    m
}

/// Single smooth sphere → expected 1 connected part.
#[cfg(test)]
fn single_sphere() -> MeshModel {
    let (v, f) = uv_sphere(1.0, [0.0, 0.0, 0.0], 24, 24);
    build_mesh(&v, &f)
}

/// Two disconnected spheres touching at a point → expected 2 connected parts.
#[cfg(test)]
fn two_spheres() -> MeshModel {
    let (va, fa) = uv_sphere(1.0, [-1.001, 0.0, 0.0], 20, 20);
    let (vb, fb) = uv_sphere(1.0, [1.001, 0.0, 0.0], 20, 20);
    let mut v = va;
    let offset = v.len() as u32;
    v.extend(vb);
    let mut f = fa;
    f.extend(fb.iter().map(|x| x + offset));
    build_mesh(&v, &f)
}

/// Axis-aligned cube, 2 triangles per face → expected 6 flat parts.
///
/// Negative control for the contour merge: a rule that dissolves *every*
/// boundary would make the sphere test pass for entirely the wrong reason, so
/// something with real creases has to stay split.
#[cfg(test)]
pub(crate) fn unit_cube() -> MeshModel {
    let v: Vec<[f32; 3]> = vec![
        [-1.0, -1.0, -1.0],
        [1.0, -1.0, -1.0],
        [1.0, 1.0, -1.0],
        [-1.0, 1.0, -1.0],
        [-1.0, -1.0, 1.0],
        [1.0, -1.0, 1.0],
        [1.0, 1.0, 1.0],
        [-1.0, 1.0, 1.0],
    ];
    #[rustfmt::skip]
    let f: Vec<u32> = vec![
        0, 3, 2,  0, 2, 1, // -Z
        4, 5, 6,  4, 6, 7, // +Z
        0, 1, 5,  0, 5, 4, // -Y
        3, 7, 6,  3, 6, 2, // +Y
        0, 4, 7,  0, 7, 3, // -X
        1, 2, 6,  1, 6, 5, // +X
    ];
    build_mesh(&v, &f)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::segment::curvature::segment_by_curvature_kmeans;
    use crate::segment::dihedral::segment_by_dihedral_angle;
    use crate::segment::sdf::segment_by_sdf;

    fn noop(_: f32, _: &str) {}

    fn region_count(mesh: &MeshModel) -> usize {
        mesh.segments.len()
    }

    /// Every algorithm must yield only connected labels, and must never breach
    /// the manual-segment label namespace (REFUTE blocker #5).
    fn check_invariants(mesh: &MeshModel, labels: &[u32], tag: &str) {
        assert!(
            all_labels_connected(mesh, labels),
            "{tag}: produced disconnected fragments (REFUTE #2 violated)"
        );
        let max_label = labels.iter().copied().max().unwrap_or(0);
        assert!(
            max_label < 100_000,
            "{tag}: label {max_label} breaches MANUAL_SEGMENT_OFFSET namespace"
        );
    }

    /// The contour merge must dissolve fake borders, not real ones. A cube's
    /// 90° edges are the textbook real ones, so both the boundary-based baseline
    /// and the feature clusterer have to come back with exactly six faces.
    #[test]
    fn cube_stays_six_faces() {
        for (tag, use_sdf) in [("curvature(no-sdf)", false), ("curvature(sdf)", true)] {
            let mut mesh = unit_cube();
            segment_by_curvature_kmeans(&mut mesh, 6, 0, use_sdf, 0.0, &noop);
            let labels = mesh.segment_labels.clone();
            check_invariants(&mesh, &labels, tag);
            println!("[golden:cube] {} -> {} regions", tag, region_count(&mesh));
            assert_eq!(region_count(&mesh), 6, "{tag}: a cube has six flat faces");
        }
        let mut mesh = unit_cube();
        segment_by_dihedral_angle(&mut mesh, 30.0, &noop);
        assert_eq!(region_count(&mesh), 6, "dihedral: a cube has six flat faces");
    }

    #[test]
    fn single_sphere_is_one_connected_part_for_all_algorithms() {
        for (tag, mut mesh, expected) in [
            ("dihedral", single_sphere(), 1usize),
            ("curvature(no-sdf)", single_sphere(), 1),
            ("curvature(sdf)", single_sphere(), 1),
            ("sdf", single_sphere(), 1),
        ] {
            let labels = match tag {
                "dihedral" => {
                    segment_by_dihedral_angle(&mut mesh, 30.0, &noop);
                    mesh.segment_labels.clone()
                }
                "curvature(no-sdf)" => {
                    segment_by_curvature_kmeans(&mut mesh, 6, 1, false, 0.0, &noop);
                    mesh.segment_labels.clone()
                }
                "curvature(sdf)" => {
                    segment_by_curvature_kmeans(&mut mesh, 6, 1, true, 0.0, &noop);
                    mesh.segment_labels.clone()
                }
                "sdf" => {
                    segment_by_sdf(&mut mesh, 0, &noop);
                    mesh.segment_labels.clone()
                }
                _ => unreachable!(),
            };
            let n = region_count(&mesh);
            println!("[golden:single_sphere] {tag} -> {n} regions");
            check_invariants(&mesh, &labels, tag);
            assert_eq!(n, expected, "{tag}: expected {expected} region(s)");
        }
    }

    #[test]
    fn two_spheres_are_two_separate_parts_for_all_algorithms() {
        for (tag, mut mesh) in [
            ("dihedral", two_spheres()),
            ("curvature(no-sdf)", two_spheres()),
            ("curvature(sdf)", two_spheres()),
            ("sdf", two_spheres()),
        ] {
            let labels = match tag {
                "dihedral" => {
                    segment_by_dihedral_angle(&mut mesh, 30.0, &noop);
                    mesh.segment_labels.clone()
                }
                "curvature(no-sdf)" => {
                    segment_by_curvature_kmeans(&mut mesh, 6, 1, false, 0.0, &noop);
                    mesh.segment_labels.clone()
                }
                "curvature(sdf)" => {
                    segment_by_curvature_kmeans(&mut mesh, 6, 1, true, 0.0, &noop);
                    mesh.segment_labels.clone()
                }
                "sdf" => {
                    segment_by_sdf(&mut mesh, 0, &noop);
                    mesh.segment_labels.clone()
                }
                _ => unreachable!(),
            };
            let n = region_count(&mesh);
            println!("[golden:two_spheres] {tag} -> {n} regions");
            check_invariants(&mesh, &labels, tag);
            assert_eq!(n, 2, "{tag}: two disconnected spheres must stay 2 parts");
        }
    }

    #[test]
    fn curvature_kmeans_deterministic() {
        // Re-running must be bit-identical (no RNG) — guards REFUTE blocker #6.
        let mut a = two_spheres();
        let mut b = two_spheres();
        segment_by_curvature_kmeans(&mut a, 6, 1, true, 0.0, &noop);
        segment_by_curvature_kmeans(&mut b, 6, 1, true, 0.0, &noop);
        assert_eq!(a.segment_labels, b.segment_labels, "k-means must be deterministic");
    }

    #[test]
    fn metrics_sanity() {
        let m = single_sphere();
        let a = m.segment_labels.clone();
        // Self-comparison is a perfect match.
        assert!((rand_index(&a, &a) - 1.0).abs() < 1e-9);
        assert_eq!(hamming_distance(&a, &a), 0.0);
        let (p, _r, f) = boundary_f_score(&a, &a, &m);
        assert!((p - 1.0).abs() < 1e-9 && (f - 1.0).abs() < 1e-9);
    }

    /// Reports live metric numbers for the user. Does NOT assert a fake pass
    /// threshold; prints so a human can judge accuracy against PSB baselines.
    #[test]
    fn golden_eval_reports_metrics() {
        let mut mesh = two_spheres();
        let labels = {
            segment_by_curvature_kmeans(&mut mesh, 6, 1, true, 0.0, &noop);
            mesh.segment_labels.clone()
        };
        // Reference = the topological truth: two disconnected blobs.
        let gt: Vec<u32> = mesh
            .faces
            .iter()
            .enumerate()
            .map(|(i, _)| {
                // faces 0..half belong to sphere A, rest to sphere B
                if i < mesh.faces.len() / 2 {
                    0
                } else {
                    1
                }
            })
            .collect();
        let ri = rand_index(&labels, &gt);
        let (p, r, f) = boundary_f_score(&labels, &gt, &mesh);
        let h = hamming_distance(&labels, &gt);
        println!(
            "[golden:eval] curvature(sdf) vs topological-GT: Rand={ri:.3} Boundary(P={p:.3},R={r:.3},F1={f:.3}) Hamming={h:.3}"
        );
    }
}
