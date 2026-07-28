use std::cmp::Ordering;
use std::collections::{BinaryHeap, HashMap, HashSet};

use petgraph::visit::EdgeRef;

use crate::mesh::kdtree::distance;
use crate::mesh::model::{MeshModel, ManualRegionSnapshot};

/// f32 wrapper implementing Ord (NaN treated as equal) so it can be used as a
/// BinaryHeap key for Dijkstra. Plain f32 is not Ord, which makes
/// `BinaryHeap<(Reverse<f32>, u32)>` fail to compile.
#[derive(Clone, Copy, Debug)]
struct F32Ord(f32);
impl PartialEq for F32Ord {
    fn eq(&self, other: &Self) -> bool {
        self.0 == other.0
    }
}
impl Eq for F32Ord {}
impl PartialOrd for F32Ord {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        self.0.partial_cmp(&other.0)
    }
}
impl Ord for F32Ord {
    fn cmp(&self, other: &Self) -> Ordering {
        self.0.partial_cmp(&other.0).unwrap_or(Ordering::Equal)
    }
}

/// Label offset separating manual segments from auto-segment labels.
/// Auto-segment uses labels 0..N; manual labels start at 100_000 so that
/// re-running auto_segment after manual work never collides.
/// Re-exported from `mesh::model` (the single definition).
pub use crate::mesh::model::MANUAL_SEGMENT_OFFSET;

/// Snap a clicked 3D point to the nearest mesh vertex.
/// Returns (vertex_index, snapped_position). This guarantees that when the user
/// clicks the start point again to close the loop, it maps to the exact same
/// vertex (closure is exact).
pub fn snap_point_to_vertex(mesh: &MeshModel, point: &[f32; 3]) -> Option<(u32, [f32; 3])> {
    let (vi, _sq) = mesh.nearest_vertex(point)?;
    let v = mesh.vertices[vi as usize];
    Some((vi, v))
}

/// Shortest path between two vertices over the mesh vertex-edge graph
/// (Dijkstra, edge weight = Euclidean distance). Used to densify the lasso
/// loop so the boundary follows the surface between clicked points.
fn shortest_vertex_path(mesh: &MeshModel, from: u32, to: u32) -> Vec<u32> {
    if from == to {
        return vec![from];
    }
    // Build vertex adjacency on the fly (region_from_loop is infrequent).
    let mut vadj: HashMap<u32, Vec<u32>> = HashMap::new();
    for face in &mesh.faces {
        let edges = [(face[0], face[1]), (face[1], face[2]), (face[0], face[2])];
        for (a, b) in edges {
            vadj.entry(a).or_default().push(b);
            vadj.entry(b).or_default().push(a);
        }
    }
    let nv = mesh.vertices.len();
    let mut dist = vec![f32::INFINITY; nv];
    let mut prev = vec![u32::MAX; nv];
    dist[from as usize] = 0.0;
    let mut heap: BinaryHeap<(std::cmp::Reverse<F32Ord>, u32)> = BinaryHeap::new();
    heap.push((std::cmp::Reverse(F32Ord(0.0)), from));
    while let Some((std::cmp::Reverse(d), cur)) = heap.pop() {
        if cur == to {
            break;
        }
        if d.0 > dist[cur as usize] {
            continue;
        }
        let cv = mesh.vertices[cur as usize];
        for &nb in vadj.get(&cur).unwrap_or(&Vec::new()) {
            let nw = mesh.vertices[nb as usize];
            let w = distance(&cv, &nw);
            let nd = d.0 + w;
            if nd < dist[nb as usize] {
                dist[nb as usize] = nd;
                prev[nb as usize] = cur;
                heap.push((std::cmp::Reverse(F32Ord(nd)), nb));
            }
        }
    }
    let mut path = Vec::new();
    let mut cur = to;
    loop {
        path.push(cur);
        if cur == from {
            break;
        }
        cur = prev[cur as usize];
        if cur == u32::MAX {
            break;
        }
    }
    path.reverse();
    path
}

/// The shared mesh edge (as an undirected (min,max) pair) between two faces,
/// if they share exactly two vertices.
fn shared_edge(mesh: &MeshModel, fi: u32, fj: u32) -> Option<(u32, u32)> {
    let f = mesh.faces[fi as usize];
    let g = mesh.faces[fj as usize];
    let common: Vec<u32> = f.iter().copied().filter(|x| g.contains(x)).collect();
    if common.len() >= 2 {
        Some((common[0].min(common[1]), common[0].max(common[1])))
    } else {
        None
    }
}

/// Compute the region enclosed by a closed lasso loop.
///
/// Algorithm (topology-correct; REFUTE P3/P4 resolved):
/// 1. Snap clicked points to vertices.
/// 2. Densify the loop over the vertex-edge graph (Dijkstra) → `loop_edges`,
///    the surface boundary the user actually drew.
/// 3. Remove every face-adjacency edge whose shared mesh edge is in
///    `loop_edges`; BFS the remaining face graph from a start face. The loop
///    splits the surface into components; return the SMALLER component (the
///    patch the user drew around). For a simple loop on a closed surface this
///    is exactly the intended region (e.g. clicking a cube's 4 top corners
///    isolates the top face, not the 4 side faces).
pub fn region_from_loop(mesh: &MeshModel, points: &[[f32; 3]]) -> Vec<u32> {
    if points.len() < 3 {
        return Vec::new();
    }
    // 1: snap to vertices
    let mut verts: Vec<u32> = Vec::with_capacity(points.len());
    for p in points {
        match snap_point_to_vertex(mesh, p) {
            Some((vi, _)) => verts.push(vi),
            None => continue,
        }
    }
    if verts.len() < 3 {
        return Vec::new();
    }

    // 2: densify loop edges over the vertex graph
    let mut loop_edges: HashSet<(u32, u32)> = HashSet::new();
    let n = verts.len();
    for i in 0..n {
        let a = verts[i];
        let b = verts[(i + 1) % n];
        if a == b {
            continue;
        }
        let path = shortest_vertex_path(mesh, a, b);
        for w in path.windows(2) {
            let e = (w[0].min(w[1]), w[0].max(w[1]));
            loop_edges.insert(e);
        }
    }
    if loop_edges.is_empty() {
        return Vec::new();
    }

    // 3: edge-barrier BFS over the face graph; return the smaller component.
    let total = mesh.faces.len();
    if total == 0 {
        return Vec::new();
    }
    let start = 0u32;
    let mut visited = vec![false; total];
    let mut comp_a: Vec<u32> = Vec::new();
    let mut queue = std::collections::VecDeque::new();
    visited[start as usize] = true;
    queue.push_back(start);
    while let Some(cur) = queue.pop_front() {
        comp_a.push(cur);
        let node = petgraph::graph::NodeIndex::new(cur as usize);
        for edge in mesh.face_adjacency.edges(node) {
            let nb = if edge.source() == node {
                edge.target()
            } else {
                edge.source()
            };
            let ni = nb.index() as u32;
            if visited[ni as usize] {
                continue;
            }
            // Skip crossing a loop edge (this is the barrier).
            if let Some(se) = shared_edge(mesh, cur, ni) {
                if loop_edges.contains(&se) {
                    continue;
                }
            }
            visited[ni as usize] = true;
            queue.push_back(ni);
        }
    }
    let comp_b: Vec<u32> = (0..total as u32).filter(|f| !visited[*f as usize]).collect();
    if comp_b.len() < comp_a.len() {
        comp_b
    } else {
        comp_a
    }
}

/// Finalize a manual region from an ordered list of clicked 3D points.
///
/// Snaps points to vertices (exact closure), builds the enclosed region via
/// geodesic loop + barrier BFS, assigns a fresh manual label + color, and
/// rebuilds segment metadata. Returns (label, region_face_ids).
pub fn finalize_manual_region(
    mesh: &mut MeshModel,
    points: &[[f32; 3]],
) -> Result<(u32, Vec<u32>), String> {
    if points.len() < 3 {
        return Err(format!(
            "At least 3 points are required to close a region (got {})",
            points.len()
        ));
    }
    let region = region_from_loop(mesh, points);
    if region.is_empty() {
        return Err("Enclosed region is empty (degenerate loop)".into());
    }

    let max_label = mesh
        .segment_labels
        .iter()
        .copied()
        .max()
        .unwrap_or(MANUAL_SEGMENT_OFFSET.wrapping_sub(1));
    let label = std::cmp::max(MANUAL_SEGMENT_OFFSET, max_label + 1);

    let color_seed = label.wrapping_mul(2654435761) >> 24;
    let color: [u8; 4] = [
        ((color_seed * 73) % 200 + 55) as u8,
        ((color_seed * 151) % 200 + 55) as u8,
        ((color_seed * 223) % 200 + 55) as u8,
        255,
    ];
    // Record pre-finalize state for undo (faces may belong to an earlier region).
    let mut snapshot = ManualRegionSnapshot {
        label,
        faces: Vec::with_capacity(region.len()),
    };
    for &f in &region {
        let fi = f as usize;
        snapshot.faces.push((f, mesh.segment_labels[fi], mesh.face_colors[fi]));
    }
    for &f in &region {
        mesh.segment_labels[f as usize] = label;
        mesh.face_colors[f as usize] = color;
    }
    mesh.manual_region_history.push(snapshot);

    log::info!(
        "[manual] region finalized: label={}, faces={}",
        label,
        region.len()
    );
    Ok((label, region))
}

/// Undo the most recently finalized manual (lasso) region.
///
/// Pops the LIFO `manual_region_history`, restores each affected face to its
/// recorded prior `(segment_label, face_color)`, then rebuilds segment
/// metadata from the restored labels. Returns `Some(label)` of the reverted
/// region, or `None` when history is empty.
pub fn undo_last_manual_region(mesh: &mut MeshModel) -> Option<u32> {
    let snapshot = mesh.manual_region_history.pop()?;
    for (face, prev_label, prev_color) in snapshot.faces {
        mesh.segment_labels[face as usize] = prev_label;
        mesh.face_colors[face as usize] = prev_color;
    }
    mesh.rebuild_segments();
    Some(snapshot.label)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mesh::model::MeshModel;

    /// Build a unit cube (8 verts, 12 tris), with consistent outward normals.
    fn unit_cube() -> MeshModel {
        let v = [
            [0.0, 0.0, 0.0],
            [1.0, 0.0, 0.0],
            [1.0, 1.0, 0.0],
            [0.0, 1.0, 0.0],
            [0.0, 0.0, 1.0],
            [1.0, 0.0, 1.0],
            [1.0, 1.0, 1.0],
            [0.0, 1.0, 1.0],
        ];
        // Faces (outward winding)
        let f: [u32; 36] = [
            0, 3, 2, 0, 2, 1, // bottom (y=0)
            4, 5, 6, 4, 6, 7, // top (y=1)
            0, 1, 5, 0, 5, 4, // front (z=0..1, x low)
            2, 3, 7, 2, 7, 6, // back (x high)
            1, 2, 6, 1, 6, 5, // right (y=0..1, x=1)
            3, 0, 4, 3, 4, 7, // left (x=0)
        ];
        let mut m = MeshModel::new();
        m.vertices = v.to_vec();
        m.faces = f.chunks(3).map(|c| [c[0], c[1], c[2]]).collect();
        m.compute_normals();
        m.compute_bbox();
        m.build_kdtree();
        m.build_vertex_kdtree();
        m.build_adjacency();
        m.init_default_colors();
        m.segment_labels = vec![0u32; m.faces.len()];
        m
    }

    #[test]
    fn snap_finds_nearest_vertex() {
        let m = unit_cube();
        // A point just off the top-front-right vertex (index 6) should snap to it
        let (vi, snapped) = snap_point_to_vertex(&m, &[1.0, 1.0, 1.05]).unwrap();
        assert_eq!(vi, 6);
        assert_eq!(snapped, [1.0, 1.0, 1.0]);
    }

    #[test]
    fn loop_around_top_selects_cap() {
        let mut m = unit_cube();
        // Click the 4 top corners in order → loop around the top face.
        // Top vertices: 4(0,0,1) 5(1,0,1) 6(1,1,1) 7(0,1,1)
        let pts = [
            [0.0, 0.0, 1.0],
            [1.0, 0.0, 1.0],
            [1.0, 1.0, 1.0],
            [0.0, 1.0, 1.0],
        ];
        let (label, region) = finalize_manual_region(&mut m, &pts).unwrap();
        assert!(label >= MANUAL_SEGMENT_OFFSET);
        // The top cap (z=1) is exactly 2 triangles → smaller side = 2
        assert_eq!(region.len(), 2, "top cap should be 2 faces");
        // All selected faces must have z≈1 (this cube defines up = z, not y)
        for &f in &region {
            let c = m.face_center(f);
            assert!((c[2] - 1.0).abs() < 1e-4, "selected face not on top cap");
        }
    }

    #[test]
    fn too_few_points_errors() {
        let mut m = unit_cube();
        let r = finalize_manual_region(&mut m, &[[0.0, 0.0, 1.0], [1.0, 0.0, 1.0]]);
        assert!(r.is_err());
    }

    #[test]
    fn undo_restores_previous_state() {
        let mut m = unit_cube();
        let pts = [
            [0.0, 0.0, 1.0],
            [1.0, 0.0, 1.0],
            [1.0, 1.0, 1.0],
            [0.0, 1.0, 1.0],
        ];

        // First finalize: faces go from default (label 0 / white) to L1.
        let (l1, region1) = finalize_manual_region(&mut m, &pts).unwrap();
        assert!(l1 >= MANUAL_SEGMENT_OFFSET);
        let color_l1 = m.face_colors[region1[0] as usize];
        for &f in &region1 {
            assert_eq!(m.segment_labels[f as usize], l1);
        }
        assert_eq!(m.manual_region_history.len(), 1);

        // Second finalize over the SAME faces → L2; prev state was L1.
        let (l2, _region2) = finalize_manual_region(&mut m, &pts).unwrap();
        assert_ne!(l1, l2);
        let color_l2 = m.face_colors[region1[0] as usize];
        for &f in &region1 {
            assert_eq!(m.segment_labels[f as usize], l2);
        }

        // Undo once → back to L1 (layered restore), colors match L1 exactly.
        let undone = undo_last_manual_region(&mut m).unwrap();
        assert_eq!(undone, l2);
        for &f in &region1 {
            assert_eq!(m.segment_labels[f as usize], l1);
            assert_eq!(m.face_colors[f as usize], color_l1);
        }
        assert!(m.segments.get(&l2).is_none());
        assert!(m.segments.get(&l1).is_some());

        // Undo again → back to default (label 0, white).
        let undone2 = undo_last_manual_region(&mut m).unwrap();
        assert_eq!(undone2, l1);
        for &f in &region1 {
            assert_eq!(m.segment_labels[f as usize], 0);
            assert_eq!(m.face_colors[f as usize], [255, 255, 255, 255]);
        }

        // History drained; no manual segment remains.
        assert!(m.manual_region_history.is_empty());
        assert!(m.segments.get(&l1).is_none());
        assert!(m.segments.get(&l2).is_none());
    }
}
