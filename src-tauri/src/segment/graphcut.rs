//! Graph-cut refinement for SDF segmentation (Shapira 2008 / CGAL standard
//! second stage). Adversarial review (iter 45) demanded the *signed* minima-rule
//! prior here — the unsigned dot merge in `sdf.rs` was the known defect.
//!
//! Stage 1 — GMM(EM) soft clustering: fit k Gaussians to the log-SDF values and
//! assign each face a k-dimensional soft probability vector. This replaces the
//! hard 1-D k-means used by the legacy `segment_by_sdf` path; the soft posterior
//! is the graph-cut data term.
//!
//! Stage 2 — alpha-expansion graph cut (Boykov, Veksler, Zabih 2001): minimise
//!   E(x) = Σ_f −log(max(P(f|x_f), ε)) + λ Σ_{f,g} V(x_f≠x_g)
//! where the Potts penalty V uses a *signed* dihedral weight — a concave fold
//! costs CONCAVE_WEIGHT to cut (cheap, it is a real part boundary) and a convex
//! edge costs CONVEX_WEIGHT (expensive). This is exactly CGAL's
//! `Surface_mesh_segmentation` second term; the legacy merge had no way to
//! express it because `acos(dot)` is unsigned.

use petgraph::visit::EdgeRef;

use crate::mesh::loader::ProgressFn;
use crate::mesh::model::{MeshModel, Segment};
use crate::segment::postprocess::{finalize_segments, log_normalize};
use crate::segment::sdf::compute_sdf_inner;

/// Concave folds cost 1× their dihedral to cut; convex edges cost 0.1× — CGAL's
/// Surface_mesh_segmentation ratio (0.1), stricter than `postprocess`'s 0.5.
const CONCAVE_WEIGHT: f32 = 1.0;
const CONVEX_WEIGHT: f32 = 0.1;
/// Floor for log(P): a zero posterior must not produce −∞.
const LOG_EPS: f32 = 1e-6;
/// Default smoothness λ. Higher → fewer, larger regions (CGAL's "smoothness").
const DEFAULT_LAMBDA: f32 = 0.35;
/// Hard ceiling on the cluster count (mirrors curvature::MAX_K; keeps the
/// label space under MANUAL_SEGMENT_OFFSET and the expansion loop tractable).
const MAX_K: usize = 24;
/// EM iterations. Deterministic seed (sorted evenly-spaced means) → converges
/// to a fixed point; 50 iters is generous for 1-D mixtures.
const EM_ITERS: usize = 50;
/// α-expansion outer loops. Each full pass over all labels tries one expansion
/// per label; convergence is usually reached in 2–3 passes.
const EXPANSION_PASSES: usize = 3;

/// Signed dihedral angle (radians) across the shared edge of adjacent faces
/// `fi`,`fj`, weighted concave×CONCAVE_WEIGHT / convex×CONVEX_WEIGHT. Sign test
/// mirrors `postprocess::crease_strength_deg`: the neighbour centroid on the
/// front side of fi's plane ⇒ concave.
fn signed_dihedral_rad(mesh: &MeshModel, normals: &[[f32; 3]], fi: usize, fj: usize) -> f32 {
    let ni = normals[fi];
    let dot = (ni[0] * normals[fj][0] + ni[1] * normals[fj][1] + ni[2] * normals[fj][2])
        .clamp(-1.0, 1.0);
    let ang = dot.acos(); // 0..π
    let ci = mesh.face_center(fi as u32);
    let cj = mesh.face_center(fj as u32);
    let d = [cj[0] - ci[0], cj[1] - ci[1], cj[2] - ci[2]];
    let concave = d[0] * ni[0] + d[1] * ni[1] + d[2] * ni[2] > 0.0;
    if concave {
        ang * CONCAVE_WEIGHT
    } else {
        ang * CONVEX_WEIGHT
    }
}

/// Potts smoothness penalty for a cut across edge (fi,fj): −log(θ/π) with θ the
/// signed weighted dihedral. A sharp concave fold → θ near π → cost → 0 (cheap
/// to cut, a real boundary); a flat edge → θ near 0 → cost large (expensive to
/// split a smooth surface). Matches CGAL's `e2`.
fn smooth_penalty(mesh: &MeshModel, normals: &[[f32; 3]], fi: usize, fj: usize) -> f32 {
    let theta = signed_dihedral_rad(mesh, normals, fi, fj)
        .clamp(1e-3, std::f32::consts::PI);
    -((theta / std::f32::consts::PI).ln())
}

/// GMM(EM) soft clustering of 1-D values into `k` components. Deterministic:
/// means seeded evenly across the sorted range. Returns a (n × k) matrix of
/// soft assignments, rows summing to ~1.
fn gmm_soft_cluster(values: &[f32], k: usize) -> Vec<Vec<f32>> {
    let n = values.len();
    if n == 0 {
        return Vec::new();
    }
    let k = k.clamp(1, MAX_K).min(n);
    if k == 1 {
        return vec![vec![1.0; 1]; n];
    }

    let mut sorted: Vec<f32> = values.to_vec();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap());

    // Deterministic seed: evenly spaced means across the sorted range, then one
    // hard-assignment pass (nearest mean) to initialise each component's
    // variance and weight from its actual share of the data. CGAL's
    // segmentation initialises the GMM with k-means; a fixed unit variance here
    // degenerates on tightly-clustered inputs (two point masses → every point
    // shares both Gaussians, EM never separates them).
    let mut means: Vec<f32> = (0..k).map(|i| sorted[(i * n / k).min(n - 1)]).collect();
    let mut assigned: Vec<u32> = vec![0; n];
    for (i, &v) in values.iter().enumerate() {
        let c = (0..k)
            .min_by(|&a, &b| {
                let da = (v - means[a]).abs();
                let db = (v - means[b]).abs();
                da.partial_cmp(&db).unwrap()
            })
            .unwrap();
        assigned[i] = c as u32;
    }
    let mut vars = vec![1.0f32; k];
    let mut weights = vec![0.0f32; k];
    for c in 0..k {
        let members: Vec<f32> = values
            .iter()
            .zip(assigned.iter())
            .filter(|(_, &a)| a == c as u32)
            .map(|(v, _)| *v)
            .collect();
        if members.is_empty() {
            continue;
        }
        let m: f32 = members.iter().sum::<f32>() / members.len() as f32;
        let v: f32 = members.iter().map(|&x| (x - m) * (x - m)).sum::<f32>()
            / members.len() as f32;
        means[c] = m;
        vars[c] = v.max(1e-4);
        weights[c] = members.len() as f32 / n as f32;
    }
    let w_sum: f32 = weights.iter().sum();
    if w_sum > 0.0 {
        for w in weights.iter_mut() {
            *w /= w_sum;
        }
    } else {
        weights = vec![1.0 / k as f32; k];
    }

    let mut gamma = vec![vec![0.0f32; k]; n];
    for _ in 0..EM_ITERS {
        // E-step: posterior γ_ic ∝ w_c · N(x_i; μ_c, σ_c), normalised over c.
        for i in 0..n {
            let mut row_sum = 0.0f32;
            for c in 0..k {
                let d = values[i] - means[c];
                let var = vars[c].max(1e-4);
                let pdf = (-0.5 * d * d / var).exp() / (2.0 * std::f32::consts::PI * var).sqrt();
                let g = weights[c] * pdf.max(1e-12);
                gamma[i][c] = g;
                row_sum += g;
            }
            if row_sum > 1e-12 {
                for c in 0..k {
                    gamma[i][c] /= row_sum;
                }
            }
        }
        // M-step: re-estimate weights / means / variances.
        let mut new_w = vec![0.0f32; k];
        let mut new_m = vec![0.0f32; k];
        let mut new_v = vec![0.0f32; k];
        for c in 0..k {
            let mut sw = 0.0f32;
            let mut sm = 0.0f32;
            for i in 0..n {
                sw += gamma[i][c];
                sm += gamma[i][c] * values[i];
            }
            new_m[c] = if sw > 1e-12 { sm / sw } else { means[c] };
            new_w[c] = sw / n as f32;
            let mut sv = 0.0f32;
            for i in 0..n {
                let d = values[i] - new_m[c];
                sv += gamma[i][c] * d * d;
            }
            new_v[c] = if sw > 1e-12 { sv / sw } else { vars[c] };
        }
        means = new_m;
        vars = new_v;
        weights = new_w;
    }

    // Sort components by mean (ascending) so the soft-assignment columns have a
    // deterministic, semantically meaningful order. EM is permutation-symmetric:
    // without this, a bimodal fit can assign the low-mass component to column 0
    // on one run and column 1 on another, which would make downstream code (and
    // tests) depend on convergence order instead of on the data.
    let mut order: Vec<usize> = (0..k).collect();
    order.sort_by(|&a, &b| means[a].partial_cmp(&means[b]).unwrap());
    // Rebuild gamma columns in sorted-mean order: out[i][new_c] = old[i][orig_c].
    let mut reordered = vec![vec![0.0f32; k]; n];
    for i in 0..n {
        for (new_c, &orig_c) in order.iter().enumerate() {
            reordered[i][new_c] = gamma[i][orig_c];
        }
    }
    reordered
}

/// Dinic max-flow (array-of-vectors adjacency). f32 capacities; the graphs here
/// are sparse (mesh face adjacency + auxiliary nodes), so Dinic's layered BFS
/// + blocking-flow DFS is adequate and deterministic.
struct Dinic {
    n: usize,
    graph: Vec<Vec<Edge>>,
}

#[derive(Clone)]
struct Edge {
    to: usize,
    rev: usize,
    cap: f32,
}

impl Dinic {
    fn new(n: usize) -> Self {
        Self {
            n,
            graph: vec![Vec::new(); n],
        }
    }

    fn add_edge(&mut self, u: usize, v: usize, cap: f32) {
        if cap <= 0.0 {
            return;
        }
        let fwd = Edge {
            to: v,
            rev: self.graph[v].len(),
            cap,
        };
        let rev = Edge {
            to: u,
            rev: self.graph[u].len(),
            cap: 0.0,
        };
        self.graph[u].push(fwd);
        self.graph[v].push(rev);
    }

    fn bfs(&self, s: usize, t: usize, level: &mut [i32]) -> bool {
        for l in level.iter_mut() {
            *l = -1;
        }
        let mut q = std::collections::VecDeque::new();
        level[s] = 0;
        q.push_back(s);
        while let Some(u) = q.pop_front() {
            for e in &self.graph[u] {
                if e.cap > 1e-9 && level[e.to] < 0 {
                    level[e.to] = level[u] + 1;
                    q.push_back(e.to);
                }
            }
        }
        level[t] >= 0
    }

    fn dfs(&mut self, u: usize, t: usize, f: f32, level: &[i32], it: &mut [usize]) -> f32 {
        if u == t {
            return f;
        }
        while it[u] < self.graph[u].len() {
            let ei = it[u];
            let e = self.graph[u][ei].clone();
            if e.cap > 1e-9 && level[e.to] == level[u] + 1 {
                let d = self.dfs(e.to, t, f.min(e.cap), level, it);
                if d > 1e-9 {
                    self.graph[u][ei].cap -= d;
                    self.graph[e.to][e.rev].cap += d;
                    return d;
                }
            }
            it[u] += 1;
        }
        0.0
    }

    fn max_flow(&mut self, s: usize, t: usize) -> f32 {
        let mut flow = 0.0f32;
        let mut level = vec![0i32; self.n];
        let mut it = vec![0usize; self.n];
        while self.bfs(s, t, &mut level) {
            for v in it.iter_mut() {
                *v = 0;
            }
            loop {
                let f = self.dfs(s, t, f32::INFINITY, &level, &mut it);
                if f <= 1e-9 {
                    break;
                }
                flow += f;
            }
        }
        flow
    }
}

/// One α-expansion move: fix target label `alpha`, build the Boykov–Veksler–
/// Zabih graph for Potts V, solve the min-cut, and apply the new labels.
/// Returns true if any vertex changed.
///
/// Graph layout (nodes):
///   0..n            — original faces
///   n..n+edges      — one auxiliary node per face-adjacency edge
///   S = n + edges   — source (label = alpha)
///   T = S + 1       — sink (label = keep current)
///
/// T-links: face v with x_v≠α gets S→v cap D_v(α) and v→T cap D_v(x_v); a face
/// already at α gets S→v cap ∞ (it may not move away during this expansion).
/// N-links through the auxiliary node a_e enforce the Potts penalty w_e when
/// exactly one endpoint adopts α (derivation in the module doc / BVZ01 §3.2).
fn alpha_expansion_move(
    mesh: &MeshModel,
    normals: &[[f32; 3]],
    probs: &[Vec<f32>],
    labels: &mut [u32],
    alpha: u32,
    lambda: f32,
) -> bool {
    let n = mesh.faces.len();
    let n_edges = mesh.face_adjacency.edge_count();
    let s = n + n_edges;
    let t = s + 1;
    let mut g = Dinic::new(t + 1);

    // Data-term t-links.
    for (v, p) in probs.iter().enumerate() {
        let cur = labels[v];
        if cur == alpha {
            g.add_edge(s, v, f32::INFINITY);
        } else {
            let d_alpha = -(p[alpha as usize].max(LOG_EPS)).ln();
            let d_cur = -(p[cur as usize].max(LOG_EPS)).ln();
            g.add_edge(s, v, d_alpha);
            g.add_edge(v, t, d_cur);
        }
    }

    // Smoothness N-links via one auxiliary node per adjacency edge.
    let mut aux = n;
    for edge in mesh.face_adjacency.edge_references() {
        let fi = mesh.face_adjacency[edge.source()] as usize;
        let fj = mesh.face_adjacency[edge.target()] as usize;
        if fi == fj {
            continue;
        }
        let w = lambda * smooth_penalty(mesh, normals, fi, fj);
        let a = aux;
        aux += 1;
        // Potts construction: aux↔u, aux↔v each w; S→aux w; aux→T w.
        g.add_edge(s, a, w);
        g.add_edge(a, fi, w);
        g.add_edge(a, fj, w);
        g.add_edge(fi, a, w);
        g.add_edge(fj, a, w);
        g.add_edge(a, t, w);
    }
    debug_assert_eq!(aux, n + n_edges);

    let _flow = g.max_flow(s, t);

    // Min-cut side: S-reachable faces adopt α, others keep their label.
    let mut seen = vec![false; n + n_edges + 2];
    let mut stack = vec![s];
    seen[s] = true;
    while let Some(u) = stack.pop() {
        for e in &g.graph[u] {
            if e.cap > 1e-9 && !seen[e.to] {
                seen[e.to] = true;
                stack.push(e.to);
            }
        }
    }

    let mut changed = false;
    for (v, p) in probs.iter().enumerate() {
        if seen[v] && labels[v] != alpha {
            // Guard: only move when the data term actually favours α, so a
            // degenerate cut cannot thrash labels back and forth.
            if p[alpha as usize] > p[labels[v] as usize] {
                labels[v] = alpha;
                changed = true;
            }
        }
    }
    changed
}

/// Segment by SDF + GMM soft clustering + alpha-expansion graph cut.
/// This is the CGAL-standard second stage; the legacy `segment_by_sdf` keeps
/// its hard k-means + merge path as the comparison baseline.
pub fn segment_by_sdf_graphcut(
    mesh: &mut MeshModel,
    k_user: u32,
    on_progress: &ProgressFn,
) -> Vec<Segment> {
    let n = mesh.faces.len();
    on_progress(0.0, "sdf:sample");
    let oriented = crate::segment::sdf::oriented_normals(mesh);
    let sdf = compute_sdf_inner(mesh, &oriented, on_progress, 0.0, 0.35);
    on_progress(0.35, "sdf:gmm");
    let ln_sdf = log_normalize(&sdf);

    let k = if k_user == 0 {
        // Reuse the histogram-peak estimator from the legacy path.
        crate::segment::sdf::estimate_k(&ln_sdf)
    } else {
        k_user as usize
    };

    let probs = gmm_soft_cluster(&ln_sdf, k);
    on_progress(0.55, "sdf:graphcut");

    // Initial labels: MAP (argmax) of the soft posterior.
    let mut labels: Vec<u32> = (0..n)
        .map(|v| {
            probs[v]
                .iter()
                .enumerate()
                .max_by(|a, b| a.1.partial_cmp(b.1).unwrap())
                .map(|(i, _)| i as u32)
                .unwrap_or(0)
        })
        .collect();

    // α-expansion: multiple passes until a pass changes nothing.
    let lambda = DEFAULT_LAMBDA;
    let mut total_changes = 0usize;
    for _pass in 0..EXPANSION_PASSES {
        let mut changed_this_pass = false;
        for alpha in 0..k as u32 {
            if alpha_expansion_move(mesh, &oriented, &probs, &mut labels, alpha, lambda) {
                changed_this_pass = true;
                total_changes += 1;
            }
        }
        on_progress(0.7 + 0.2 * (_pass as f32 + 1.0) / EXPANSION_PASSES as f32, "sdf:graphcut");
        if !changed_this_pass {
            break;
        }
    }
    log::info!(
        "[segment:sdf_graphcut] k={} expansion_passes_done, {}/{} labels moved during expansion",
        k,
        total_changes,
        n
    );

    on_progress(0.95, "sdf:connectivity");
    // Split disconnected same-label patches into separate segments and clean
    // crumbs, exactly as the shared pipeline does — the graph cut is a
    // *label* partition; connectivity must still be enforced per region.
    let curv = crate::segment::postprocess::face_curvature(mesh, &oriented);
    let feats = crate::segment::postprocess::assemble_features(&curv, Some(&ln_sdf));
    let labels = crate::segment::postprocess::refine_regions(mesh, &labels, &feats, &oriented, None);

    on_progress(1.0, "done");
    finalize_segments(mesh, labels)
}

#[cfg(test)]
mod tests {
    use super::*;

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

    /// Large cube (3^3 at origin) + small cube (0.5^3 far away), spatially
    /// separated so SDF rays do not leak between the shells.
    fn two_separated_cubes() -> MeshModel {
        let v: [f32; 48] = [
            0.0, 0.0, 0.0, 3.0, 0.0, 0.0, 3.0, 3.0, 0.0, 0.0, 3.0, 0.0,
            0.0, 0.0, 3.0, 3.0, 0.0, 3.0, 3.0, 3.0, 3.0, 0.0, 3.0, 3.0,
            10.0, 10.0, 10.0, 10.5, 10.0, 10.0, 10.5, 10.5, 10.0, 10.0, 10.5, 10.0,
            10.0, 10.0, 10.5, 10.5, 10.0, 10.5, 10.5, 10.5, 10.5, 10.0, 10.5, 10.5,
        ];
        let verts: Vec<[f32; 3]> = v.chunks(3).map(|c| [c[0], c[1], c[2]]).collect();
        let f: [u32; 72] = [
            0, 3, 2, 0, 2, 1, 4, 5, 6, 4, 6, 7,
            0, 1, 5, 0, 5, 4, 2, 3, 7, 2, 7, 6,
            1, 2, 6, 1, 6, 5, 3, 0, 4, 3, 4, 7,
            8, 11, 10, 8, 10, 9, 12, 13, 14, 12, 14, 15,
            8, 9, 13, 8, 13, 12, 10, 11, 15, 10, 15, 14,
            9, 10, 14, 9, 14, 13, 11, 8, 12, 11, 12, 15,
        ];
        build_mesh(&verts, &f)
    }

    #[test]
    fn gmm_soft_cluster_1d_is_deterministic_and_normalized() {
        let values: Vec<f32> = (0..100).map(|i| (i as f32) / 100.0).collect();
        let a = gmm_soft_cluster(&values, 3);
        let b = gmm_soft_cluster(&values, 3);
        assert_eq!(a, b, "GMM must be deterministic (same seed path)");
        for row in &a {
            let sum: f32 = row.iter().sum();
            assert!((sum - 1.0).abs() < 1e-3, "row sum must be ~1, got {}", sum);
            assert_eq!(row.len(), 3);
        }
    }

    #[test]
    fn gmm_soft_cluster_separates_bimodal_data() {
        // 50 values near 0.2, 50 near 0.8 → two clean clusters.
        let mut values = vec![0.2f32; 50];
        values.extend(vec![0.8f32; 50]);
        let probs = gmm_soft_cluster(&values, 2);
        let low: Vec<f32> = probs[0..50].iter().map(|r| r[0]).collect();
        let high: Vec<f32> = probs[50..100].iter().map(|r| r[0]).collect();
        let low_mean = low.iter().sum::<f32>() / 50.0;
        let high_mean = high.iter().sum::<f32>() / 50.0;
        assert!(
            low_mean > 0.9 && high_mean < 0.1,
            "bimodal split failed: low={} high={}",
            low_mean,
            high_mean
        );
    }

    #[test]
    fn graphcut_two_cubes_k2() {
        let mut m = two_separated_cubes();
        let segs = segment_by_sdf_graphcut(&mut m, 2, &|_, _| {});
        assert_eq!(segs.len(), 2, "two separated cubes must be 2 segments");
    }

    #[test]
    fn graphcut_auto_k_two_cubes() {
        let mut m = two_separated_cubes();
        let segs = segment_by_sdf_graphcut(&mut m, 0, &|_, _| {});
        assert!(
            segs.len() >= 2,
            "auto-k must not collapse two cubes to 1 (got {})",
            segs.len()
        );
    }
}
