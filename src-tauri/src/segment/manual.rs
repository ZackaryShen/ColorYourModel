use std::cmp::Ordering;
use std::collections::{BinaryHeap, HashMap, HashSet};

use petgraph::visit::EdgeRef;

use crate::mesh::history::OpKind;
use crate::mesh::kdtree::distance;
use crate::mesh::model::MeshModel;

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

/// Snap a clicked point to the nearest mesh vertex, but constrained to the
/// SAME SIDE of the shell as the hit face.
///
/// Three-tier candidate restriction (iteration 12):
///   Tier 1 — hit-face + 1-ring same-side vertices within `k * max_edge`
///            of the hit face (auto-adapts to local mesh density).
///   Tier 2 — hit-face-only same-side vertices (no distance limit; still
///            local because candidates are only the 3 face corners).
///   Tier 3 — hit-face-only ANY vertex (ignores same-side; degenerate
///            fallback for flat/inverted faces).
///
/// NEVER falls back to global nearest vertex — that was the root cause of
/// "snap jumps to distant geometry on small features" (the 1-ring or global
/// set includes vertices from neighboring large faces far from the click).
pub fn snap_point_to_vertex_on_face(
    mesh: &MeshModel,
    point: &[f32; 3],
    face_id: u32,
) -> Option<(u32, [f32; 3])> {
    let fi = face_id as usize;
    if fi >= mesh.faces.len() || fi >= mesh.normals.len() {
        return mesh
            .nearest_vertex(point)
            .map(|(vi, _)| (vi, mesh.vertices[vi as usize]));
    }
    let face_normal = normalize(mesh.normals[fi]);
    let hit_face = mesh.faces[fi];

    // Local snap radius: k × longest edge of the hit face.
    // Adapts automatically from mm-scale prints to 200 mm parts and to
    // local mesh density.  k=1.5 ensures the hit face's own vertices are
    // always within threshold (triangle inequality), so normal clicks always
    // snap while distant 1-ring geometry is excluded.
    let k = 1.5_f32;
    let max_snap_sq = sq_dist(&mesh.vertices[hit_face[0] as usize], &mesh.vertices[hit_face[1] as usize])
        .max(sq_dist(&mesh.vertices[hit_face[1] as usize], &mesh.vertices[hit_face[2] as usize]))
        .max(sq_dist(&mesh.vertices[hit_face[2] as usize], &mesh.vertices[hit_face[0] as usize]))
        * k * k;

    // Same-side normal threshold: 60° (was 50° in iteration <26).
    // arccos(1/√3) ≈ 54.74° is the vertex-normal / face-normal angle at
    // a cube corner (3 orthogonal faces).  50° rejected these legitimate
    // ridge/feature vertices, causing "yellow dot" (unpickable) near sharp
    // edges.  60° covers the cube-corner case with ~5° margin while still
    // excluding >60° extreme folds that would snap across thin walls.
    let cos_thresh = (60.0f32).to_radians().cos();
    let p = *point;

    // ── Build candidate sets ──────────────────────────────────────
    // hit-face verts (always present)
    let mut seen: HashSet<u32> = HashSet::new();
    let mut face_only: Vec<u32> = Vec::new();
    for &v in &hit_face {
        if seen.insert(v) {
            face_only.push(v);
        }
    }
    // 1-ring extension (faces sharing any hit-face vertex)
    let mut one_ring: Vec<u32> = face_only.clone();
    for &v in &hit_face {
        for (fi2, face) in mesh.faces.iter().enumerate() {
            if (fi2 as u32) == face_id { continue; }
            if face[0] == v || face[1] == v || face[2] == v {
                for &w in face {
                    if seen.insert(w) {
                        one_ring.push(w);
                    }
                }
            }
        }
    }

    // ── Helper: pick nearest same-side vertex from a candidate list ─
    let pick_nearest_same_side = |candidates: &[u32]| -> Option<(u32, f32)> {
        let mut best: Option<u32> = None;
        let mut best_sq = f32::INFINITY;
        for &v in candidates {
            let vn = vertex_normal(mesh, v);
            if dot3(vn, face_normal) < cos_thresh { continue; }
            let vp = mesh.vertices[v as usize];
            let d = sq_dist(&p, &vp);
            if d < best_sq { best_sq = d; best = Some(v); }
        }
        best.map(|v| (v, best_sq))
    };

    // ── Helper: pick nearest ANY vertex from a candidate list (no normal check) ──
    let pick_nearest_any = |candidates: &[u32]| -> Option<(u32, f32)> {
        let mut best: Option<u32> = None;
        let mut best_sq = f32::INFINITY;
        for &v in candidates {
            let vp = mesh.vertices[v as usize];
            let d = sq_dist(&p, &vp);
            if d < best_sq { best_sq = d; best = Some(v); }
        }
        best.map(|v| (v, best_sq))
    };

    // ── Tier 1: 1-ring same-side within local threshold ───────────
    if let Some((vi, sq)) = pick_nearest_same_side(&one_ring) {
        if sq <= max_snap_sq {
            return Some((vi, mesh.vertices[vi as usize]));
        }
    }

    // ── Tier 2: hit-face-only same-side ──────────────────────────
    if let Some((vi, _sq)) = pick_nearest_same_side(&face_only) {
        return Some((vi, mesh.vertices[vi as usize]));
    }

    // ── Tier 3: hit-face-only any vertex (degenerate fallback) ────
    if let Some((vi, _sq)) = pick_nearest_any(&face_only) {
        return Some((vi, mesh.vertices[vi as usize]));
    }

    // Absolute last resort: global nearest (only when face data is bad).
    mesh.nearest_vertex(point)
        .map(|(vi, _)| (vi, mesh.vertices[vi as usize]))
}

fn normalize(n: [f32; 3]) -> [f32; 3] {
    let len = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
    if len < 1e-10 {
        [0.0, 0.0, 1.0]
    } else {
        [n[0] / len, n[1] / len, n[2] / len]
    }
}

fn dot3(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn sq_dist(a: &[f32; 3], b: &[f32; 3]) -> f32 {
    let dx = a[0] - b[0];
    let dy = a[1] - b[1];
    let dz = a[2] - b[2];
    dx * dx + dy * dy + dz * dz
}

/// Averaged normal of all faces incident to a vertex (sum, then normalized).
/// Used for the same-side test in `snap_point_to_vertex_on_face`.
fn vertex_normal(mesh: &MeshModel, v: u32) -> [f32; 3] {
    let mut n = [0.0f32; 3];
    let mut count = 0u32;
    for (fi, face) in mesh.faces.iter().enumerate() {
        if face[0] == v || face[1] == v || face[2] == v {
            if fi < mesh.normals.len() {
                let fnormal = mesh.normals[fi];
                n[0] += fnormal[0];
                n[1] += fnormal[1];
                n[2] += fnormal[2];
                count += 1;
            }
        }
    }
    if count == 0 {
        return [0.0, 0.0, 1.0];
    }
    normalize(n)
}

/// Build the vertex-edge adjacency graph ONCE (reused across all loop edges).
/// When `front_faces` is `Some(set)`, only edges belonging to a face in `set`
/// are added — this confines the densified loop to the front shell so Dijkstra
/// can no longer "tunnel" through a thin/closed shell to the back face (the
/// root cause of the "water-ripple overflow" where the selected region spilled
/// outside the drawn loop). Faces whose normal is only mildly off the loop's
/// average normal stay connected; only the opposite shell is excluded.
fn build_vertex_adjacency(
    mesh: &MeshModel,
    front_faces: Option<&HashSet<u32>>,
) -> HashMap<u32, Vec<u32>> {
    let mut vadj: HashMap<u32, Vec<u32>> = HashMap::new();
    for (fi, face) in mesh.faces.iter().enumerate() {
        if let Some(ff) = front_faces {
            if !ff.contains(&(fi as u32)) {
                continue;
            }
        }
        let edges = [(face[0], face[1]), (face[1], face[2]), (face[0], face[2])];
        for (a, b) in edges {
            vadj.entry(a).or_default().push(b);
            vadj.entry(b).or_default().push(a);
        }
    }
    vadj
}

/// Shortest path between two vertices over a PRE-BUILT vertex-edge graph
/// (Dijkstra, edge weight = Euclidean distance). Used to densify the lasso
/// loop so the boundary follows the surface between clicked points. The graph
/// is built once by the caller (`region_from_loop`) instead of per edge, which
/// removes the O(F) per-edge rebuild that made finalize slow on large meshes.
fn shortest_vertex_path_with_adj(
    mesh: &MeshModel,
    vadj: &HashMap<u32, Vec<u32>>,
    from: u32,
    to: u32,
) -> Vec<u32> {
    if from == to {
        return vec![from];
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

/// Number of smoothing passes applied to a freshly enclosed lasso region.
/// One pass trims single-face spikes and fills single-face notches; raise only
/// if denser meshes still show stair-stepping (cost grows linearly with passes).
const SMOOTH_PASSES: u32 = 1;

/// Smooth a lasso region's boundary to remove the thin-face "stair-step"
/// aliasing produced by the edge-barrier BFS (iteration 21, Issue 3 —
/// option B, chosen by the user).
///
/// The barrier BFS can only cut along mesh edges, so on a curved or low-density
/// surface a hand-drawn straight loop becomes a jagged zig-zag of triangles.
/// A balanced morphological filter fixes this without materially shrinking the
/// patch:
///
///   * ERODE (trim spikes): drop an in-region face that is strongly surrounded
///     by OUT faces (`out > 2 * in`). A 1-wide protrusion has every face with
///     `in == 1, out == 2` → `2 > 2*1` is false, so a single pass only peels the
///     genuine minority tips; a straight edge (`in == 2, out == 1`) is untouched.
///   * DILATE (fill notches): add an OUT face strongly surrounded by IN faces
///     (`in > 2 * out`), symmetrically healing thin concavities.
///
/// Both rules keep the area roughly constant, so the filled partition matches
/// what the user drew while reading as a smooth boundary. A region whose faces
/// are ALL isolated (`in == 0`) is preserved verbatim — we never delete a region
/// the loop actually enclosed.
fn smooth_region_boundary(mesh: &MeshModel, region: &[u32]) -> Vec<u32> {
    if region.is_empty() {
        return Vec::new();
    }
    let mut current: HashSet<u32> = region.iter().copied().collect();
    for _ in 0..SMOOTH_PASSES.max(1) {
        // Candidates = the region plus its face-neighbours; only these can
        // change (trim a spike or fill a notch), so evaluation stays O(boundary).
        let mut candidates: HashSet<u32> = current.clone();
        for &f in &current {
            let node = petgraph::graph::NodeIndex::new(f as usize);
            for edge in mesh.face_adjacency.edges(node) {
                let nb = if edge.source() == node {
                    edge.target()
                } else {
                    edge.source()
                };
                candidates.insert(nb.index() as u32);
            }
        }

        let mut next: HashSet<u32> = current.clone();
        for &f in &candidates {
            let node = petgraph::graph::NodeIndex::new(f as usize);
            let mut in_c = 0u32;
            let mut out_c = 0u32;
            for edge in mesh.face_adjacency.edges(node) {
                let nb = if edge.source() == node {
                    edge.target()
                } else {
                    edge.source()
                };
                if current.contains(&(nb.index() as u32)) {
                    in_c += 1;
                } else {
                    out_c += 1;
                }
            }
            if current.contains(&f) {
                // In-region face: trim only if it is a strong minority protrusion.
                // `in_c > 0` guard keeps a fully-isolated region intact.
                if in_c > 0 && out_c > 2 * in_c {
                    next.remove(&f);
                }
            } else if in_c > 2 * out_c {
                // Out-of-region face: fill only if it is a thin notch.
                next.insert(f);
            }
        }
        current = next;
    }
    // Safety net: if smoothing emptied the set (should not happen), return input.
    if current.is_empty() {
        return region.to_vec();
    }
    let mut out: Vec<u32> = current.into_iter().collect();
    out.sort_unstable();
    out
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
pub fn region_from_loop(
    mesh: &MeshModel,
    points: &[[f32; 3]],
    face_indices: &[u32],
) -> Vec<u32> {
    if points.len() < 3 {
        return Vec::new();
    }
    // 1: snap to vertices (same-side constrained when a hit face is known)
    let mut verts: Vec<u32> = Vec::with_capacity(points.len());
    for (i, p) in points.iter().enumerate() {
        let fi = face_indices.get(i).copied().unwrap_or(u32::MAX);
        let vi = if fi != u32::MAX {
            match snap_point_to_vertex_on_face(mesh, p, fi) {
                Some((vi, _)) => vi,
                None => continue,
            }
        } else {
            match snap_point_to_vertex(mesh, p) {
                Some((vi, _)) => vi,
                None => continue,
            }
        };
        verts.push(vi);
    }
    if verts.len() < 3 {
        return Vec::new();
    }

    // 1b: derive the loop's average front normal from the hit faces the user
    // actually clicked (they are guaranteed front-side). Confine densification
    // to faces whose normal is NOT strongly opposite this average — i.e. drop
    // only the opposite shell of a thin/closed mesh, while keeping curved front
    // faces connected. This is what stops Dijkstra from tunneling to the back
    // and producing an oversized ("overflowing") region.
    let front_faces: Option<HashSet<u32>> = {
        let mut avg = [0.0f32; 3];
        let mut count = 0u32;
        for &fi in face_indices {
            if (fi as usize) < mesh.normals.len() {
                let n = mesh.normals[fi as usize];
                avg[0] += n[0];
                avg[1] += n[1];
                avg[2] += n[2];
                count += 1;
            }
        }
        if count == 0 {
            None
        } else {
            let len = (avg[0] * avg[0] + avg[1] * avg[1] + avg[2] * avg[2]).sqrt();
            if len < 1e-10 {
                None
            } else {
                let an = [avg[0] / len, avg[1] / len, avg[2] / len];
                // Exclude only faces whose normal points the opposite way
                // (dot < -cos 50° ≈ -0.64): the back shell of a thin mesh.
                let cos_thresh = -(50.0f32).to_radians().cos();
                let mut set = HashSet::new();
                for (fi, n) in mesh.normals.iter().enumerate() {
                    if dot3(*n, an) > cos_thresh {
                        set.insert(fi as u32);
                    }
                }
                Some(set)
            }
        }
    };

    // Build vertex adjacency ONCE for the whole loop (cheap relative to the old
    // per-edge rebuild), then densify each loop edge over that single graph.
    let vadj = build_vertex_adjacency(mesh, front_faces.as_ref());
    let mut loop_edges: HashSet<(u32, u32)> = HashSet::new();
    let n = verts.len();
    for i in 0..n {
        let a = verts[i];
        let b = verts[(i + 1) % n];
        if a == b {
            continue;
        }
        let path = shortest_vertex_path_with_adj(mesh, &vadj, a, b);
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
    let raw = if comp_b.len() < comp_a.len() {
        comp_b
    } else {
        comp_a
    };
    smooth_region_boundary(mesh, &raw)
}

/// Finalize a manual region from an ordered list of clicked 3D points.
///
/// Snaps points to vertices (exact closure), builds the enclosed region via
/// geodesic loop + barrier BFS, assigns a fresh manual label + color, and
/// rebuilds segment metadata. Returns (label, region_face_ids).
pub fn finalize_manual_region(
    mesh: &mut MeshModel,
    points: &[[f32; 3]],
    face_indices: &[u32],
) -> Result<(u32, Vec<u32>), String> {
    if points.len() < 3 {
        return Err(format!(
            "At least 3 points are required to close a region (got {})",
            points.len()
        ));
    }
    let region = region_from_loop(mesh, points, face_indices);
    if region.is_empty() {
        return Err("Enclosed region is empty (degenerate loop)".into());
    }

    // Never recycles a number, even if an earlier region was undone or fully
    // painted over — see MeshModel::alloc_manual_label.
    let label = mesh.alloc_manual_label();
    let color = MeshModel::manual_label_color(label);
    // Record pre-finalize state for undo (faces may belong to an earlier region).
    // Colours and labels go into one entry so undo can never restore them out of
    // step, and because the prior state may itself be an earlier manual region
    // the LIFO order keeps overlapping lassos consistent.
    let mut prev_colors = Vec::with_capacity(region.len());
    let mut prev_labels = Vec::with_capacity(region.len());
    for &f in &region {
        let fi = f as usize;
        prev_colors.push((f, mesh.face_colors[fi]));
        prev_labels.push((f, mesh.segment_labels[fi]));
    }
    // A lasso is a single confirmed gesture, never a coalescing drag: `None`.
    mesh.history
        .record(OpKind::ManualRegion, None, &prev_colors, &prev_labels);
    for &f in &region {
        mesh.segment_labels[f as usize] = label;
        mesh.face_colors[f as usize] = color;
    }
    // Rebuild segment metadata so the new manual region is queryable by the
    // frontend (SegmentResult.segments is read from `mesh.segments`). Without
    // this, manual regions are written to faces but never appear as selectable
    // regions — mirroring finalize_segment.
    mesh.rebuild_segments();

    log::info!(
        "[manual] region finalized: label={}, faces={}",
        label,
        region.len()
    );
    Ok((label, region))
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
        let (label, region) = finalize_manual_region(&mut m, &pts, &[]).unwrap();
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
        let r = finalize_manual_region(&mut m, &[[0.0, 0.0, 1.0], [1.0, 0.0, 1.0]], &[]);
        assert!(r.is_err());
    }

    #[test]
    fn finalize_populates_segment_metadata() {
        let mut m = unit_cube();
        let pts = [
            [0.0, 0.0, 1.0],
            [1.0, 0.0, 1.0],
            [1.0, 1.0, 1.0],
            [0.0, 1.0, 1.0],
        ];
        let (label, region) = finalize_manual_region(&mut m, &pts, &[]).unwrap();
        // The new manual region must be queryable as segment metadata; otherwise
        // the frontend receives an empty `segments` list (regression guard).
        let seg = m
            .segments
            .get(&label)
            .expect("new manual region must be in segment metadata");
        assert_eq!(seg.face_count, region.len() as u32);
    }

    /// Build an OPEN triangulated plane grid (single-sided, no thickness) —
    /// the shape most real STL "surface" models resemble. rows x cols cells.
    fn open_plane(rows: u32, cols: u32) -> MeshModel {
        let mut v: Vec<[f32; 3]> = Vec::new();
        for j in 0..=rows {
            for i in 0..=cols {
                // Tiny z variation keeps the kd-tree (kiddo) non-degenerate;
                // still essentially planar so adjacency logic is exercised.
                let z = 0.001f32 * (i as f32) + 0.0007f32 * (j as f32);
                v.push([i as f32, j as f32, z]);
            }
        }
        let idx = |i: u32, j: u32| j * (cols + 1) + i;
        let mut faces: Vec<[u32; 3]> = Vec::new();
        for j in 0..rows {
            for i in 0..cols {
                let a = idx(i, j);
                let b = idx(i + 1, j);
                let c = idx(i + 1, j + 1);
                let d = idx(i, j + 1);
                faces.push([a, b, c]);
                faces.push([a, c, d]);
            }
        }
        let mut m = MeshModel::new();
        m.vertices = v;
        m.faces = faces;
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
    fn manual_region_on_open_mesh() {
        // 10x10 grid → 200 faces. Select the interior 2x2 cell patch around
        // grid vertices (4,4)-(6,6): that patch is 2x2 cells = 8 triangles.
        let mut m = open_plane(10, 10);
        // Click the four corners of an interior square (in grid coordinates).
        let pts = [
            [4.0, 4.0, 0.0],
            [6.0, 4.0, 0.0],
            [6.0, 6.0, 0.0],
            [4.0, 6.0, 0.0],
        ];
        let (label, region) = finalize_manual_region(&mut m, &pts, &[]).unwrap();
        assert!(label >= MANUAL_SEGMENT_OFFSET);
        // The enclosed patch must be a small, positive number of faces.
        assert!(
            !region.is_empty(),
            "region_from_loop returned an EMPTY region on an open mesh — this is the bug"
        );
        // Interior 2x2 cells = 8 triangles; allow the algorithm some slack but
        // it must not return the whole mesh (200) nor nearly all of it.
        assert!(
            region.len() < 200 / 2,
            "region covers most of the mesh ({} faces); loop did not enclose a patch",
            region.len()
        );
        let seg = m
            .segments
            .get(&label)
            .expect("manual region must appear in segment metadata on open mesh");
        assert_eq!(seg.face_count as usize, region.len());
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

        // First finalize: faces go from default (label 0 / default color) to L1.
        let (l1, region1) = finalize_manual_region(&mut m, &pts, &[]).unwrap();
        assert!(l1 >= MANUAL_SEGMENT_OFFSET);
        let color_l1 = m.face_colors[region1[0] as usize];
        for &f in &region1 {
            assert_eq!(m.segment_labels[f as usize], l1);
        }

        // Second finalize over the SAME faces → L2; prev state was L1.
        let (l2, _region2) = finalize_manual_region(&mut m, &pts, &[]).unwrap();
        assert_ne!(l1, l2);
        for &f in &region1 {
            assert_eq!(m.segment_labels[f as usize], l2);
        }

        // Undo once → back to L1 (layered restore), colors match L1 exactly.
        assert!(undo_once(&mut m));
        for &f in &region1 {
            assert_eq!(m.segment_labels[f as usize], l1);
            assert_eq!(m.face_colors[f as usize], color_l1);
        }
        assert!(m.segments.get(&l2).is_none());
        assert!(m.segments.get(&l1).is_some());

        // Undo again → back to default (label 0, DEFAULT_FACE_COLOR).
        assert!(undo_once(&mut m));
        for &f in &region1 {
            assert_eq!(m.segment_labels[f as usize], 0);
            assert_eq!(
                m.face_colors[f as usize],
                crate::mesh::model::DEFAULT_FACE_COLOR
            );
        }

        // History drained; no manual segment remains.
        assert!(!m.history.can_undo());
        assert!(m.segments.get(&l1).is_none());
        assert!(m.segments.get(&l2).is_none());
    }

    /// Undo one entry through the unified history and refresh segment metadata,
    /// mirroring what `commands::history::step` does for the real UI.
    fn undo_once(m: &mut MeshModel) -> bool {
        let MeshModel {
            history,
            face_colors,
            segment_labels,
            ..
        } = &mut *m;
        let applied = history.undo(face_colors, segment_labels).is_some();
        if applied {
            m.rebuild_segments();
        }
        applied
    }

    /// Undoing a lasso through `mesh.history` has to restore labels *and*
    /// colours — the two are stored in one entry precisely so they cannot come
    /// back out of step.
    #[test]
    fn finalize_records_into_the_unified_history() {
        let mut m = unit_cube();
        let pts = [
            [0.0, 0.0, 1.0],
            [1.0, 0.0, 1.0],
            [1.0, 1.0, 1.0],
            [0.0, 1.0, 1.0],
        ];
        let before: Vec<[u8; 4]> = m.face_colors.clone();

        let (label, region) = finalize_manual_region(&mut m, &pts, &[]).unwrap();
        assert!(m.history.can_undo(), "lasso must be undoable via mesh.history");

        let crate::mesh::model::MeshModel {
            history,
            face_colors,
            segment_labels,
            ..
        } = &mut m;
        let out = history.undo(face_colors, segment_labels).unwrap();
        assert!(out.labels_changed);
        assert_eq!(out.faces.len(), region.len());

        for &f in &region {
            assert_eq!(m.segment_labels[f as usize], 0, "label reverted");
            assert_eq!(m.face_colors[f as usize], before[f as usize], "colour reverted");
        }
        assert_ne!(label, 0);
        assert!(m.history.can_redo());
    }

    #[test]
    fn smooth_preserves_solid_block_and_never_deletes() {
        let m = open_plane(10, 10); // 200 faces, quad grid → dense adjacency
        // A solid interior block of 30 contiguous faces. Smoothing must keep the
        // bulk of it (only thin tips, if any, are trimmed) and never return an
        // empty region — guards iteration 21 Issue 3 "smoothing deleted my region".
        let block: Vec<u32> = (0..30u32).collect();
        let smoothed = smooth_region_boundary(&m, &block);
        assert!(
            !smoothed.is_empty(),
            "smoothing returned EMPTY for a non-empty solid block"
        );
        assert!(
            smoothed.len() >= 24,
            "smoothing trimmed too much of a solid block: {} faces (expected >=24)",
            smoothed.len()
        );
        // Idempotency-ish: re-smoothing a stable block must not shrink it further
        // by more than a couple of faces.
        let smoothed2 = smooth_region_boundary(&m, &smoothed);
        assert!(
            smoothed2.len() >= smoothed.len().saturating_sub(3),
            "re-smoothing shrank an already-smooth block unexpectedly: {} -> {}",
            smoothed.len(),
            smoothed2.len()
        );
    }

    /// Undo followed by redo must land back on the exact post-lasso state,
    /// including the segment metadata that the frontend renders from. The
    /// history entry is self-inverse, so the only thing that can drift is the
    /// derived `segments` map — hence the explicit check that the region
    /// reappears with the same face count.
    #[test]
    fn lasso_undo_then_redo_round_trips() {
        let mut m = unit_cube();
        let pts = [
            [0.0, 0.0, 1.0],
            [1.0, 0.0, 1.0],
            [1.0, 1.0, 1.0],
            [0.0, 1.0, 1.0],
        ];
        let (label, region) = finalize_manual_region(&mut m, &pts, &[]).unwrap();
        let after_colors = m.face_colors.clone();
        let after_labels = m.segment_labels.clone();

        assert!(undo_once(&mut m));
        assert!(m.segments.get(&label).is_none(), "region gone after undo");

        let MeshModel {
            history,
            face_colors,
            segment_labels,
            ..
        } = &mut m;
        history.redo(face_colors, segment_labels).unwrap();
        m.rebuild_segments();

        assert_eq!(m.face_colors, after_colors, "colours restored by redo");
        assert_eq!(m.segment_labels, after_labels, "labels restored by redo");
        let seg = m
            .segments
            .get(&label)
            .expect("region must reappear in segment metadata after redo");
        assert_eq!(seg.face_count as usize, region.len());
    }
}
