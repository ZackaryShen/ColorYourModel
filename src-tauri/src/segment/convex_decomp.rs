use parry3d::math::Vector;
use parry3d::transformation::vhacd::{VHACD, VHACDParameters};
use petgraph::visit::EdgeRef;

use crate::mesh::loader::ProgressFn;
use crate::mesh::model::{MeshModel, Segment};
use crate::segment::postprocess::min_region_faces;

/// Convex-decomposition default / clamp bounds (UI mirrors these).
const DEFAULT_MAX_HULLS: u32 = 32;
const MAX_HULLS_CAP: u32 = 64;
const MIN_RESOLUTION: u32 = 64;
const DEFAULT_RESOLUTION: u32 = 128;

/// Intermediate convex-decomposition result, shared by the convex-decomposition
/// and curve-skeleton algorithms so V-HACD runs only once.
pub struct ConvexParts {
    /// Compact part id (0..P) for every mesh face.
    pub face_part: Vec<u32>,
    /// Centroid of each part (compact ids), in mesh coordinates.
    pub part_centroids: Vec<[f32; 3]>,
    /// Undirected adjacency between parts that share a mesh boundary edge.
    pub adjacency: Vec<(u32, u32)>,
}

#[inline]
fn tri_centroid(verts: &[[f32; 3]], tri: &[u32; 3]) -> [f32; 3] {
    let a = verts[tri[0] as usize];
    let b = verts[tri[1] as usize];
    let c = verts[tri[2] as usize];
    [
        (a[0] + b[0] + c[0]) / 3.0,
        (a[1] + b[1] + c[1]) / 3.0,
        (a[2] + b[2] + c[2]) / 3.0,
    ]
}

/// Run V-HACD and assign every triangle to the nearest convex-hull centroid.
///
/// The nearest-centroid rule is unambiguous — a triangle inside block A is always
/// closer to A's centroid than to B's — so boundaries land on the correct side
/// without a fragile point-in-hull test. V-HACD cuts at the model's narrow joints
/// (neck / waist / wrist / ankle), which is precisely where the concavity field
/// went silent on armoured characters (convex ridges carry no concavity signal).
/// A block-level partition is exactly what a character needs: head, torso, each
/// limb segment, hands come back as separate parts.
pub fn compute_convex_parts(
    mesh: &MeshModel,
    max_hulls: u32,
    concavity: f32,
    resolution: u32,
) -> ConvexParts {
    let n = mesh.faces.len();
    let points: Vec<Vector> = mesh
        .vertices
        .iter()
        .map(|v| Vector::new(v[0], v[1], v[2]))
        .collect();
    let indices: Vec<[u32; 3]> = mesh.faces.clone();

    let max_hulls = if max_hulls == 0 {
        DEFAULT_MAX_HULLS
    } else {
        max_hulls.clamp(2, MAX_HULLS_CAP)
    };
    let mut params = VHACDParameters::default();
    params.max_convex_hulls = max_hulls;
    params.concavity = concavity.clamp(0.005, 0.2);
    params.resolution = resolution.max(MIN_RESOLUTION);

    // keep_voxel_to_primitives_map = false: voxel-based hulls are enough for the
    // nearest-centroid assignment and avoid the heavy exact-hull intersection map
    // (which would otherwise blow up memory on million-face STLs).
    let decomposition = VHACD::decompose(&params, &points, &indices, false);
    let hulls = decomposition.compute_convex_hulls(4);

    // Keep only hulls with real geometry; skip empty ones (their centroid would
    // otherwise pull triangles toward the origin). The kept index position is the
    // compact part id.
    let valid: Vec<usize> = (0..hulls.len())
        .filter(|&i| !hulls[i].0.is_empty())
        .collect();
    let part_centroids: Vec<[f32; 3]> = valid
        .iter()
        .map(|&i| {
            let hv = &hulls[i].0;
            let mut s = [0.0f32; 3];
            for p in hv {
                s[0] += p.x;
                s[1] += p.y;
                s[2] += p.z;
            }
            let k = hv.len() as f32;
            [s[0] / k, s[1] / k, s[2] / k]
        })
        .collect();

    // Assign each triangle to the nearest valid hull centroid.
    let mut face_part = vec![0u32; n];
    if part_centroids.is_empty() {
        // Degenerate: no hulls produced (empty mesh). Everything is one part.
        return ConvexParts {
            face_part,
            part_centroids: vec![[0.0; 3]],
            adjacency: vec![],
        };
    }
    for (fi, tri) in indices.iter().enumerate() {
        let c = tri_centroid(&mesh.vertices, tri);
        let mut best = 0usize;
        let mut best_d = f32::MAX;
        for (pos, pc) in part_centroids.iter().enumerate() {
            let dx = c[0] - pc[0];
            let dy = c[1] - pc[1];
            let dz = c[2] - pc[2];
            let d = dx * dx + dy * dy + dz * dz;
            if d < best_d {
                best_d = d;
                best = pos;
            }
        }
        face_part[fi] = best as u32;
    }

    // Build part adjacency from shared mesh boundary edges.
    let mut adjacency: Vec<(u32, u32)> = Vec::new();
    for edge in mesh.face_adjacency.edge_references() {
        let fi = mesh.face_adjacency[edge.source()];
        let fj = mesh.face_adjacency[edge.target()];
        let li = face_part[fi as usize];
        let lj = face_part[fj as usize];
        if li != lj {
            adjacency.push((li, lj));
        }
    }

    ConvexParts {
        face_part,
        part_centroids,
        adjacency,
    }
}

/// Group convex parts into limbs along the skeleton graph.
///
/// The curve skeleton of the decomposition is its adjacency graph (nodes = part
/// centroids, edges = shared boundaries). We treat the *chain* between two joints
/// as one limb: remove branch nodes (degree ≥ 3), label each remaining connected
/// component (a path or ring) as one limb, then attach every joint to its nearest
/// incident limb. The result is coarser and more semantic than the raw block
/// partition — head / torso / each limb comes back as one region.
fn limb_grouping(adjacency: &[(u32, u32)], num_parts: usize) -> Vec<u32> {
    let mut nbr: Vec<Vec<u32>> = vec![Vec::new(); num_parts];
    for &(a, b) in adjacency {
        if a != b && (a as usize) < num_parts && (b as usize) < num_parts {
            nbr[a as usize].push(b);
            nbr[b as usize].push(a);
        }
    }
    let mut limb = vec![u32::MAX; num_parts];
    let mut next_limb = 0u32;

    // Branch nodes are joints; each becomes its own seed we will attach to a limb.
    let is_branch = |i: usize| nbr[i].len() >= 3;

    // First label every non-branch connected component as one limb.
    for start in 0..num_parts {
        if limb[start] != u32::MAX || is_branch(start) {
            continue;
        }
        // BFS over non-branch nodes only.
        let mut stack = vec![start as u32];
        limb[start] = next_limb;
        while let Some(node) = stack.pop() {
            for &nb in &nbr[node as usize] {
                if !is_branch(nb as usize) && limb[nb as usize] == u32::MAX {
                    limb[nb as usize] = next_limb;
                    stack.push(nb);
                }
            }
        }
        next_limb += 1;
    }

    // Any unlabeled node is a branch (joint). Attach it to the nearest incident
    // limb — here, the limb with the most shared boundary edges wins, which is
    // the most-connected limb and reads as the natural "owner" of the joint.
    for i in 0..num_parts {
        if limb[i] != u32::MAX {
            continue;
        }
        // Largest adjacent limb wins the joint.
        let mut best_limb = u32::MAX;
        let mut best_size = 0usize;
        let mut count: std::collections::HashMap<u32, usize> = std::collections::HashMap::new();
        for &nb in &nbr[i] {
            if let Some(l) = limb[nb as usize].try_into().ok().filter(|&l| l != u32::MAX) {
                let c = count.entry(l).or_insert(0);
                *c += 1;
            }
        }
        for (l, c) in count {
            if c > best_size {
                best_size = c;
                best_limb = l;
            }
        }
        limb[i] = if best_limb != u32::MAX { best_limb } else { next_limb };
        if limb[i] == next_limb {
            next_limb += 1;
        }
    }

    limb
}

/// Compact per-face labels to 0..K, then merge regions smaller than the
/// model-relative floor into their strongest adjacent neighbour. Shared by both
/// algorithms so the "too small to be a part" rule cannot drift.
fn finalize_segments(mesh: &mut MeshModel, raw_labels: Vec<u32>, on_progress: &ProgressFn) -> Vec<Segment> {
    let n = raw_labels.len();
    on_progress(0.85, "finalize:merge");

    // Compact labels to 0..K.
    let mut remap: std::collections::HashMap<u32, u32> = std::collections::HashMap::new();
    let mut next = 0u32;
    let mut labels = vec![0u32; n];
    for (i, &l) in raw_labels.iter().enumerate() {
        let id = *remap.entry(l).or_insert_with(|| {
            let id = next;
            next += 1;
            id
        });
        labels[i] = id;
    }
    let k = next as usize;

    // Region face counts + adjacency.
    let mut face_count = vec![0u32; k];
    for &l in &labels {
        face_count[l as usize] += 1;
    }
    let mut region_adj: Vec<std::collections::HashMap<u32, u32>> = vec![std::collections::HashMap::new(); k];
    for edge in mesh.face_adjacency.edge_references() {
        let fi = mesh.face_adjacency[edge.source()];
        let fj = mesh.face_adjacency[edge.target()];
        let li = labels[fi as usize];
        let lj = labels[fj as usize];
        if li != lj {
            *region_adj[li as usize].entry(lj).or_insert(0) += 1;
            *region_adj[lj as usize].entry(li).or_insert(0) += 1;
        }
    }

    let min_faces = min_region_faces(n);
    for _ in 0..10 {
        let small: Vec<u32> = (0..k)
            .filter(|&i| face_count[i] < min_faces)
            .map(|i| i as u32)
            .collect();
        if small.is_empty() {
            break;
        }
        let mut merged = false;
        for s in small {
            if face_count[s as usize] == 0 {
                continue;
            }
            let best = region_adj[s as usize]
                .iter()
                .filter(|(&nb, _)| face_count[nb as usize] >= min_faces)
                .max_by_key(|(_, &c)| c)
                .map(|(&nb, _)| nb);
            let Some(large) = best else { continue };
            // Merge s into large.
            for i in 0..n {
                if labels[i] == s {
                    labels[i] = large;
                }
            }
            face_count[large as usize] += face_count[s as usize];
            face_count[s as usize] = 0;
            // Update adjacency.
            let s_nbrs: Vec<u32> = region_adj[s as usize].keys().cloned().collect();
            for nb in s_nbrs {
                if let Some(c) = region_adj[s as usize].remove(&nb) {
                    region_adj[nb as usize].remove(&s);
                    if nb != large {
                        *region_adj[large as usize].entry(nb).or_insert(0) += c;
                        *region_adj[nb as usize].entry(large).or_insert(0) += c;
                    }
                }
            }
            region_adj[large as usize].remove(&large);
            merged = true;
        }
        if !merged {
            break;
        }
    }

    mesh.segment_labels = labels;
    mesh.rebuild_segments();
    on_progress(0.95, "finalize:done");
    mesh.sorted_segments()
}

/// Block-level convex decomposition (V-HACD). Each region is one approximately
/// convex part; boundaries sit at the model's narrow joints.
pub fn segment_by_convex_decomposition(
    mesh: &mut MeshModel,
    max_hulls: u32,
    concavity: f32,
    on_progress: &ProgressFn,
) -> Vec<Segment> {
    on_progress(0.05, "vhacd:decompose");
    let parts = compute_convex_parts(mesh, max_hulls, concavity, DEFAULT_RESOLUTION);
    on_progress(0.7, "vhacd:assign");
    finalize_segments(mesh, parts.face_part, on_progress)
}

/// Curve-skeleton segmentation: convex decomposition, then merge each chain
/// between joints into one limb-level region. Coarser and more semantic than the
/// raw block partition — head / torso / limbs instead of dozens of blocks.
pub fn segment_by_curve_skeleton(
    mesh: &mut MeshModel,
    max_hulls: u32,
    concavity: f32,
    on_progress: &ProgressFn,
) -> Vec<Segment> {
    on_progress(0.05, "skeleton:decompose");
    let parts = compute_convex_parts(mesh, max_hulls, concavity, DEFAULT_RESOLUTION);
    on_progress(0.7, "skeleton:limbs");

    let num_parts = parts.part_centroids.len();
    let limb = limb_grouping(&parts.adjacency, num_parts);

    // Map face -> limb.
    let mut face_limb = vec![0u32; parts.face_part.len()];
    for (fi, &p) in parts.face_part.iter().enumerate() {
        face_limb[fi] = limb.get(p as usize).copied().unwrap_or(0);
    }
    finalize_segments(mesh, face_limb, on_progress)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::segment::metrics::unit_cube;

    #[test]
    fn cube_is_one_convex_part() {
        let mesh = unit_cube();
        let parts = compute_convex_parts(&mesh, 0, 0.05, MIN_RESOLUTION);
        // A cube is already convex → V-HACD collapses to a single hull.
        let distinct: std::collections::HashSet<u32> = parts.face_part.iter().cloned().collect();
        assert!(
            distinct.len() <= 2,
            "convex cube should be ~1 part, got {}",
            distinct.len()
        );
    }

    #[test]
    fn limb_grouping_splits_a_chain_at_branch() {
        // Graph: 0-1-2-3-4 with 2 as a branch (2-5). Expect 5 nodes -> limbs.
        // 0-1 chain (limb A), 3-4 chain (limb B), 5 separate (limb C), 2 joint.
        let adj: Vec<(u32, u32)> = vec![
            (0, 1),
            (1, 2),
            (2, 3),
            (3, 4),
            (2, 5),
        ];
        let limb = limb_grouping(&adj, 6);
        // 0 and 1 share a limb; 3 and 4 share a (different) limb; 5 is its own.
        assert_eq!(limb[0], limb[1], "0-1 should be one limb");
        assert_eq!(limb[3], limb[4], "3-4 should be one limb");
        assert_ne!(limb[0], limb[3], "left and right chains are distinct limbs");
        // The branch node 2 attaches to one of its incident limbs.
        assert!(limb[2] != u32::MAX);
        assert_ne!(limb[5], u32::MAX);
    }
}
