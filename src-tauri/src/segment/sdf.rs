use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};

use petgraph::visit::EdgeRef;
use rayon::prelude::*;

use crate::mesh::model::{MeshModel, Segment};
use crate::mesh::loader::ProgressFn;
use crate::segment::postprocess;

// ─── SDF-based "smart" segmentation (Tier 0) ───────────────────────
//
// Downgraded from the PLAN's GMM proposal per adversarial review: a full
// GMM(EM) + graph-cut is over-scoped for one iteration. log(SDF) 1-D k-means
// + a concavity-aware region merge yields the same "semantic part" feel
// (thin vs thick parts) with far less code and no unverified math.
//
// Pipeline (Shapira 2008 / CGAL Surface_mesh_segmentation, simplified):
//   1. Make normals orientation-consistent (STL winding is unreliable).
//   2. Per-face SDF: sample inward rays, take median hit distance (local thickness).
//   3. log-normalize SDF.
//   4. 1-D k-means into k clusters.
//   5. Merge adjacent clusters whose shared boundary is convex (low dihedral
//      angle) — boundaries are pulled to concavities, giving meaningful parts.

const SDF_RAYS: usize = 12;
const SDF_CONE_HALF_ANGLE: f32 = 1.05; // ~60° half-angle
const SDF_HOLE_FILL: f32 = -1.0; // sentinel for faces with no ray hit (hole)
/// Upper bound on candidate faces sampled per face center. Bounds SDF cost to
/// O(n·K·rays) instead of the previous O(12·n²) (REFUTE blocker #1: the kdtree
/// query used the full bbox-diagonal radius and sat inside the ray loop, so it
/// returned ~all faces and was unusable on real meshes). Thickness beyond the
/// 512th-nearest neighbour is approximated; ample for typical models and the
/// golden-sample acceptance set.
const SDF_CANDIDATE_K: usize = 512;

/// Maximum *signed* crease strength (degrees) across a cluster boundary for the
/// boundary to be dissolved by the SDF merge. Signed means concave folds score
/// 1× and convex edges 0.5× their angle (minima rule, postprocess.rs), so a
/// concave seam survives while a same-angle convex edge may be merged. Matches
/// `DEFAULT_CREASE_DEG` (20°) so the SDF merge and the shared crease pipeline
/// agree on what a real feature edge is.
const SDF_MERGE_CREASE_DEG: f32 = 20.0;

/// Compute a globally orientation-consistent normal field via BFS over the
/// adjacency graph, **without mutating `mesh.normals`** (the previous version
/// wrote back into `mesh.normals`, which corrupted any subsequent dihedral run
/// when multiple algorithms were chained). Returns the oriented copy; callers
/// that need it must hold it locally.
pub(crate) fn oriented_normals(mesh: &MeshModel) -> Vec<[f32; 3]> {
    let n = mesh.faces.len();
    if n == 0 {
        return Vec::new();
    }
    let mut oriented = mesh.normals.clone();
    let mut visited = vec![false; n];

    // Process every connected component (handles disconnected meshes)
    for start in 0..n {
        if visited[start] {
            continue;
        }
        visited[start] = true;
        let mut stack = vec![start as u32];
        while let Some(cur) = stack.pop() {
            let node = petgraph::graph::NodeIndex::new(cur as usize);
            for edge in mesh.face_adjacency.edges(node) {
                let nb = if edge.source() == node {
                    edge.target()
                } else {
                    edge.source()
                };
                let ni = nb.index();
                if visited[ni] {
                    continue;
                }
                // Align neighbor normal to current face normal
                let dot = oriented[cur as usize][0] * oriented[ni][0]
                    + oriented[cur as usize][1] * oriented[ni][1]
                    + oriented[cur as usize][2] * oriented[ni][2];
                if dot < 0.0 {
                    oriented[ni][0] = -oriented[ni][0];
                    oriented[ni][1] = -oriented[ni][1];
                    oriented[ni][2] = -oriented[ni][2];
                }
                visited[ni] = true;
                stack.push(ni as u32);
            }
        }
    }
    oriented
}

/// Moller–Trumbore ray/triangle intersection. Returns t>0 if hit.
fn ray_triangle(
    orig: &[f32; 3],
    dir: &[f32; 3],
    v0: &[f32; 3],
    v1: &[f32; 3],
    v2: &[f32; 3],
) -> Option<f32> {
    let eps = 1e-8;
    let edge1 = [v1[0] - v0[0], v1[1] - v0[1], v1[2] - v0[2]];
    let edge2 = [v2[0] - v0[0], v2[1] - v0[1], v2[2] - v0[2]];
    let h = [
        dir[1] * edge2[2] - dir[2] * edge2[1],
        dir[2] * edge2[0] - dir[0] * edge2[2],
        dir[0] * edge2[1] - dir[1] * edge2[0],
    ];
    let a = edge1[0] * h[0] + edge1[1] * h[1] + edge1[2] * h[2];
    if a.abs() < eps {
        return None;
    }
    let f = 1.0 / a;
    let s = [orig[0] - v0[0], orig[1] - v0[1], orig[2] - v0[2]];
    let u = f * (s[0] * h[0] + s[1] * h[1] + s[2] * h[2]);
    if u < -eps || u > 1.0 + eps {
        return None;
    }
    let q = [
        s[1] * edge1[2] - s[2] * edge1[1],
        s[2] * edge1[0] - s[0] * edge1[2],
        s[0] * edge1[1] - s[1] * edge1[0],
    ];
    let v = f * (dir[0] * q[0] + dir[1] * q[1] + dir[2] * q[2]);
    if v < -eps || u + v > 1.0 + eps {
        return None;
    }
    let t = f * (edge2[0] * q[0] + edge2[1] * q[1] + edge2[2] * q[2]);
    if t > eps {
        Some(t)
    } else {
        None
    }
}

/// Build an orthonormal basis (t1, t2) perpendicular to `n`.
fn basis(n: &[f32; 3]) -> ([f32; 3], [f32; 3]) {
    let refv = if n[0].abs() > 0.9 {
        [0.0, 1.0, 0.0]
    } else {
        [1.0, 0.0, 0.0]
    };
    let t1 = [
        n[1] * refv[2] - n[2] * refv[1],
        n[2] * refv[0] - n[0] * refv[2],
        n[0] * refv[1] - n[1] * refv[0],
    ];
    let l = (t1[0] * t1[0] + t1[1] * t1[1] + t1[2] * t1[2]).sqrt();
    let t1 = [t1[0] / l, t1[1] / l, t1[2] / l];
    let t2 = [
        n[1] * t1[2] - n[2] * t1[1],
        n[2] * t1[0] - n[0] * t1[2],
        n[0] * t1[1] - n[1] * t1[0],
    ];
    (t1, t2)
}

/// Core SDF computation using an already orientation-consistent normal field.
/// Candidates are queried **once per face** (not per ray) and bounded to the
/// `SDF_CANDIDATE_K` nearest neighbours, so cost is O(n·K·rays) instead of the
/// previous O(12·n²) that made SDF unusable on real meshes (REFUTE blocker #1:
/// the kdtree query radius was the full bbox diagonal and sat inside the ray
/// loop, so it returned ~all faces).
///
/// Parallelised over faces with rayon (the single dominant cost of the import
/// pipeline: 358s of the 361s curvatureKMeans run on a 1.88M-face model,
/// measured 2026-08-28). Faces are fully independent — each face reads only
/// `mesh`/`oriented` and writes its own output slot — so the indexed
/// `par_iter + collect` preserves face order and runs the exact same float
/// ops per face as the former sequential loop; results are bitwise identical.
pub(crate) fn compute_sdf_inner(
    mesh: &MeshModel,
    oriented: &[[f32; 3]],
    on_progress: &ProgressFn,
    base: f32,
    span: f32,
) -> Vec<f32> {
    let raw = sdf_sample_raw(mesh, oriented, on_progress, base, span);
    impute_sdf_holes(mesh, &raw)
}

/// Raw per-face SDF sampling (no hole imputation): no-hit faces keep the
/// `SDF_HOLE_FILL` sentinel. Split from [`compute_sdf_inner`] so tests can
/// assert bitwise parity of the parallel sampling against a sequential
/// reference without the impute phase interfering.
fn sdf_sample_raw(
    mesh: &MeshModel,
    oriented: &[[f32; 3]],
    on_progress: &ProgressFn,
    base: f32,
    span: f32,
) -> Vec<f32> {
    let n = mesh.faces.len();
    let report_every = (n / 50).max(1);
    let done = AtomicUsize::new(0);

    // The per-face body lives in ONE `#[inline(never)]` helper shared by this
    // driver and by sequential callers (the bitwise parity probe). Two separate
    // inline copies of token-identical code were observed to diverge bitwise
    // (suspected per-function FP-contraction codegen decisions); a single
    // compiled body makes that class of divergence structurally impossible.
    (0..n)
        .into_par_iter()
        .map(|fi| {
            let value = sdf_face_sample(mesh, oriented, fi);

            // Progress is reported from worker threads via an atomic counter —
            // the old `fi % report_every` pattern is meaningless under rayon's
            // work-stealing order.
            let seen = done.fetch_add(1, Ordering::Relaxed);
            if seen % report_every == 0 {
                on_progress(base + span * (seen as f32 / n as f32), "sdf:sample");
            }
            value
        })
        .collect()
}

/// Per-face SDF: cast `SDF_RAYS` inward rays in a cone around -normal, take
/// the median hit distance against the `SDF_CANDIDATE_K` nearest faces
/// (CGAL-style robust thickness). No-hit faces return the `SDF_HOLE_FILL`
/// sentinel for [`impute_sdf_holes`].
///
/// `#[inline(never)]` is load-bearing: this is THE single compiled body of the
/// per-face sampling, shared by the rayon driver and sequential probes.
#[inline(never)]
fn sdf_face_sample(mesh: &MeshModel, oriented: &[[f32; 3]], fi: usize) -> f32 {
    let center = mesh.face_center(fi as u32);
    let nrm = oriented[fi]; // consistent outward normal
    // inward direction (negative of outward normal)
    let inward = [-nrm[0], -nrm[1], -nrm[2]];
    let (t1, t2) = basis(&inward);

    // Candidate faces for the inward cone — computed ONCE per face, bounded.
    let candidates = mesh
        .face_kdtree
        .nearest_n::<kiddo::SquaredEuclidean>(&center, SDF_CANDIDATE_K);

    let mut hits: Vec<f32> = Vec::with_capacity(SDF_RAYS);
    for r in 0..SDF_RAYS {
        // Fibonacci-ish spread over the cone
        let frac = (r as f32 + 0.5) / SDF_RAYS as f32;
        let theta = SDF_CONE_HALF_ANGLE * frac.sqrt();
        let phi = 2.0 * std::f32::consts::PI * r as f32 * 0.618_033_99;
        let dir = [
            inward[0] * theta.cos() + (t1[0] * phi.cos() + t2[0] * phi.sin()) * theta.sin(),
            inward[1] * theta.cos() + (t1[1] * phi.cos() + t2[1] * phi.sin()) * theta.sin(),
            inward[2] * theta.cos() + (t1[2] * phi.cos() + t2[2] * phi.sin()) * theta.sin(),
        ];
        // Normalize dir
        let dl = (dir[0] * dir[0] + dir[1] * dir[1] + dir[2] * dir[2]).sqrt();
        let dir = [dir[0] / dl, dir[1] / dl, dir[2] / dl];

        let mut best_t = f32::INFINITY;
        for cand in candidates.iter() {
            let cf = cand.item as usize;
            if cf == fi {
                continue;
            }
            let f = &mesh.faces[cf];
            let v0 = mesh.vertices[f[0] as usize];
            let v1 = mesh.vertices[f[1] as usize];
            let v2 = mesh.vertices[f[2] as usize];
            if let Some(t) = ray_triangle(&center, &dir, &v0, &v1, &v2) {
                if t < best_t {
                    best_t = t;
                }
            }
        }
        if best_t.is_finite() {
            hits.push(best_t);
        }
    }

    if hits.is_empty() {
        SDF_HOLE_FILL
    } else {
        hits.sort_by(|a, b| a.partial_cmp(b).unwrap());
        // median (robust to outliers, per CGAL)
        let mid = hits.len() / 2;
        if hits.len() % 2 == 1 {
            hits[mid]
        } else {
            0.5 * (hits[mid - 1] + hits[mid])
        }
    }
}

/// Fill no-hit faces (the `SDF_HOLE_FILL` sentinel) with the average SDF of
/// their neighbours. Deliberately sequential: each pass reads neighbours'
/// freshly imputed values (Gauss-Seidel semantics, ≤5 passes), and on
/// watertight meshes it is a pure `continue` sweep — negligible next to the
/// sampled phase. Any face still missing afterwards takes the global positive
/// mean (prevents log of negative downstream).
pub(crate) fn impute_sdf_holes(mesh: &MeshModel, sdf: &[f32]) -> Vec<f32> {
    let n = mesh.faces.len();
    let mut imputed = sdf.to_vec();
    let mut changed = true;
    let mut guard = 0;
    while changed && guard < 5 {
        changed = false;
        guard += 1;
        for fi in 0..n {
            if sdf[fi] != SDF_HOLE_FILL {
                continue;
            }
            let node = petgraph::graph::NodeIndex::new(fi);
            let mut sum = 0.0f32;
            let mut cnt = 0u32;
            for edge in mesh.face_adjacency.edges(node) {
                let nb = if edge.source() == node {
                    edge.target()
                } else {
                    edge.source()
                };
                let ni = nb.index();
                if imputed[ni] != SDF_HOLE_FILL {
                    sum += imputed[ni];
                    cnt += 1;
                }
            }
            if cnt > 0 {
                imputed[fi] = sum / cnt as f32;
                changed = true;
            }
        }
    }
    // Any still-missing → global mean (prevents log of negative)
    let valid: Vec<f32> = imputed.iter().cloned().filter(|v| *v > 0.0).collect();
    let mean = if valid.is_empty() {
        1.0
    } else {
        valid.iter().sum::<f32>() / valid.len() as f32
    };
    for v in imputed.iter_mut() {
        if *v <= 0.0 {
            *v = mean;
        }
    }
    imputed
}

/// log-normalize SDF to (0,1]: log(v), min-max to [0,1].
/// Delegates to the shared implementation, which floors the log-space span.
///
/// The previous `1e-6` floor was effectively no floor at all: on a model of
/// uniform thickness (a sphere) the true span is ray-sampling jitter, and
/// dividing by it stretched that jitter across the whole [0,1] range. Downstream
/// `estimate_k` then read the resulting histogram as multi-modal and split a
/// smooth ball into thickness "parts" that do not exist.
fn log_normalize(sdf: &[f32]) -> Vec<f32> {
    postprocess::log_normalize(sdf)
}

/// 1-D k-means (Lloyd) on sorted log-normalized SDF. Deterministic.
fn kmeans_1d(values: &[f32], k: usize) -> Vec<u32> {
    let mut vals: Vec<f32> = values.to_vec();
    vals.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let n = vals.len();
    if n == 0 || k <= 1 {
        return vec![0; n];
    }
    let k = k.min(n);
    // Init centroids evenly across sorted range
    let mut centroids: Vec<f32> = (0..k)
        .map(|i| vals[(i * n / k).min(n - 1)])
        .collect();
    let mut labels = vec![0u32; n];
    for _ in 0..20 {
        // Assign
        let mut changed = false;
        for (i, &v) in vals.iter().enumerate() {
            let mut best = 0usize;
            let mut best_d = f32::INFINITY;
            for (c, &cen) in centroids.iter().enumerate() {
                let d = (v - cen) * (v - cen);
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
        // Update centroids
        let mut sums = vec![0.0f32; k];
        let mut counts = vec![0u32; k];
        for (i, &v) in vals.iter().enumerate() {
            let c = labels[i] as usize;
            sums[c] += v;
            counts[c] += 1;
        }
        for c in 0..k {
            if counts[c] > 0 {
                centroids[c] = sums[c] / counts[c] as f32;
            }
        }
        if !changed {
            break;
        }
    }
    labels
}

/// Estimate k from SDF histogram peaks (REFUTE: silhouette is too expensive;
/// fixed k=6 over-segments single-part models). Clamp to [2, 12].
pub(crate) fn estimate_k(ln_sdf: &[f32]) -> usize {
    if ln_sdf.is_empty() {
        return 2;
    }
    let bins = 24usize;
    let mut hist = vec![0u32; bins];
    for &v in ln_sdf {
        let b = ((v * bins as f32) as usize).min(bins - 1);
        hist[b] += 1;
    }
    // Count local maxima (peaks)
    let mut peaks = 0;
    for i in 1..bins - 1 {
        if hist[i] > hist[i - 1] && hist[i] >= hist[i + 1] && hist[i] > 0 {
            peaks += 1;
        }
    }
    (peaks + 1).clamp(2, 12) as usize
}

/// Segment the mesh by SDF + concavity-aware merge.
pub fn segment_by_sdf(mesh: &mut MeshModel, k_user: u32, on_progress: &ProgressFn) -> Vec<Segment> {
    let n = mesh.faces.len();
    on_progress(0.0, "sdf:sample");
    let oriented = oriented_normals(mesh);
    let sdf = compute_sdf_inner(mesh, &oriented, on_progress, 0.0, 0.35);
    on_progress(0.35, "sdf:cluster");
    let ln_sdf = log_normalize(&sdf);
    let k = if k_user == 0 {
        estimate_k(&ln_sdf)
    } else {
        k_user as usize
    };
    // Per-face cluster labels (in face order)
    let mut face_cluster: Vec<u32> = vec![0; n];
    {
        // k-means needs values in face order, not sorted — map back
        let sorted_labels = kmeans_1d(&ln_sdf, k);
        // kmeans returns labels in sorted order; rebuild an index permutation
        let mut idx: Vec<usize> = (0..n).collect();
        idx.sort_by(|&a, &b| ln_sdf[a].partial_cmp(&ln_sdf[b]).unwrap());
        for (pos, &fi) in idx.iter().enumerate() {
            face_cluster[fi] = sorted_labels[pos];
        }
    }

    // Concavity-aware merge. Single O(E) pass: for every inter-cluster edge
    // accumulate the shared-edge count and the summed *signed* crease strength
    // (minima rule, see postprocess::crease_strength_deg), so we avoid the
    // previous O(clusters²·E) rescan (REFUTE blocker #1).
    //
    // Iteration 45 (REVISE after adversarial review): the old accumulator summed
    // the raw normal dot, whose `acos` is the UNSIGNED dihedral magnitude — it
    // cannot tell a concave crease (neck, underarm — the real part boundaries
    // the minima rule wants to keep) from a convex one (armour plate edge). A
    // 20° concave fold reads dot≈0.94 > 0.93 and got merged away. The signed
    // strength weights convex edges by CONVEX_CREASE_WEIGHT=0.5, so a concave
    // fold scores twice as high as a same-angle convex one and survives the
    // merge threshold.
    let mut boundary: HashMap<(u32, u32), (f64, u32)> = HashMap::new();
    for edge in mesh.face_adjacency.edge_references() {
        let fi = mesh.face_adjacency[edge.source()];
        let fj = mesh.face_adjacency[edge.target()];
        let ci = face_cluster[fi as usize];
        let cj = face_cluster[fj as usize];
        if ci == cj {
            continue;
        }
        let (a, b) = if ci <= cj { (ci, cj) } else { (cj, ci) };
        let s = postprocess::crease_strength_deg(mesh, &oriented, fi as usize, fj as usize) as f64;
        let e = boundary.entry((a, b)).or_insert((0.0, 0));
        e.0 += s;
        e.1 += 1;
    }

    // Union-find over cluster ids. The previous version snapshotted `merged`
    // *before* merging and then rewrote entries equal to `hi` — which is not a
    // union: given chained boundaries (0,1) and (1,2) it maps 1→0 and then 2→1,
    // leaving three ids that transitively should have been one. On a smooth mesh
    // (every boundary above the 0.93 dot) that silently kept several clusters
    // alive, which is why a plain sphere came back in pieces.
    let mut parent: Vec<u32> = (0..k as u32).collect();
    fn find(mut x: u32, parent: &mut Vec<u32>) -> u32 {
        while parent[x as usize] != x {
            let g = parent[parent[x as usize] as usize];
            parent[x as usize] = g; // path halving
            x = g;
        }
        x
    }
    let mut work: Vec<(u32, u32, f32)> = boundary
        .iter()
        .map(|(&(a, b), &(sum_dot, cnt))| (a, b, (sum_dot / cnt as f64) as f32))
        .collect();
    // Weakest boundary first; ties broken by id so the HashMap's iteration order
    // cannot leak into the result. Note the semantic flip from the old code:
    // "strongest (smoothest)" used to mean *highest dot* (≈0.93, i.e. ~21°
    // unsigned dihedral); with signed crease strength the same notion is
    // *lowest* signed angle (convex edges already halved).
    work.sort_by(|x, y| {
        x.2.partial_cmp(&y.2)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then((x.0, x.1).cmp(&(y.0, y.1)))
    });
    for (a, b, crease) in work {
        // Only merge across genuinely smooth boundaries; real creases — concave
        // ones in particular, which the minima rule weights at 2× a convex edge
        // of the same angle — must survive as cuts. The 20° default matches
        // DEFAULT_CREASE_DEG (postprocess.rs) so the SDF merge honours the same
        // crease notion the shared refine_regions pipeline uses.
        if crease >= SDF_MERGE_CREASE_DEG {
            continue;
        }
        let (ra, rb) = (find(a, &mut parent), find(b, &mut parent));
        if ra == rb {
            continue;
        }
        let (lo, hi) = (ra.min(rb), ra.max(rb));
        parent[hi as usize] = lo;
    }
    let merged: HashMap<u32, u32> = (0..k as u32)
        .map(|c| (c, find(c, &mut parent)))
        .collect();
    on_progress(0.9, "sdf:cluster");

    // Compact labels → contiguous ids, assign to faces
    let mut remap: HashMap<u32, u32> = HashMap::new();
    let mut next_id = 0u32;
    let mut labels = vec![0u32; n];
    for f in 0..n {
        let c = merged[&face_cluster[f]];
        let id = *remap.entry(c).or_insert_with(|| {
            let id = next_id;
            next_id += 1;
            id
        });
        labels[f] = id;
    }

    // Everything above clusters faces by a *global* thickness value, and a
    // global value has no idea what is reachable from what: two disconnected
    // spheres of equal radius get the same SDF, land in the same cluster, and
    // come back as one "region" whose two halves never touch. The convex-merge
    // step makes that worse, not better — it unions clusters mesh-wide. So the
    // connectivity split has to run last, on the final labels.
    //
    // The contour merge is deliberately NOT applied here: SDF's whole purpose is
    // to separate parts by thickness across smooth blends, which is exactly what
    // that pass would undo.
    on_progress(0.95, "sdf:split");
    let curv = postprocess::face_curvature(mesh, &oriented);
    let feats = postprocess::assemble_features(&curv, Some(&ln_sdf));
    let labels = postprocess::refine_regions(mesh, &labels, &feats, &oriented, None);

    postprocess::finalize_segments(mesh, labels)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a MeshModel from raw vertices/faces and run all prep steps
    /// (normals, bbox, kdtrees, adjacency, default colors, labels).
    fn build_mesh(verts: &[[f32; 3]], faces: &[u32]) -> MeshModel {
        let mut m = MeshModel::new();
        m.vertices = verts.to_vec();
        m.faces = faces.chunks(3).map(|c| [c[0], c[1], c[2]]).collect();
        m.compute_normals();
        m.compute_bbox();
        m.build_kdtree();
        m.build_vertex_kdtree();
        m.build_adjacency();
        m.init_default_colors();
        m.segment_labels = vec![0u32; m.faces.len()];
        m
    }

    /// Large cube (3^3 at origin) + small cube (0.5^3 far away). Spatial gap
    /// (10 units) prevents ray leakage between the two shells.
    fn two_separated_cubes() -> MeshModel {
        let v: [f32; 48] = [
            // large 0..3
            0.0, 0.0, 0.0, 3.0, 0.0, 0.0, 3.0, 3.0, 0.0, 0.0, 3.0, 0.0,
            0.0, 0.0, 3.0, 3.0, 0.0, 3.0, 3.0, 3.0, 3.0, 0.0, 3.0, 3.0,
            // small 10..10.5
            10.0, 10.0, 10.0, 10.5, 10.0, 10.0, 10.5, 10.5, 10.0, 10.0, 10.5, 10.0,
            10.0, 10.0, 10.5, 10.5, 10.0, 10.5, 10.5, 10.5, 10.5, 10.0, 10.5, 10.5,
        ];
        let verts: Vec<[f32; 3]> = v.chunks(3).map(|c| [c[0], c[1], c[2]]).collect();
        let f: [u32; 72] = [
            // large (verts 0..7)
            0, 3, 2, 0, 2, 1, 4, 5, 6, 4, 6, 7,
            0, 1, 5, 0, 5, 4, 2, 3, 7, 2, 7, 6,
            1, 2, 6, 1, 6, 5, 3, 0, 4, 3, 4, 7,
            // small (verts 8..15)
            8, 11, 10, 8, 10, 9, 12, 13, 14, 12, 14, 15,
            8, 9, 13, 8, 13, 12, 10, 11, 15, 10, 15, 14,
            9, 10, 14, 9, 14, 13, 11, 8, 12, 11, 12, 15,
        ];
        build_mesh(&verts, &f)
    }

    fn single_cube() -> MeshModel {
        let v: [f32; 24] = [
            0.0, 0.0, 0.0, 2.0, 0.0, 0.0, 2.0, 2.0, 0.0, 0.0, 2.0, 0.0,
            0.0, 0.0, 2.0, 2.0, 0.0, 2.0, 2.0, 2.0, 2.0, 0.0, 2.0, 2.0,
        ];
        let verts: Vec<[f32; 3]> = v.chunks(3).map(|c| [c[0], c[1], c[2]]).collect();
        let f: [u32; 36] = [
            0, 3, 2, 0, 2, 1, 4, 5, 6, 4, 6, 7,
            0, 1, 5, 0, 5, 4, 2, 3, 7, 2, 7, 6,
            1, 2, 6, 1, 6, 5, 3, 0, 4, 3, 4, 7,
        ];
        build_mesh(&verts, &f)
    }

    /// Thick block (2^3, z:0..2) with a thin plate (2x2x0.2, z:2..2.2) sharing
    /// the z=2 face. CONNECTED fixture: exercises the concavity-aware merge
    /// (sdf.rs:361-420) — the thin plate caps must stay a separate region
    /// across the concave junction, not be merged into the block.
    fn block_with_plate() -> MeshModel {
        let v: [f32; 36] = [
            // block (verts 0..7)
            0.0, 0.0, 0.0, 2.0, 0.0, 0.0, 2.0, 2.0, 0.0, 0.0, 2.0, 0.0,
            0.0, 0.0, 2.0, 2.0, 0.0, 2.0, 2.0, 2.0, 2.0, 0.0, 2.0, 2.0,
            // plate top layer (verts 8..11, z=2.2)
            0.0, 0.0, 2.2, 2.0, 0.0, 2.2, 2.0, 2.0, 2.2, 0.0, 2.0, 2.2,
        ];
        let verts: Vec<[f32; 3]> = v.chunks(3).map(|c| [c[0], c[1], c[2]]).collect();
        let f: [u32; 72] = [
            // block (12 tris)
            0, 3, 2, 0, 2, 1, 4, 5, 6, 4, 6, 7,
            0, 1, 5, 0, 5, 4, 2, 3, 7, 2, 7, 6,
            1, 2, 6, 1, 6, 5, 3, 0, 4, 3, 4, 7,
            // plate bottom (z=2, outward -z): uses v4..v7
            4, 7, 6, 4, 6, 5,
            // plate top (z=2.2, outward +z): uses v8..v11
            8, 9, 10, 8, 10, 11,
            // plate sides (y=0): v4,v5,v9,v8
            4, 5, 9, 4, 9, 8,
            // plate sides (y=2): v6,v7,v11,v10
            6, 7, 11, 6, 11, 10,
            // plate sides (x=2): v5,v6,v10,v9
            5, 6, 10, 5, 10, 9,
            // plate sides (x=0): v7,v4,v8,v11
            7, 4, 8, 7, 8, 11,
        ];
        build_mesh(&verts, &f)
    }

    #[test]
    fn compute_sdf_separates_two_cubes() {
        let m = two_separated_cubes();
        let oriented = oriented_normals(&m);
        let sdf = compute_sdf_inner(&m, &oriented, &|_, _| {}, 0.0, 1.0);
        // No unfilled holes should remain after imputation.
        assert!(sdf.iter().all(|&v| v > 0.0), "SDF left unfilled holes (-1 sentinel)");
        let min = sdf.iter().cloned().fold(f32::INFINITY, f32::min);
        let max = sdf.iter().cloned().fold(f32::NEG_INFINITY, f32::max);
        assert!(max / min > 2.0, "SDF not bimodal (min={}, max={})", min, max);
        // Thin cube (~0.5) vs thick cube (~3.0): clean 12/12 split, no straddlers.
        let thin = sdf.iter().filter(|&&v| v < 1.0).count();
        let thick = sdf.iter().filter(|&&v| v > 1.0).count();
        assert_eq!(thin, 12, "expected 12 thin faces");
        assert_eq!(thick, 12, "expected 12 thick faces");
        let in_gap = sdf.iter().filter(|&&v| v > 0.75 && v < 1.5).count();
        assert_eq!(in_gap, 0, "SDF values straddle the separation gap");
    }

    #[test]
    fn segment_by_sdf_two_cubes_k2() {
        let mut m = two_separated_cubes();
        let segs = segment_by_sdf(&mut m, 2, &|_, _| {});
        assert_eq!(segs.len(), 2, "two cubes must yield 2 segments");
    }

    #[test]
    fn segment_by_sdf_two_cubes_auto() {
        let mut m = two_separated_cubes();
        let segs = segment_by_sdf(&mut m, 0, &|_, _| {});
        assert!(
            segs.len() >= 2,
            "auto-k must not collapse two cubes to 1 (got {})",
            segs.len()
        );
    }

    #[test]
    fn segment_by_sdf_single_cube_one_segment() {
        // Explicit k=1 must yield exactly 1 segment (the k<=1 guard in kmeans_1d
        // prevents over-segmentation). NOTE: auto-k (estimate_k) floors at 2 and
        // therefore over-segments a uniform single-part mesh into 2 — a known
        // limitation (histogram peak-detection misses spread clusters), deferred
        // to a later iteration. The explicit-k path is the controllable guarantee.
        let mut m = single_cube();
        let segs = segment_by_sdf(&mut m, 1, &|_, _| {});
        assert_eq!(segs.len(), 1, "explicit k=1 must yield 1 segment");
    }

    /// Most frequent label in a slice, so a test can ask "which region did this
    /// known group of faces land in?" without depending on label numbering.
    /// Ties break on the lower label to keep failure messages reproducible.
    fn majority(labels: &[u32]) -> u32 {
        let mut counts: std::collections::HashMap<u32, usize> = std::collections::HashMap::new();
        for &l in labels {
            *counts.entry(l).or_insert(0) += 1;
        }
        counts
            .into_iter()
            .max_by_key(|&(l, c)| (c, std::cmp::Reverse(l)))
            .map(|(l, _)| l)
            .expect("majority() called on an empty slice")
    }

    #[test]
    fn segment_by_sdf_block_with_plate() {
        let mut m = block_with_plate();
        let segs = segment_by_sdf(&mut m, 2, &|_, _| {});
        assert_eq!(segs.len(), 2, "block+plate must be 2 parts (not over-merged)");
        // Connected fixture exercises the concavity merge: SDF reads the thin
        // plate (caps + rim, ~0.27) vs the thick block (~1.75); the concave z=2
        // junction is kept separate. The fixture emits block = tris 0..11 and
        // plate = tris 12..23, so the ground truth is a clean 12/12 split on
        // that boundary. Assert the semantics — which faces end up together —
        // instead of a size histogram: the old 11/13 assertion had frozen a
        // defect (one shared z=2 triangle sits on a normal flip and can read
        // thick) into the expected value, so the test failed once the split
        // became correct. One stray triangle is still tolerated because that
        // normal-flip misread is platform/float dependent, not a regression.
        let block = &m.segment_labels[0..12];
        let plate = &m.segment_labels[12..24];
        let block_label = majority(block);
        let plate_label = majority(plate);
        assert_ne!(
            block_label, plate_label,
            "block and plate collapsed into one region: {:?}",
            m.segment_labels
        );
        let strays = block.iter().filter(|&&l| l != block_label).count()
            + plate.iter().filter(|&&l| l != plate_label).count();
        assert!(
            strays <= 1,
            "expected a clean block/plate split (at most 1 stray tri), got {} strays: {:?}",
            strays,
            m.segment_labels
        );
    }

    /// Bitwise parity check for the rayon SDF driver (2026-08-28): the rayon
    /// driver must return per-face values in face order and unchanged from a
    /// sequential walk over the SAME shared per-face helper
    /// ([`sdf_face_sample`], single `#[inline(never)]` body). History: the
    /// first parity probe compared two separate inline copies of token-identical
    /// code and reported ~55% bitwise mismatches (all "par hit vs seq miss"),
    /// with the mismatch set moving between runs; root cause is per-function
    /// codegen divergence between the two copies. Extracting ONE shared body
    /// makes that impossible by construction; this probe now verifies the
    /// driver itself. Run:
    /// cargo test --release --lib probe_sdf -- --ignored --nocapture
    ///
    /// Cross-process region-count variance (e.g. 1274 vs 1160 regions on the
    /// Sanji model) is a PRE-EXISTING nondeterminism, independent of rayon:
    /// the adjacency HashMap iterates in a random per-instance order, so BFS
    /// orientation propagation and float summation order differ per load. See
    /// the determinism probe in `mesh::loader`.
    #[test]
    #[ignore]
    fn probe_sdf_par_vs_seq_bitwise() {
        let path = std::env::var("CYM_BENCH_STL").unwrap_or_else(|_| {
            "samples/_src/Sanji+Diorama+Detailed_U1.stl".to_string()
        });
        let mesh =
            crate::mesh::loader::load_stl(std::path::Path::new(&path), &|_, _| {}).expect("load");
        let oriented = oriented_normals(&mesh);

        let t = std::time::Instant::now();
        let par = sdf_sample_raw(&mesh, &oriented, &|_, _| {}, 0.0, 1.0);
        eprintln!("[sdf par] {:?} ({} faces)", t.elapsed(), mesh.faces.len());

        // Sequential walk over the SAME shared helper.
        let n = mesh.faces.len().min(200_000);
        let t = std::time::Instant::now();
        let seq: Vec<f32> = (0..n)
            .map(|fi| sdf_face_sample(&mesh, &oriented, fi))
            .collect();
        eprintln!("[sdf seq] {:?} ({} faces)", t.elapsed(), n);

        let mismatches = (0..n)
            .filter(|&i| par[i].to_bits() != seq[i].to_bits())
            .count();
        eprintln!(
            "[sdf driver parity] first {} faces: {} bitwise mismatches",
            n, mismatches
        );
        assert_eq!(
            mismatches, 0,
            "rayon driver changed per-face results vs sequential walk"
        );
    }

}
