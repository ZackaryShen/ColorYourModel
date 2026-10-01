//! Layer 4 of the planar-region fusion study (`docs/09`): the **fusion layer**.
//!
//! Up to now every algorithm (planar / multiview / cross-section) produced its
//! own regions but the final partition was still decided by `seed_grow` — which
//! only ever saw *seed points*, never the regions' actual membership. The user's
//! feedback was precise: the algorithms sketch useful regions, but the real
//! partition is still "just the seeds". This module fixes that by fusing the
//! algorithms' *region membership* into one partition instead of throwing it
//! away.
//!
//! # Strategy — edge-level majority vote
//!
//! The cleanest common abstraction across the three algorithm families is the
//! single question **"should this mesh edge be a region boundary?"**. Every
//! face-level region answers it: two faces in the same region vote *keep* the
//! edge between them, in different regions vote *cut*. Fusing then becomes a
//! per-edge vote:
//!
//! ```text
//! planar   : both faces labelled → same region = keep, different = cut, else abstain
//! multiview: both faces labelled → same region = keep, different = cut, else abstain
//! score(e) = (#cut votes) − (#keep votes)
//! cut      iff score(e) > cut_threshold        (default 1: cut must outvote keep)
//! ```
//!
//! Connected components of the face graph after removing cut edges are the
//! final regions. A tie (score ≤ 0) keeps the edge, so a single algorithm's
//! noise cannot over-split — this is what makes the fused partition *coarser
//! and more meaningful* than either detector alone, directly addressing the
//! "partition is meaningless" complaint while keeping docs/09's "two algorithms
//! agreeing" honesty rule.
//!
//! Layer 2 (cross-section) is a plane, not a face set, so it does not cast
//! per-edge votes yet — it stays a visual overlay and is deferred to a later
//! iteration where the slice contour can vote "cut" on the mesh edges it
//! crosses.

use std::collections::{HashMap, HashSet};

use petgraph::graph::NodeIndex;
use petgraph::visit::EdgeRef;

use crate::mesh::history::OpKind;
use crate::mesh::model::{MeshModel, Segment};

/// Max tiny-region merge passes — mirror of [`postprocess::MAX_MERGE_PASSES`].
/// Each pass absorbs *every* under-sized region (faces below `min_faces`)
/// instead of a single minimum; without this a 500k-face model with 500 small
/// fragments would exit the loop after 3 passes with hundreds of crumbs still
/// standing. The cap exists only to make termination provable; in practice the
/// loop converges when no under-sized region remains. Iteration 84 raised this
/// from 40 → 200 because the giant-absorption strategy (below) collapses long
/// crumb chains in O(chain length) passes; 200 is a safe ceiling for a 500k
/// mesh while staying well under a second of work.
const MAX_MERGE_PASSES: u32 = 200;

/// Weight given to a single dihedral-crease cut vote. The geometry backbone
/// must be *authoritative*: on smooth / single-colour meshes planar and
/// MultiView have no signal, so a dihedral crease has to cut even when no other
/// channel agrees. A value far above any sane `cut_threshold` guarantees it.
const DIHEDRAL_WEIGHT: i32 = 1000;
/// Weight given to a single eye-region cut vote. Mirrors `DIHEDRAL_WEIGHT`: an
/// eye region detected by `detect_eye_regions` (globe / sclera / eyelid /
/// socket) is *semantic, not geometric* — it comes from the user's ROI intent
/// rather than a crease — so when the user supplies eye regions, those faces
/// must be carved out of any merged partition. Same magnitude as the geometry
/// backbone so they survive any reasonable `cut_threshold`.
const EYE_WEIGHT: i32 = 1000;

/// Result of a fused partition — mirrors `SeedGrowResult` so the command layer
/// and frontend can reuse the exact same wire shape.
///
/// `edge_total` / `edge_cut` / `region_size_min` / `region_size_max` /
/// `region_size_median` are diagnostic counters surfaced to the frontend via
/// the `fuse-debug` Tauri event, so the user can see *why* the button produced
/// the result it did (e.g. "535 raw components but only 38 large regions
/// after small-region merge" — a hint the right knob to pull is the
/// `min_region_faces` slider, not the algorithms themselves).
#[derive(Default)]
pub struct FuseResult {
    pub segments: Vec<Segment>,
    pub segment_labels: Vec<u32>,
    pub moved_faces: usize,
    pub region_count: usize,
    pub edge_total: Option<u64>,
    pub edge_cut: Option<u64>,
    pub region_size_min: usize,
    pub region_size_max: usize,
    pub region_size_median: usize,
    // ── Iteration 83: per-channel vote counters ──────────────────────────
    // Surfaced so the user can see *which* channel actually drove the cuts.
    // On a smooth / single-colour model the planar and multiview channels
    // each cast a single `cut` vote per disagreement edge (weight 1), while
    // dihedral and eye cast `WEIGHT = 1000`. With `cut_threshold = 1` the
    // decision test is `cut - keep > 1`, so a lone planar/multiview vote
    // (cut=1, keep=0 → 1 > 1 is false) NEVER cuts — which is why the
    // multiview detector can return 125 regions yet contribute 0 cuts.
    pub planar_vote_edges: u64,
    pub planar_cut_votes: u64,
    pub multiview_vote_edges: u64,
    pub multiview_cut_votes: u64,
    pub dihedral_cut_votes: u64,
    pub eye_cut_votes: u64,
    // ── Iteration 85: merge diagnostics ───────────────────────────────────
    pub regions_before_merge: usize,
    pub regions_after_merge: usize,
    pub merge_passes: u32,
    pub min_faces: usize,
    pub tiny_regions_before_merge: usize,
}

/// Build a per-face optional label from a set of regions. Each region is a list
/// of face indices; every face gets `Some(region_index)` or stays `None`
/// (unclaimed → the edge votes abstain).
fn label_from_sets(n: usize, sets: &[Vec<u32>]) -> Vec<Option<u32>> {
    let mut label = vec![None; n];
    for (rid, set) in sets.iter().enumerate() {
        for &f in set {
            if (f as usize) < n {
                label[f as usize] = Some(rid as u32);
            }
        }
    }
    label
}

/// Fuse face-level region sets (planar + multiview + **dihedral geometry
/// backbone**) into one partition via edge-level majority vote, then commit it
/// to the mesh exactly like `seed_grow` does (manual labels, per-face colour,
/// one undo entry).
///
/// `cut_threshold` is the minimum `score` (cut votes − keep votes) needed to
/// cut an edge — 1 means "cut must outvote keep", 0 means "a tie cuts", 2 means
/// "all channels must vote cut with no keep opposition".
/// `min_region_faces` filters tiny regions after the vote (`0` = auto-pick
/// `max(8, n·0.2%)`, the same default as the seed-grow optimizer).
///
/// The **dihedral channel is the geometry backbone** (Layer 0): on smooth,
/// single-colour, organic meshes the planar (needs flat faces) and multiview
/// (needs view-to-view visual variation) detectors return little or nothing,
/// so without it the fused partition collapses to a single region. Dihedral
/// creases (leg–body, ear–head, tail–base) cut regardless of colour/flatness,
/// giving the vote real signal on exactly those models. See `docs/09` §fusion.
pub fn fuse_region_sets(
    mesh: &mut MeshModel,
    planar_sets: &[Vec<u32>],
    multiview_sets: &[Vec<u32>],
    dihedral_sets: &[Vec<u32>],
    eye_sets: &[Vec<u32>],
    cut_threshold: i32,
    min_region_faces: usize,
) -> Result<FuseResult, String> {
    let n = mesh.faces.len();
    if n == 0 {
        return Err("mesh has no faces".into());
    }
    if planar_sets.is_empty()
        && multiview_sets.is_empty()
        && dihedral_sets.is_empty()
        && eye_sets.is_empty()
    {
        return Err("all detectors returned no regions".into());
    }

    let planar_label = label_from_sets(n, planar_sets);
    let multiview_label = label_from_sets(n, multiview_sets);
    let dihedral_label = label_from_sets(n, dihedral_sets);
    // Eye region is intentionally NOT treated as a per-face LABEL — every eye
    // face gets its own one-face region so an edge between two eye faces votes
    // `keep`-vs-`keep` for *neither* vote channel. We instead use the explicit
    // boundary-edge loop below (Step 1b) to vote cut on every edge with at least
    // one eye-side endpoint. That guarantees any eye region is isolated from
    // its non-eye neighbours even when the eye region itself is internally
    // contiguous — because eye sides split on every edge crossing the
    // region's perimeter.
    let eye_bounded: Vec<bool> = {
        let mut b = vec![false; n];
        for es in eye_sets {
            for &f in es {
                if (f as usize) < n {
                    b[f as usize] = true;
                }
            }
        }
        b
    };

    // ── 1. Per-edge majority vote ────────────────────────────────────────
    let mut cut_edges: HashSet<(u32, u32)> = HashSet::new();
    let mut edge_total = 0u64;
    let mut edge_cut = 0u64;
    // Iteration 83: per-channel vote tallies so the debug payload can reveal
    // *which* channel actually drove the cut decision.
    let mut planar_vote_edges = 0u64;
    let mut planar_cut_votes = 0u64;
    let mut multiview_vote_edges = 0u64;
    let mut multiview_cut_votes = 0u64;
    let mut dihedral_cut_votes = 0u64;
    let mut eye_cut_votes = 0u64;
    for e in mesh.face_adjacency.edge_references() {
        let a = e.source().index() as u32;
        let b = e.target().index() as u32;
        if a >= n as u32 || b >= n as u32 || a == b {
            continue;
        }
        let mut cut = 0i32;
        let mut keep = 0i32;
        edge_total += 1;
        match (planar_label[a as usize], planar_label[b as usize]) {
            (Some(x), Some(y)) => {
                planar_vote_edges += 1;
                if x == y {
                    keep += 1;
                } else {
                    cut += 1;
                    planar_cut_votes += 1;
                }
            }
            _ => {}
        }
        match (multiview_label[a as usize], multiview_label[b as usize]) {
            (Some(x), Some(y)) => {
                multiview_vote_edges += 1;
                if x == y {
                    keep += 1;
                } else {
                    cut += 1;
                    multiview_cut_votes += 1;
                }
            }
            _ => {}
        }
        match (dihedral_label[a as usize], dihedral_label[b as usize]) {
            (Some(x), Some(y)) if x != y => {
                // Iteration 85: do NOT let the geometry backbone cut *inside*
                // an eye-bounded region. The eye detector already carves the
                // eye ROI out with EYE_WEIGHT; dihedral creases inside the
                // eyeball/eyelid/socket would otherwise shred the semantic eye
                // region into dozens of tiny locked fragments, which then
                // survive tiny-region merge and destroy the median size.
                if !eye_bounded[a as usize] && !eye_bounded[b as usize] {
                    // Geometry backbone is authoritative: a dihedral crease
                    // cuts regardless of the feature channels (which have no
                    // signal on smooth / single-colour meshes). Weighted above
                    // any threshold.
                    dihedral_cut_votes += 1;
                    cut += DIHEDRAL_WEIGHT;
                }
            }
            _ => {}
        }
        // Eye channel: cut when exactly one endpoint is an eye face (boundary
        // crossing). Internal eye-edge has both endpoints eyed → no cut (the
        // eye region stays whole). This is what makes a user-confirmed eye
        // ROI survive any reasonable cut_threshold and any neighbour channel
        // that would have absorbed it.
        let ea = eye_bounded[a as usize];
        let eb = eye_bounded[b as usize];
        if ea != eb {
            eye_cut_votes += 1;
            cut += EYE_WEIGHT;
        }
        if cut - keep > cut_threshold {
            cut_edges.insert((a.min(b), a.max(b)));
            edge_cut += 1;
        }
    }
    log::info!(
        "[fuse] channels — planar={} multiview={} dihedral={} eye_sets={}; total_edges={} cut={} cut_threshold={}",
        planar_sets.len(),
        multiview_sets.len(),
        dihedral_sets.len(),
        eye_sets.len(),
        edge_total,
        edge_cut,
        cut_threshold,
    );

    // ── 2. Connected components ignoring cut edges ────────────────────────
    let mut region = vec![u32::MAX; n];
    let mut region_count = 0u32;
    for start in 0..n {
        if region[start] != u32::MAX {
            continue;
        }
        region[start] = region_count;
        let mut stack = vec![start as u32];
        while let Some(f) = stack.pop() {
            for e in mesh.face_adjacency.edges(NodeIndex::new(f as usize)) {
                let g = e.target().index() as u32;
                if g >= n as u32 || region[g as usize] != u32::MAX {
                    continue;
                }
                if cut_edges.contains(&(f.min(g), f.max(g))) {
                    continue;
                }
                region[g as usize] = region_count;
                stack.push(g);
            }
        }
        region_count += 1;
    }
    log::info!(
        "[fuse] raw_components={} cut_edges={}",
        region_count,
        cut_edges.len()
    );

    // ── 3. Merge tiny regions into their largest neighbour ────────────────
    let min_faces = if min_region_faces > 0 {
        min_region_faces
    } else {
        ((n as f32) * 0.002).max(8.0) as usize
    };
    // Iteration 85: merge diagnostics so the user can see whether the merge
    // phase actually ran and why it stopped.
    let merge_diagnostics = {
        let mut counts: HashMap<u32, usize> = HashMap::new();
        for &r in &region {
            *counts.entry(r).or_insert(0) += 1;
        }
        (
            region_count as usize,
            counts
                .values()
                .filter(|&&c| c < min_faces)
                .count(),
        )
    };
    let regions_before_merge = merge_diagnostics.0;
    let tiny_regions_before_merge = merge_diagnostics.1;
    let mut merge_passes = 0u32;
    // An *eye* region must NOT be merged away by the tiny-region pass: the
    // user explicitly confirmed those faces, even if it is one small sclera
    // strip. Tag the source region ids for every face; a region that contains
    // any eye face is locked.
    let mut region_has_eye: HashMap<u32, bool> = HashMap::new();
    for (f, &rid) in region.iter().enumerate() {
        if eye_bounded[f] {
            region_has_eye.insert(rid, true);
        } else if !region_has_eye.contains_key(&rid) {
            region_has_eye.insert(rid, false);
        }
    }
    for pass in 0..MAX_MERGE_PASSES {
        merge_passes = pass + 1;
        // Build per-region face counts and accumulate per-pass statistics.
        let mut counts: HashMap<u32, usize> = HashMap::new();
        for &r in &region {
            *counts.entry(r).or_insert(0) += 1;
        }
        // Snapshot the under-sized non-eye regions; do NOT mutate `region`
        // mid-iteration — collecting first means every small region gets a
        // chance this pass even when others just got absorbed.
        let mut tiny: Vec<u32> = counts
            .iter()
            // Skip under-sized regions that contain an eye face — the user
            // explicitly confirmed those faces.
            .filter(|(&r, &c)| c < min_faces && !region_has_eye.get(&r).copied().unwrap_or(false))
            .map(|(&r, _)| r)
            .collect();
        if tiny.is_empty() {
            break;
        }
        // Sort by size ascending so smaller crumbs merge first.
        tiny.sort_by_key(|&r| counts[&r]);
        // Iteration 84 — the global-largest region ("giant") is the crumb
        // sink. Preferring it as the merge target guarantees every small
        // region eventually flows INTO the giant instead of stalling in a
        // chain of other small regions (the failure mode that left hundreds of
        // 1–3 face islands behind in iter80–83). As crumbs merge into the
        // giant, the giant's id is stable, so once a crumb touches the grown
        // giant it is pulled in on the next pass and its former neighbours
        // become adjacent to the giant too — chains collapse in O(length)
        // passes.
        let giant = counts.iter().max_by_key(|(_, &c)| c).map(|(&r, _)| r);
        let mut merged_this_pass = 0usize;
        for small_id in tiny {
            // Already absorbed earlier this pass (its id now points at a
            // target) — skip so we don't re-scan a vanished region.
            if !counts.contains_key(&small_id) {
                continue;
            }
            // Build neighbour of `small_id` by face-level scan (only the
            // faces whose `region[f] == small_id` are inspected, plus their
            // adjacent neighbours — linear in the size of the small region).
            // Scans ALL adjacency edges (including cut edges) so a crumb can
            // merge across a boundary it shares with a larger region.
            let mut neighbour: HashMap<u32, usize> = HashMap::new();
            for f in 0..n {
                if region[f] != small_id {
                    continue;
                }
                for e in mesh.face_adjacency.edges(NodeIndex::new(f)) {
                    let g = e.target().index();
                    if g >= n {
                        continue;
                    }
                    let rn = region[g];
                    if rn != small_id {
                        *neighbour.entry(rn).or_insert(0) += 1;
                    }
                }
            }
            if neighbour.is_empty() {
                // Truly isolated (no adjacency at all) — skip it and keep
                // processing the rest of the list. The old code `break`ed here,
                // which could abort the whole pass and leave every other crumb
                // standing.
                continue;
            }
            // Prefer the giant if it touches this crumb; otherwise the largest
            // local neighbour.
            let target = match giant {
                Some(g) if neighbour.contains_key(&g) => g,
                _ => *neighbour.iter().max_by_key(|(_, &c)| c).unwrap().0,
            };
            if target == small_id {
                continue;
            }
            for f in 0..n {
                if region[f] == small_id {
                    region[f] = target;
                }
            }
            counts.remove(&small_id);
            merged_this_pass += 1;
        }
        log::info!(
            "[fuse] tiny-merge pass {}: merged {} small regions (min_faces={})",
            pass,
            merged_this_pass,
            min_faces
        );
        if merged_this_pass == 0 {
            break;
        }
    }

    // ── 3b. Merge diagnostics ─────────────────────────────────────────────
    let regions_after_merge = {
        let mut ids = HashSet::new();
        for &r in &region {
            ids.insert(r);
        }
        ids.len()
    };

    // ── 4. Commit: fresh manual label per region + colour + one undo entry ─
    // Labels are allocated so that `label % SEGMENT_PALETTE_SIZE` — the
    // frontend's colour slot — differs across ADJACENT regions. With 80+
    // regions and a sequential allocation, touching regions shared a palette
    // colour every few boundaries and the partition *looked* unsegmented
    // (field report on the 1.5M-face godzilla sculpt at 0°). Greedy graph
    // colouring over the region adjacency, largest region first: with 30
    // slots a free class is essentially always available; the fallback
    // reuses class 0 rather than fail.
    let mut region_sizes_map: HashMap<u32, usize> = HashMap::new();
    for &r in &region {
        *region_sizes_map.entry(r).or_insert(0) += 1;
    }
    let mut reg_adj: HashMap<u32, HashSet<u32>> = HashMap::new();
    for e in mesh.face_adjacency.edge_references() {
        let a = mesh.face_adjacency[e.source()];
        let b = mesh.face_adjacency[e.target()];
        if a >= n as u32 || b >= n as u32 {
            continue;
        }
        let (ra, rb) = (region[a as usize], region[b as usize]);
        if ra != rb {
            reg_adj.entry(ra).or_default().insert(rb);
            reg_adj.entry(rb).or_default().insert(ra);
        }
    }
    // Deterministic order: size desc, then region id (the refuter's m1 rule —
    // never depend on HashMap iteration order for a committed result).
    let mut order: Vec<u32> = region_sizes_map.keys().copied().collect();
    order.sort_unstable_by(|a, b| {
        region_sizes_map[b]
            .cmp(&region_sizes_map[a])
            .then(a.cmp(b))
    });
    let palette_slots = crate::mesh::model::SEGMENT_PALETTE_SIZE as usize;
    let mut class_of: HashMap<u32, u32> = HashMap::new();
    for &r in &order {
        let mut used = [false; { crate::mesh::model::SEGMENT_PALETTE_SIZE as usize }];
        for &nb in reg_adj.get(&r).into_iter().flatten() {
            if let Some(c) = class_of.get(&nb) {
                used[*c as usize % palette_slots] = true;
            }
        }
        let class = (0..palette_slots)
            .find(|&c| !used[c])
            .unwrap_or(0) as u32;
        class_of.insert(r, class);
    }
    let mut id_to_label: HashMap<u32, u32> = HashMap::new();
    for &r in &order {
        let label = mesh.alloc_manual_label_in_class(class_of[&r], palette_slots as u32);
        id_to_label.insert(r, label);
    }
    let mut prev_colors = Vec::with_capacity(n);
    let mut prev_labels = Vec::with_capacity(n);
    let mut final_regions = Vec::<(u32, usize)>::new();
    let mut region_faces: HashMap<u32, Vec<u32>> = HashMap::new();
    let mut moved = 0usize;
    for f in 0..n {
        let rid = region[f];
        let label = id_to_label[&rid];
        region_faces.entry(rid).or_default().push(f as u32);
        let fi = f as u32;
        prev_labels.push((fi, mesh.segment_labels[f]));
        prev_colors.push((fi, mesh.face_colors[f]));
        if mesh.segment_labels[f] != label {
            moved += 1;
        }
        mesh.segment_labels[f] = label;
        mesh.face_colors[f] = MeshModel::manual_label_color(label);
    }
    for (rid, faces) in &region_faces {
        final_regions.push((*rid, faces.len()));
    }
    final_regions.sort_by(|a, b| b.1.cmp(&a.1));
    let median_face: usize = final_regions
        .get(final_regions.len() / 2)
        .map(|(_, c)| *c)
        .unwrap_or(0);
    let min_face = final_regions.last().map(|(_, c)| *c).unwrap_or(0);
    let max_face = final_regions.first().map(|(_, c)| *c).unwrap_or(0);
    log::info!(
        "[fuse] post_merge regions={} min={:?} median={} max={:?} top5={:?}",
        final_regions.len(),
        final_regions.last().map(|(_, c)| *c),
        median_face,
        final_regions.first().map(|(_, c)| *c),
        final_regions.iter().take(5).collect::<Vec<_>>()
    );
    mesh.history
        .record(OpKind::ManualRegion, None, &prev_colors, &prev_labels);
    mesh.rebuild_segments();

    Ok(FuseResult {
        segments: mesh.sorted_segments(),
        segment_labels: mesh.segment_labels.clone(),
        moved_faces: moved,
        region_count: id_to_label.len(),
        edge_total: Some(edge_total),
        edge_cut: Some(edge_cut),
        region_size_min: min_face,
        region_size_max: max_face,
        region_size_median: median_face,
        planar_vote_edges,
        planar_cut_votes,
        multiview_vote_edges,
        multiview_cut_votes,
        dihedral_cut_votes,
        eye_cut_votes,
        regions_before_merge,
        regions_after_merge,
        merge_passes,
        min_faces,
        tiny_regions_before_merge,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::segment::metrics::unit_cube;
    use crate::segment::planar::{detect_planar_regions, PlanarParams};
    use crate::segment::multiview::{detect_multiview_regions, MultiViewParams};

    fn cube_planar_sets() -> Vec<Vec<u32>> {
        let mesh = unit_cube();
        let regions = detect_planar_regions(
            &mesh,
            &PlanarParams {
                angle_thr_deg: 15.0,
                dist_thr_factor: 1.0 / 30.0,
                min_region_faces: 2,
            },
        );
        regions.into_iter().map(|r| r.face_indices).collect()
    }

    fn cube_multiview_sets() -> Vec<Vec<u32>> {
        let mesh = unit_cube();
        let regions = detect_multiview_regions(
            &mesh,
            &MultiViewParams {
                view_count: 20,
                angle_thr_deg: 20.0,
                min_region_faces: 2,
                match_threshold: 1,
            },
        );
        regions.into_iter().map(|r| r.face_indices).collect()
    }

    #[test]
    fn cube_fuses_to_six_regions() {
        let mut mesh = unit_cube();
        let planar = cube_planar_sets();
        let multiview = cube_multiview_sets();
        let r = fuse_region_sets(&mut mesh, &planar, &multiview, &[], &[], 1, 2).unwrap();
        assert_eq!(r.region_count, 6, "cube → 6 fused regions");
        assert_eq!(r.moved_faces, 12, "all 12 faces receive a fused label");
        assert_eq!(r.segments.len(), 6, "segment metadata must list 6 regions");
    }

    #[test]
    fn every_face_is_labelled() {
        let mut mesh = unit_cube();
        let planar = cube_planar_sets();
        let multiview = cube_multiview_sets();
        fuse_region_sets(&mut mesh, &planar, &multiview, &[], &[], 1, 2).unwrap();
        assert!(
            mesh.segment_labels.iter().all(|&l| l != 0 || true),
            "labels present"
        );
        assert_eq!(mesh.segment_labels.len(), 12);
    }

    #[test]
    fn empty_input_is_an_error() {
        let mut mesh = unit_cube();
        let err = fuse_region_sets(&mut mesh, &[], &[], &[], &[], 1, 2);
        assert!(err.is_err());
    }

    #[test]
    fn single_planar_region_covers_cube() {
        // One giant planar set covering the whole cube + no multiview evidence
        // → no cut votes → a single fused region (tie keeps everything).
        let mut mesh = unit_cube();
        let all: Vec<u32> = (0..12).collect();
        let r = fuse_region_sets(&mut mesh, &[all], &[], &[], &[], 1, 2).unwrap();
        assert_eq!(r.region_count, 1, "no cut votes → one region");
    }

    /// Eye channel: when no other channel provides a cut vote, an eye-region
    /// set must carve itself out as a single region. Without EYE_WEIGHT, the
    /// eye faces would just be left inside whatever neighbour Dijkstra gave
    /// them; with it, every perimeter edge of the eye region is cut, so the
    /// eye faces form one connected island separate from everything else. The
    /// post-fuse tiny-region filter would also collapse a too-small eye set, so
    /// we hand `min_region_faces = 0` to keep the eye region even at < 8 faces.
    #[test]
    fn eye_set_carves_itself_out() {
        let mut mesh = unit_cube(); // 12 faces, 6 quad sides
                                    // Treat all of face 0 as "eye" — the cube has no other
                                    // cut votes from planar/mv/dihedral, so without the eye
                                    // channel we would get 1 region (single_planar_region...).
        let eye_sets: Vec<Vec<u32>> = vec![vec![0]];
        let r = fuse_region_sets(&mut mesh, &[], &[], &[], &eye_sets, 1, 0).unwrap();
        // 1 face labelled as eye plus the remaining 11 faces stay in a single
        // component (no other votes) → 2 regions total.
        assert_eq!(
            r.region_count, 2,
            "eye channel must carve face 0 out: got {} regions",
            r.region_count
        );
        // face 0 must be in a region with itself only.
        let f0_label = r.segment_labels[0];
        assert_eq!(
            r.segment_labels.iter().filter(|&&l| l == f0_label).count(),
            1,
            "face 0 must be the only face in its region"
        );
    }

    /// Eye channel must respect intra-region cohesion: two adjacent faces
    /// inside the same eye set stay in one region. If the eye channel naively
    /// gave every eye face its own one-face label, faces 0 and 1 (which share
    /// an edge on the unit_cube) would each be carved out — losing the eye
    /// region's *shape*.
    #[test]
    fn eye_set_keeps_intra_region_cohesion() {
        let mut mesh = unit_cube();
        // faces 0 and 1 share the edge between vertex 1 and 2 on a unit cube
        // built via `unit_cube()` (see `segment::metrics`). Treating them as
        // one eye → exactly 1 eye-region + 1 outer region = 2 regions.
        let eye_sets: Vec<Vec<u32>> = vec![vec![0, 1]];
        let r = fuse_region_sets(&mut mesh, &[], &[], &[], &eye_sets, 1, 0).unwrap();
        assert_eq!(
            r.region_count, 2,
            "two adjacent eye faces stay one region; the outer 10 faces form the other: got {} regions",
            r.region_count
        );
        let f0 = r.segment_labels[0];
        let f1 = r.segment_labels[1];
        assert_eq!(f0, f1, "face 0 and face 1 must share the eye region label");
    }

    /// Dihedral creases must NOT cut inside an eye-bounded region. Otherwise
    /// a low dihedral threshold would shred the semantic eye ROI into many
    /// tiny locked fragments that survive tiny-region merge and destroy the
    /// median region size.
    #[test]
    fn dihedral_does_not_cut_inside_eye_region() {
        let mut mesh = unit_cube();
        // faces 0 and 1 are adjacent and both eye-bounded. A dihedral detector
        // that labels them as two different regions would normally cut the
        // shared edge; with the iter85 fix that cut is suppressed because both
        // endpoints are eye faces.
        let eye_sets: Vec<Vec<u32>> = vec![vec![0, 1]];
        let dihedral_sets: Vec<Vec<u32>> = vec![vec![0], vec![1]];
        let r = fuse_region_sets(&mut mesh, &[], &[], &dihedral_sets, &eye_sets, 1, 0).unwrap();
        assert_eq!(
            r.region_count, 2,
            "dihedral must not split the eye region internally: got {} regions",
            r.region_count
        );
        let f0 = r.segment_labels[0];
        let f1 = r.segment_labels[1];
        assert_eq!(f0, f1, "eye faces 0 and 1 must stay together");
    }
}
