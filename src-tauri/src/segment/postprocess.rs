//! Shared segmentation post-processing: the steps that turn a *clustering* into
//! a *segmentation*.
//!
//! Every clustering algorithm in this module tree (dihedral aside, which grows
//! regions and is connected by construction) partitions faces by a **global**
//! criterion — a thickness bucket, a curvature bucket — and a global criterion
//! knows nothing about connectivity. Two disconnected spheres of identical
//! radius land in the same thickness bucket and come back as one "region" whose
//! faces are not reachable from each other. Downstream that breaks every
//! consumer that assumes a region is a surface patch: outline extraction, fill,
//! per-region colour assignment, and the manual-merge UI.
//!
//! Merging can never *create* connectivity, so the split has to happen first
//! (REFUTE blocker #2). This module owns that pipeline so `curvature.rs` and
//! `sdf.rs` cannot drift apart on it:
//!
//!   1. [`split_connected_components`] — every label becomes ≥1 connected patch.
//!   2. [`merge_small_by_feature`]     — crumbs are absorbed by their nearest
//!                                       neighbour in feature space.
//!   3. [`merge_smooth_boundaries`]    — region borders that do not sit on a
//!                                       real crease get dissolved (opt-in).
//!
//! Step 3 is what makes the difference between "k clusters" and "the parts a
//! human sees", but it is deliberately opt-in: it rewrites the character of the
//! algorithm that calls it, and SDF's own convex-boundary merge already covers
//! part of the same ground.

use std::collections::HashMap;

use petgraph::visit::EdgeRef;
use rayon::prelude::*;

use crate::mesh::model::{MeshModel, Segment};

/// Min region size as a fraction of total faces (0.2% of the model), so a
/// 500-face model floors at 10 and a 500k-face model would want 1000.
pub(crate) const MIN_REGION_FRACTION: f64 = 0.002;
pub(crate) const MIN_REGION_FLOOR: u32 = 10;
/// Ceiling on the derived minimum, so large models do not end up with absurd
/// thresholds. E2E evidence (1.5M faces, 13k regions, avg 113 faces/region):
///   cap=500 → every region merged into one (catastrophic)
///   cap=30  → only genuinely degenerate fragments (<30 faces) get absorbed
/// 30 is the point below which a region cannot form meaningful geometry.
///
/// (This provenance used to live on a duplicate set of the same three
/// constants in `dihedral.rs`, which no code read; the numbers were kept and
/// the dead copy removed.)
pub(crate) const MIN_REGION_CAP: u32 = 30;
/// Passes over the union-find, not merges: **each pass absorbs every
/// under-sized root**, and raises the size bar when the region count is still
/// above [`MAX_AUTO_REGIONS`]. The bar doubles each pass, so 40 passes covers a
/// 2^40-face mesh — the bound exists only to make termination provable.
pub(crate) const MAX_MERGE_PASSES: u32 = 40;
/// Engineering ceiling on the number of auto regions handed to the UI. This is
/// NOT an accuracy claim: it keeps the region list navigable and, more
/// importantly, keeps auto labels ~24x below `MANUAL_SEGMENT_OFFSET` (100_000)
/// so a shattered clustering can never collide with the manual label space
/// (REFUTE blocker #5).
pub(crate) const MAX_AUTO_REGIONS: usize = 4096;
/// Minima rule (Hoffman & Richards): part boundaries follow *concave* creases.
/// A convex edge therefore has to be twice as sharp to score the same as a
/// concave one — this is what keeps a cube's exterior edges from outranking a
/// real concave neck.
pub(crate) const CONVEX_CREASE_WEIGHT: f32 = 0.5;
/// How far apart two region feature centroids may be and still be dissolved by
/// the crease merge. Both feature axes are normalized to [0,1], so this is a
/// distance in that unit square. Its job is to protect the case the crease rule
/// cannot see: a thin plate blended into a thick block has a *smooth* border but
/// a genuine thickness difference, and merging it away would undo exactly what
/// SDF was enabled for.
pub(crate) const FEATURE_GAP_KEEP: f64 = 0.30;
/// A border this flat is triangulation, not geometry.
///
/// [`FEATURE_GAP_KEEP`] exists to protect a genuinely smooth part boundary (a
/// thin plate blended into a block), but "smooth" and "coplanar" are not the
/// same claim: two faces lying in the same plane cannot belong to different
/// parts, whatever the feature field says. This matters because SDF is *not*
/// constant across a flat wall — near a corner the sample cone catches the
/// adjacent wall and reads thinner — so on a cube the diagonal splitting each
/// square face into two triangles shows a large thickness gap and would survive
/// the feature-gap guard, returning twelve "parts" for six faces.
pub(crate) const COPLANAR_EPS_DEG: f64 = 1.0;
/// Default crease angle when the caller passes 0. Sits between a typical
/// tessellation dihedral (a 24-sector sphere is ~15°) and a real modelled
/// feature edge, and is exposed in the UI because the right value is
/// model-dependent — this is the knob for user fine-tuning.
pub(crate) const DEFAULT_CREASE_DEG: f32 = 20.0;
/// Absolute curvature full-scale: a 90° mean dihedral maps to 1.0.
pub(crate) const CURV_FULL_SCALE: f32 = std::f32::consts::FRAC_PI_2;

/// The size below which a region counts as a crumb worth absorbing.
///
/// "Small" is only meaningful relative to the model. The 10-face floor was
/// written with 100k-triangle prints in mind; on a 12-triangle cube it makes
/// every single face small and the whole model collapses into one region — the
/// same failure hits any low-poly import (a 120-face mechanical part, a
/// decimated preview mesh). Capping the floor at n/20 means a region has to be
/// under 5% of the model before it can be absorbed, which changes nothing for
/// real prints and stops coarse meshes from being dissolved outright.
pub(crate) fn min_region_faces(n_faces: usize) -> u32 {
    MIN_REGION_CAP
        .min(MIN_REGION_FLOOR.max((n_faces as f64 * MIN_REGION_FRACTION).ceil() as u32))
        .min(((n_faces as f64 / 20.0).floor() as u32).max(1))
        .max(1)
}

/// Per-face 2-D intrinsic feature used for clustering and for every merge
/// decision: (curvature, thickness), each normalized to [0,1] on an **absolute**
/// scale so the numbers mean the same thing across models.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Feature {
    pub curv: f32,
    pub thick: f32,
}

/// log-normalize a thickness field to [0,1].
///
/// The `MIN_LOG_SPAN` floor is load-bearing, not defensive padding. Plain
/// min-max normalization of a *uniform* field (a sphere has the same thickness
/// everywhere) divides float noise by a near-zero span and stretches it across
/// the full [0,1] range — k-means then happily "discovers" structure in 1e-3
/// ray-sampling jitter. Flooring the span means a mesh whose thickness varies
/// by less than ~5% collapses to a near-constant feature, which is the truth.
pub(crate) fn log_normalize(values: &[f32]) -> Vec<f32> {
    const MIN_LOG_SPAN: f32 = 0.05; // ln-space ≈ 5% thickness variation
    let logs: Vec<f32> = values.iter().map(|v| v.max(1e-4).ln()).collect();
    let mn = logs.iter().cloned().fold(f32::INFINITY, f32::min);
    let mx = logs.iter().cloned().fold(f32::NEG_INFINITY, f32::max);
    let span = (mx - mn).max(MIN_LOG_SPAN);
    logs.iter()
        .map(|l| ((l - mn) / span).clamp(0.0, 1.0))
        .collect()
}

/// Per-face curvature proxy: mean unsigned dihedral angle to adjacent faces,
/// rescaled by an absolute 90° full-scale rather than by the mesh maximum.
///
/// Dividing by the mesh maximum is the same noise-amplification trap as min-max
/// on SDF: on a smooth model the largest local dihedral may be 3°, and stretching
/// that to 1.0 turns tessellation jitter into the dominant clustering axis.
///
/// Each face reads only `normals` + the adjacency graph and writes its own
/// slot, so the per-face map parallelises losslessly (rayon indexed collect
/// keeps face order and per-face float ops unchanged).
pub(crate) fn face_curvature(mesh: &MeshModel, normals: &[[f32; 3]]) -> Vec<f32> {
    let n = mesh.faces.len();
    (0..n)
        .into_par_iter()
        .map(|fi| {
            let node = petgraph::graph::NodeIndex::new(fi);
            let mut sum = 0.0f32;
            let mut cnt = 0u32;
            let ni = normals[fi];
            for edge in mesh.face_adjacency.edges(node) {
                let nb = if edge.source() == node {
                    edge.target()
                } else {
                    edge.source()
                };
                let nj = normals[nb.index()];
                let dot = (ni[0] * nj[0] + ni[1] * nj[1] + ni[2] * nj[2]).clamp(-1.0, 1.0);
                sum += dot.acos();
                cnt += 1;
            }
            if cnt > 0 {
                (sum / cnt as f32 / CURV_FULL_SCALE).min(1.0)
            } else {
                0.0
            }
        })
        .collect()
}

/// Assemble features from a curvature field and an optional pre-normalized
/// thickness field (pass `None` to zero the thickness axis).
pub(crate) fn assemble_features(curv: &[f32], thick: Option<&[f32]>) -> Vec<Feature> {
    (0..curv.len())
        .map(|i| Feature {
            curv: curv[i],
            thick: thick.map(|t| t[i]).unwrap_or(0.0),
        })
        .collect()
}

/// Split each label into its connected components over face adjacency, so every
/// output label is a single connected patch (REFUTE blocker #2).
///
/// When `crease` is `Some((normals, threshold_deg))` the traversal additionally
/// refuses to cross a real crease, which makes the output the **intersection of
/// the feature partition and the crease partition**.
///
/// That intersection is not a refinement — it is the fix for a structural blind
/// spot. Clustering assigns a label per face from a *field*, and merging can
/// only ever remove borders, so any boundary the field did not encode is
/// unreachable. A cube is the clean counter-example: every one of its twelve
/// triangles has the identical mean dihedral ((0°+90°+90°)/3), the field is
/// perfectly uniform, k-means returns one cluster, and no amount of merging can
/// recover the six faces a human sees. The information is in the *edges*, not in
/// the faces, so the edges have to be allowed to cut.
pub(crate) fn split_connected_components(
    mesh: &MeshModel,
    labels: &[u32],
    crease: Option<(&[[f32; 3]], f32)>,
) -> Vec<u32> {
    let n = mesh.faces.len();
    let mut out = vec![u32::MAX; n];
    let mut next = 0u32;
    for start in 0..n {
        if out[start] != u32::MAX {
            continue;
        }
        let lab = labels[start];
        let mut q = vec![start as u32];
        out[start] = next;
        while let Some(cur) = q.pop() {
            let node = petgraph::graph::NodeIndex::new(cur as usize);
            for edge in mesh.face_adjacency.edges(node) {
                let nb = if edge.source() == node {
                    edge.target()
                } else {
                    edge.source()
                };
                let ni = nb.index();
                if labels[ni] != lab || out[ni] != u32::MAX {
                    continue;
                }
                if let Some((normals, thr)) = crease {
                    if crease_strength_deg(mesh, normals, cur as usize, ni) >= thr {
                        continue; // a real feature edge — do not grow across it
                    }
                }
                out[ni] = next;
                q.push(ni as u32);
            }
        }
        next += 1;
    }
    out
}

/// Signed crease strength (in degrees) across the shared edge of two adjacent
/// faces, weighted by the *minima rule*.
///
/// Sign test: with outward normals, the edge is concave when the neighbour's
/// centroid lies on the *front* side of this face's plane, i.e. (c_j-c_i)·n_i > 0.
/// A cube's exterior edge gives < 0 (convex); the inner corner of an L-bracket
/// gives > 0 (concave). `acos(dot)` alone cannot tell these apart, which is why
/// the SDF merge's bare 0.93 dot was left as-is rather than reused here.
pub(crate) fn crease_strength_deg(
    mesh: &MeshModel,
    normals: &[[f32; 3]],
    fi: usize,
    fj: usize,
) -> f32 {
    let ni = normals[fi];
    let nj = normals[fj];
    let dot = (ni[0] * nj[0] + ni[1] * nj[1] + ni[2] * nj[2]).clamp(-1.0, 1.0);
    let ang = dot.acos().to_degrees();
    let ci = mesh.face_center(fi as u32);
    let cj = mesh.face_center(fj as u32);
    let d = [cj[0] - ci[0], cj[1] - ci[1], cj[2] - ci[2]];
    let concave = d[0] * ni[0] + d[1] * ni[1] + d[2] * ni[2] > 0.0;
    if concave {
        ang
    } else {
        ang * CONVEX_CREASE_WEIGHT
    }
}

/// Dissolve region boundaries that do not lie on a real crease.
///
/// This is the step that turns "clustering of a continuous field" into "feature
/// recognition". k-means on a smooth surface always returns k clusters, so a
/// sphere comes back sliced into latitude rings — each ring is a perfectly valid
/// cluster and a completely fake part. A part boundary has to be visible in the
/// geometry, so any two adjacent regions whose shared border is smooth get
/// merged back, no matter how different their feature centroids are... except
/// when the centroids are far apart ([`FEATURE_GAP_KEEP`]), which is the
/// thin-plate-blended-into-thick-block case where SDF genuinely found a part
/// that has no sharp border.
pub(crate) fn merge_smooth_boundaries(
    mesh: &MeshModel,
    labels: &[u32],
    feats: &[Feature],
    normals: &[[f32; 3]],
    crease_threshold_deg: f32,
) -> Vec<u32> {
    let n = labels.len();
    let nreg = (*labels.iter().max().unwrap_or(&0) + 1) as usize;
    if nreg <= 1 {
        return labels.to_vec();
    }

    let mut parent: Vec<u32> = (0..nreg as u32).collect();
    let find = |mut x: u32, parent: &[u32]| -> u32 {
        while parent[x as usize] != x {
            x = parent[x as usize];
        }
        x
    };

    for _pass in 0..MAX_MERGE_PASSES {
        // Boundary statistics are recomputed per pass against the *current*
        // roots: after A and B merge, the border of (A∪B) with C is the union of
        // the A-C and B-C borders, and its mean strength is not the mean of the
        // two — recomputing is the only way to keep the criterion honest.
        let mut acc: HashMap<(u32, u32), (f64, u32)> = HashMap::new();
        for edge in mesh.face_adjacency.edge_references() {
            let fi = mesh.face_adjacency[edge.source()] as usize;
            let fj = mesh.face_adjacency[edge.target()] as usize;
            let (ra, rb) = (find(labels[fi], &parent), find(labels[fj], &parent));
            if ra == rb {
                continue;
            }
            let key = if ra < rb { (ra, rb) } else { (rb, ra) };
            let s = crease_strength_deg(mesh, normals, fi, fj) as f64;
            let e = acc.entry(key).or_insert((0.0, 0));
            e.0 += s;
            e.1 += 1;
        }
        if acc.is_empty() {
            break;
        }

        let mut sizes = vec![0u32; nreg];
        let mut cen = vec![(0.0f64, 0.0f64); nreg];
        for i in 0..n {
            let r = find(labels[i], &parent) as usize;
            sizes[r] += 1;
            cen[r].0 += feats[i].curv as f64;
            cen[r].1 += feats[i].thick as f64;
        }
        for r in 0..nreg {
            if sizes[r] > 0 {
                cen[r].0 /= sizes[r] as f64;
                cen[r].1 /= sizes[r] as f64;
            }
        }

        // Weakest boundary first; ties broken by region id so HashMap iteration
        // order never leaks into the result.
        let mut cands: Vec<((u32, u32), f64)> = acc
            .into_iter()
            .map(|(k, (sum, cnt))| (k, sum / cnt as f64))
            .filter(|&((a, b), mean)| {
                if mean >= crease_threshold_deg as f64 {
                    return false;
                }
                // Coplanar ⇒ always dissolve, feature gap or not.
                if mean < COPLANAR_EPS_DEG {
                    return true;
                }
                let dx = cen[a as usize].0 - cen[b as usize].0;
                let dy = cen[a as usize].1 - cen[b as usize].1;
                (dx * dx + dy * dy).sqrt() <= FEATURE_GAP_KEEP
            })
            .collect();
        if cands.is_empty() {
            break;
        }
        cands.sort_by(|x, y| {
            x.1.partial_cmp(&y.1)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then(x.0.cmp(&y.0))
        });

        let mut merged = 0usize;
        for ((a, b), _) in cands {
            let (ra, rb) = (find(a, &parent), find(b, &parent));
            if ra == rb {
                continue;
            }
            let (lo, hi) = if sizes[ra as usize] <= sizes[rb as usize] {
                (ra, rb)
            } else {
                (rb, ra)
            };
            parent[lo as usize] = hi;
            merged += 1;
        }
        if merged == 0 {
            break;
        }
    }

    compact(labels, &parent, find)
}

/// Merge small components into the adjacent component with the closest feature
/// centroid. Keyed on feature distance, not on a raw normal dot.
pub(crate) fn merge_small_by_feature(
    mesh: &MeshModel,
    split: &[u32],
    feats: &[Feature],
) -> Vec<u32> {
    let n = mesh.faces.len();
    let ncomp = (*split.iter().max().unwrap_or(&0) + 1) as usize;

    let min_faces = min_region_faces(n);

    // Component adjacency (undirected), deduped.
    let mut comp_adj: HashMap<u32, HashMap<u32, u32>> = HashMap::new();
    for edge in mesh.face_adjacency.edge_references() {
        let fi = mesh.face_adjacency[edge.source()];
        let fj = mesh.face_adjacency[edge.target()];
        let (a, b) = (split[fi as usize], split[fj as usize]);
        if a != b {
            *comp_adj.entry(a).or_default().entry(b).or_insert(0) += 1;
            *comp_adj.entry(b).or_default().entry(a).or_insert(0) += 1;
        }
    }

    let mut parent: Vec<u32> = (0..ncomp as u32).collect();
    let find = |mut x: u32, parent: &[u32]| -> u32 {
        while parent[x as usize] != x {
            x = parent[x as usize];
        }
        x
    };

    // Batch merge. An earlier shape of this loop absorbed exactly ONE component
    // per iteration, so with a 20-iteration budget it could only ever remove 20
    // crumbs. k-means over a noisy curvature field routinely shatters into tens
    // of thousands of connected components, which that loop would have handed to
    // the UI verbatim — unusable, and close enough to MANUAL_SEGMENT_OFFSET to be
    // a namespace hazard. Each pass now absorbs *every* under-sized root, and if
    // the region count is still above MAX_AUTO_REGIONS the size bar doubles so
    // the next pass eats more.
    let threshold = min_faces;
    for _pass in 0..MAX_MERGE_PASSES {
        let mut cur_size = vec![0u32; ncomp];
        let mut cur_cen = vec![(0.0f32, 0.0f32); ncomp];
        for i in 0..n {
            let r = find(split[i], &parent);
            cur_size[r as usize] += 1;
            cur_cen[r as usize].0 += feats[i].curv;
            cur_cen[r as usize].1 += feats[i].thick;
        }
        let cur_cen: Vec<(f32, f32)> = cur_cen
            .iter()
            .enumerate()
            .map(|(r, s)| {
                if cur_size[r] > 0 {
                    (s.0 / cur_size[r] as f32, s.1 / cur_size[r] as f32)
                } else {
                    (0.0, 0.0)
                }
            })
            .collect();

        let roots: Vec<u32> = (0..ncomp as u32).filter(|&r| find(r, &parent) == r).collect();
        let mut live = roots.len();
        let too_many = live > MAX_AUTO_REGIONS;
        // When over the cap every root is fair game, not just the under-sized
        // ones — otherwise a mesh shattered into uniformly medium components
        // would never shrink. Each such pass roughly halves the root count, so
        // 50k components reach the 4096 cap in ~4 passes.
        let pass_threshold = if too_many { u32::MAX } else { threshold };

        // Smallest first, ties broken by id → order is independent of HashMap
        // iteration order, so the whole merge stays deterministic.
        let mut order: Vec<u32> = roots
            .into_iter()
            .filter(|&r| cur_size[r as usize] < pass_threshold)
            .collect();
        if order.is_empty() && !too_many {
            break;
        }
        order.sort_by_key(|&r| (cur_size[r as usize], r));

        let mut merged = 0usize;
        for &r in &order {
            if live <= 1 {
                break;
            }
            if find(r, &parent) != r {
                continue; // absorbed earlier in this same pass
            }
            let mut best_nb: Option<u32> = None;
            let mut best_d = f32::INFINITY;
            if let Some(neigh) = comp_adj.get(&r) {
                let mut neigh_ids: Vec<u32> = neigh.keys().cloned().collect();
                neigh_ids.sort_unstable();
                for nb in neigh_ids {
                    let nr = find(nb, &parent);
                    if nr == r {
                        continue;
                    }
                    let dx = cur_cen[r as usize].0 - cur_cen[nr as usize].0;
                    let dy = cur_cen[r as usize].1 - cur_cen[nr as usize].1;
                    let d = dx * dx + dy * dy;
                    if d < best_d {
                        best_d = d;
                        best_nb = Some(nr);
                    }
                }
            }
            // No neighbour at all = an isolated island; leaving it alone is
            // correct (it really is its own part), and it cannot loop forever
            // because we never revisit it within this pass.
            if let Some(nr) = best_nb {
                let (lo, hi) = if cur_size[r as usize] <= cur_size[nr as usize] {
                    (r, nr)
                } else {
                    (nr, r)
                };
                parent[lo as usize] = hi;
                merged += 1;
                live -= 1;
            }
        }

        // Nothing merged means the union-find is unchanged, so no future pass
        // can make progress either — including the over-cap case, where every
        // remaining root is an island with no mergeable neighbour.
        if merged == 0 {
            break;
        }
    }

    let out = compact(split, &parent, find);
    // A comment is not a guard (REFUTE blocker #5). Labels ≥ 100_000 are the
    // manual-segment namespace; if the merge loop ever failed to converge we
    // must find out here rather than by silently colliding with a lasso region.
    let regions = out.iter().copied().max().map(|m| m + 1).unwrap_or(0);
    debug_assert!(
        regions < crate::mesh::model::MANUAL_SEGMENT_OFFSET,
        "auto labels breached the manual namespace: {} regions",
        regions
    );
    if regions as usize > MAX_AUTO_REGIONS {
        log::warn!(
            "[segment] {} regions survived merging (cap {}) — mesh is highly fragmented",
            regions,
            MAX_AUTO_REGIONS
        );
    }
    out
}

/// Relabel to contiguous ids in first-appearance order (deterministic).
fn compact<F>(labels: &[u32], parent: &[u32], find: F) -> Vec<u32>
where
    F: Fn(u32, &[u32]) -> u32,
{
    let mut remap: HashMap<u32, u32> = HashMap::new();
    let mut next = 0u32;
    let mut out = vec![0u32; labels.len()];
    for (i, &l) in labels.iter().enumerate() {
        let r = find(l, parent);
        let id = *remap.entry(r).or_insert_with(|| {
            let id = next;
            next += 1;
            id
        });
        out[i] = id;
    }
    out
}

/// The full post-processing pipeline.
///
/// `crease_deg = Some(t)` runs the crease-aware form: cut along feature edges,
/// then dissolve every border that turned out not to be one. Cutting first and
/// merging after is what makes the result independent of how the clustering
/// happened to fall — the crease partition supplies the boundaries the field
/// cannot see, and the merge removes the boundaries the field invented.
///
/// `crease_deg = None` runs plain connectivity + crumb cleanup, which is what
/// SDF wants: its own convex-boundary merge already ran, and its thickness
/// clusters exist precisely to survive smooth borders that a curvature rule
/// would dissolve.
pub(crate) fn refine_regions(
    mesh: &MeshModel,
    labels: &[u32],
    feats: &[Feature],
    normals: &[[f32; 3]],
    crease_deg: Option<f32>,
) -> Vec<u32> {
    let crease = crease_deg.map(|deg| (normals, deg));
    let split = split_connected_components(mesh, labels, crease);
    let merged = merge_small_by_feature(mesh, &split, feats);
    match crease_deg {
        Some(deg) => merge_smooth_boundaries(mesh, &merged, feats, normals, deg),
        None => merged,
    }
}

/// Write labels into the mesh and rebuild `mesh.segments`. Every algorithm ends
/// here so the metadata shape (name, ordering, face counts) is identical.
///
/// The counting/naming used to be duplicated here (and a third time in
/// `dihedral.rs`) instead of delegating to `MeshModel::rebuild_segments`. Three
/// copies of "what a Segment looks like" is three places to forget: the local
/// copies hardcoded `color: None` and a `Region {id+1}` name, so a manual label
/// surviving into this path would have come back uncoloured, and any
/// user-supplied name would have been silently discarded. Delegating makes
/// `rebuild_segments` the only place that decides segment metadata.
pub(crate) fn finalize_segments(mesh: &mut MeshModel, labels: Vec<u32>) -> Vec<Segment> {
    mesh.segment_labels = labels;
    mesh.rebuild_segments();
    mesh.sorted_segments()
}
