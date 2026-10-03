//! Seeded watershed segmentation — the "manual partition + smart optimization"
//! workflow (iteration 50).
//!
//! The user drops a handful of seed points (one per region they care about) and
//! the algorithm grows each seed into a region using:
//!   1. **feature-edge barriers** — dihedral angles above `barrier_deg` act as
//!      hard boundaries (eye-socket rims, mouth line, jaw, armour seams), so a
//!      region never bleeds across a sharp crease. This is what makes a face
//!      split along anatomical features instead of arbitrary stripes.
//!   2. **geodesic nearest-seed Voronoi** — every face is assigned to the seed
//!      it is closest to *along the surface* (Dijkstra over the barrier-free
//!      face graph). This is the "smart optimization" over the raw geometry.
//!   3. **fallback for unseeded components** — a connected patch the user did
//!      not click still gets a sensible region by assigning it to the
//!      geodesic-nearest seed *across* barriers (with a penalty so the route
//!      prefers going around a crease rather than through it). This is the
//!      "guaranteed fallback when there is a goal + constraint": the goal is K
//!      regions from K seeds, the constraint is the mesh geometry, and the
//!      fallback fills whatever the user skipped.
//!
//! Everything reuses existing building blocks: `shared_edge` (manual.rs),
//! `snap_point_to_vertex_on_face` (manual.rs), `alloc_manual_label`,
//! `manual_label_color`, the unified undo `OpKind::ManualRegion`.

use std::cmp::Ordering;
use std::collections::{BinaryHeap, HashMap, HashSet};

use petgraph::visit::EdgeRef;

use crate::mesh::history::OpKind;
use crate::mesh::model::{MeshModel, Segment};
use crate::segment::manual::{shared_edge, snap_point_to_vertex_on_face};

/// f32 wrapper implementing `Ord` so it can key a `BinaryHeap` for Dijkstra.
#[derive(Clone, Copy)]
struct Key(f32);
impl PartialEq for Key {
    fn eq(&self, o: &Self) -> bool {
        self.0 == o.0
    }
}
impl Eq for Key {}
impl PartialOrd for Key {
    fn partial_cmp(&self, o: &Self) -> Option<Ordering> {
        self.0.partial_cmp(&o.0)
    }
}
impl Ord for Key {
    fn cmp(&self, o: &Self) -> Ordering {
        self.0.partial_cmp(&o.0).unwrap_or(Ordering::Equal)
    }
}

#[derive(Clone)]
pub struct SeedGrowParams {
    /// Dihedral angle (degrees) above which a shared edge becomes a hard
    /// boundary between seeds. Higher → fewer barriers (more bleed); lower →
    /// regions hug creases tightly. Default 45.
    pub barrier_deg: f32,
    /// After growing, merge sub-0.2%-of-mesh regions into the neighbouring
    /// region they share the most boundary with. Cleans up tiny artefacts left
    /// by a stray seed; off by default for full manual control.
    pub optimizer: bool,
}

impl Default for SeedGrowParams {
    fn default() -> Self {
        Self {
            barrier_deg: 45.0,
            optimizer: false,
        }
    }
}

#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub struct SeedInput {
    /// The 3D click position (model-local), used to snap to a mesh vertex.
    pub point: [f32; 3],
    /// The face the raycaster hit; passed alongside `point` so snapping is exact.
    pub face_index: u32,
}

#[derive(Default)]
pub struct SeedGrowResult {
    pub segments: Vec<Segment>,
    pub segment_labels: Vec<u32>,
    pub moved_faces: usize,
    pub region_count: usize,
}

fn face_centroid(mesh: &MeshModel, f: u32) -> [f32; 3] {
    let tri = mesh.faces[f as usize];
    let a = mesh.vertices[tri[0] as usize];
    let b = mesh.vertices[tri[1] as usize];
    let c = mesh.vertices[tri[2] as usize];
    [
        (a[0] + b[0] + c[0]) / 3.0,
        (a[1] + b[1] + c[1]) / 3.0,
        (a[2] + b[2] + c[2]) / 3.0,
    ]
}

fn dist3(a: &[f32; 3], b: &[f32; 3]) -> f32 {
    let dx = a[0] - b[0];
    let dy = a[1] - b[1];
    let dz = a[2] - b[2];
    (dx * dx + dy * dy + dz * dz).sqrt()
}

/// Grow seeds into a segmentation. See module docs for the algorithm.
pub fn seed_grow(
    mesh: &mut MeshModel,
    seeds: &[SeedInput],
    params: &SeedGrowParams,
) -> Result<SeedGrowResult, String> {
    if seeds.is_empty() {
        return Err("at least one seed required".into());
    }
    let n = mesh.faces.len();
    if n == 0 {
        return Err("mesh has no faces".into());
    }

    // 1. Snap each seed click to a face.
    let mut seed_faces: Vec<u32> = Vec::with_capacity(seeds.len());
    for s in seeds {
        let (vi, _snapped) = snap_point_to_vertex_on_face(mesh, &s.point, s.face_index)
            .ok_or_else(|| "seed did not snap to a face".to_string())?;
        let fid = mesh
            .face_of_vertex(vi)
            .ok_or_else(|| format!("seed vertex {vi} has no incident face"))?;
        seed_faces.push(fid);
    }

    // Each unique seed face gets one fresh manual label. Two seeds landing on
    // the same face share a region.
    let mut face_to_region: HashMap<u32, u32> = HashMap::new();
    let mut seed_regions: Vec<u32> = Vec::with_capacity(seed_faces.len());
    for &fid in &seed_faces {
        let region = *face_to_region
            .entry(fid)
            .or_insert_with(|| mesh.alloc_manual_label());
        seed_regions.push(region);
    }

    // 2. Build adjacency with edge length + barrier flag.
    let mut centroid = vec![[0.0f32; 3]; n];
    for f in 0..n {
        centroid[f] = face_centroid(mesh, f as u32);
    }
    let have_normals = mesh.normals.len() == n;
    let cos_barrier = (params.barrier_deg as f64).to_radians().cos() as f32;

    let mut adj: Vec<Vec<(u32, f32, bool)>> = vec![Vec::new(); n];
    for e in mesh.face_adjacency.edge_references() {
        let a = e.source().index() as u32;
        let b = e.target().index() as u32;
        if a == b {
            continue;
        }
        let w = dist3(&centroid[a as usize], &centroid[b as usize]);
        let barrier = if have_normals {
            match shared_edge(mesh, a, b) {
                Some(_) => {
                    let na = mesh.normals[a as usize];
                    let nb = mesh.normals[b as usize];
                    let d = na[0] * nb[0] + na[1] * nb[1] + na[2] * nb[2];
                    let d = d.clamp(-1.0, 1.0);
                    let ang = d.acos();
                    (ang * 180.0 / std::f32::consts::PI) > params.barrier_deg
                        && (ang * 180.0 / std::f32::consts::PI) < 180.0 - params.barrier_deg
                }
                None => false,
            }
        } else {
            false
        };
        let _ = &cos_barrier;
        adj[a as usize].push((b, w, barrier));
        adj[b as usize].push((a, w, barrier));
    }

    // 3. Connected components ignoring barriers (each = one seedable patch).
    let mut comp = vec![u32::MAX; n];
    let mut comp_id = 0u32;
    for start in 0..n as u32 {
        if comp[start as usize] != u32::MAX {
            continue;
        }
        comp[start as usize] = comp_id;
        let mut stack = vec![start];
        while let Some(c) = stack.pop() {
            for &(nb, _, bar) in &adj[c as usize] {
                if !bar && comp[nb as usize] == u32::MAX {
                    comp[nb as usize] = comp_id;
                    stack.push(nb);
                }
            }
        }
        comp_id += 1;
    }

    // 4. Multi-source Dijkstra over the barrier-free graph: each face → nearest
    //    seed *within its component*. Faces in components with no seed stay
    //    u32::MAX (handled by fallback below).
    let mut region = vec![u32::MAX; n];
    let mut dist = vec![f32::INFINITY; n];
    let mut pq: BinaryHeap<(std::cmp::Reverse<Key>, u32)> = BinaryHeap::new();
    for (i, &fid) in seed_faces.iter().enumerate() {
        let r = seed_regions[i];
        dist[fid as usize] = 0.0;
        region[fid as usize] = r;
        pq.push((std::cmp::Reverse(Key(0.0)), fid));
    }
    while let Some((std::cmp::Reverse(d), c)) = pq.pop() {
        if d.0 > dist[c as usize] {
            continue;
        }
        for &(nb, w, bar) in &adj[c as usize] {
            if bar {
                continue;
            }
            let nd = d.0 + w;
            if nd < dist[nb as usize] {
                dist[nb as usize] = nd;
                region[nb as usize] = region[c as usize];
                pq.push((std::cmp::Reverse(Key(nd)), nb));
            }
        }
    }

    // 5. Fallback: any face still unassigned (a component the user skipped)
    //    joins the geodesic-nearest seed *across* barriers. Barrier crossings
    //    carry a penalty so the route prefers going around a crease.
    if region.iter().any(|&r| r == u32::MAX) {
        // Mesh extent drives the penalty magnitude.
        let mut ext = [0.0f32; 3];
        for v in &mesh.vertices {
            ext[0] = ext[0].max(v[0].abs());
            ext[1] = ext[1].max(v[1].abs());
            ext[2] = ext[2].max(v[2].abs());
        }
        let penalty = (ext[0] + ext[1] + ext[2]).max(1.0) * 5.0;
        let mut dist2 = vec![f32::INFINITY; n];
        let mut region2 = vec![u32::MAX; n];
        let mut pq2: BinaryHeap<(std::cmp::Reverse<Key>, u32)> = BinaryHeap::new();
        for (i, &fid) in seed_faces.iter().enumerate() {
            let r = seed_regions[i];
            dist2[fid as usize] = 0.0;
            region2[fid as usize] = r;
            pq2.push((std::cmp::Reverse(Key(0.0)), fid));
        }
        while let Some((std::cmp::Reverse(d), c)) = pq2.pop() {
            if d.0 > dist2[c as usize] {
                continue;
            }
            for &(nb, w, bar) in &adj[c as usize] {
                let ww = if bar { w + penalty } else { w };
                let nd = d.0 + ww;
                if nd < dist2[nb as usize] {
                    dist2[nb as usize] = nd;
                    region2[nb as usize] = region2[c as usize];
                    pq2.push((std::cmp::Reverse(Key(nd)), nb));
                }
            }
        }
        for f in 0..n {
            if region[f] == u32::MAX {
                region[f] = region2[f];
            }
        }
    }

    // 6. Optimizer: merge sub-threshold regions into their largest neighbour.
    if params.optimizer {
        let min_faces = ((n as f32) * 0.002).max(8.0) as usize;
        for _pass in 0..3 {
            // Count faces per region.
            let mut counts: HashMap<u32, usize> = HashMap::new();
            for &r in &region {
                *counts.entry(r).or_insert(0) += 1;
            }
            // Find the smallest region that is below threshold.
            let tiny = counts
                .iter()
                .filter(|(_, &c)| c < min_faces)
                .min_by_key(|(_, &c)| c)
                .map(|(&r, _)| r);
            let tiny = match tiny {
                Some(t) => t,
                None => break,
            };
            // Reassign tiny region's faces to the neighbour region sharing the
            // most boundary edges.
            let mut neighbour: HashMap<u32, usize> = HashMap::new();
            for f in 0..n {
                if region[f] != tiny {
                    continue;
                }
                for &(nb, _, _) in &adj[f] {
                    let rn = region[nb as usize];
                    if rn != tiny {
                        *neighbour.entry(rn).or_insert(0) += 1;
                    }
                }
            }
            if let Some((target, _)) = neighbour.iter().max_by_key(|(_, &c)| c) {
                let target = *target;
                for f in 0..n {
                    if region[f] == tiny {
                        region[f] = target;
                    }
                }
            } else {
                break; // isolated region, leave it
            }
        }
    }

    // 7. Write labels + colours, record one undo entry (labels only — paint
    //    is untouched, mirroring finalize_manual_region).
    let mut prev_colors = Vec::with_capacity(n);
    let mut prev_labels = Vec::with_capacity(n);
    let mut moved = 0usize;
    for f in 0..n {
        let mut r = region[f];
        if r == u32::MAX {
            // Disconnected shell with no reachable seed: give it its own region.
            r = mesh.alloc_manual_label();
            region[f] = r;
        }
        let fi = f as u32;
        prev_labels.push((fi, mesh.segment_labels[f]));
        prev_colors.push((fi, mesh.face_colors[f]));
        if mesh.segment_labels[f] != r {
            moved += 1;
        }
        mesh.segment_labels[f] = r;
        // Same rule as fuse: `face_colors` carries USER paint only — a manual
        // grow assigns labels (segment view tints by label) but never paints.
    }
    mesh.history
        .record(OpKind::ManualRegion, None, &prev_colors, &prev_labels);
    mesh.rebuild_segments();

    let region_count = region.iter().collect::<HashSet<_>>().len();
    Ok(SeedGrowResult {
        segments: mesh.sorted_segments(),
        segment_labels: mesh.segment_labels.clone(),
        moved_faces: moved,
        region_count,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mesh::model::DEFAULT_FACE_COLOR;
    use crate::segment::metrics::unit_cube;

    #[test]
    fn single_seed_covers_whole_mesh_via_fallback() {
        let mut mesh = unit_cube();
        let r = seed_grow(
            &mut mesh,
            &[SeedInput {
                point: [-1.0, -1.0, -1.0],
                face_index: 0,
            }],
            &SeedGrowParams::default(),
        )
        .unwrap();
        assert_eq!(r.region_count, 1, "one seed must yield one region");
        let lbl = mesh.segment_labels[0];
        assert!(
            mesh.segment_labels.iter().all(|&l| l == lbl),
            "every face must join the single seed region"
        );
    }

    #[test]
    fn two_seeds_split_into_two_regions() {
        let mut mesh = unit_cube();
        let r = seed_grow(
            &mut mesh,
            &[
                SeedInput {
                    point: [-1.0, -1.0, -1.0],
                    face_index: 0,
                },
                SeedInput {
                    point: [1.0, 1.0, 1.0],
                    face_index: 11,
                },
            ],
            &SeedGrowParams::default(),
        )
        .unwrap();
        assert_eq!(r.region_count, 2, "two seeds must yield two regions");
    }

    #[test]
    fn no_seeds_is_an_error() {
        let mut mesh = unit_cube();
        let err = seed_grow(&mut mesh, &[], &SeedGrowParams::default());
        assert!(err.is_err());
    }

    /// GUI audit round 3 (B1, same rule as fuse): a manual grow assigns labels
    /// but never paints — existing paint survives, unpainted faces stay at the
    /// default base colour so only USER colours reach the export buffer.
    #[test]
    fn seed_grow_keeps_user_paint_only() {
        let mut mesh = unit_cube();
        let painted: [u8; 4] = [0, 32, 255, 255];
        mesh.face_colors[11] = painted;
        seed_grow(
            &mut mesh,
            &[SeedInput {
                point: [-1.0, -1.0, -1.0],
                face_index: 0,
            }],
            &SeedGrowParams::default(),
        )
        .unwrap();
        assert_eq!(
            mesh.face_colors[11], painted,
            "existing paint must survive seed_grow"
        );
        assert!(
            mesh.face_colors
                .iter()
                .enumerate()
                .all(|(i, c)| i == 11 || c == &DEFAULT_FACE_COLOR),
            "grow must not auto-paint unpainted faces (export = user colours only)"
        );
    }
}
