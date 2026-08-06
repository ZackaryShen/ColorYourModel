use crate::mesh::model::{MeshModel, Segment};
use crate::mesh::loader::ProgressFn;
use petgraph::visit::EdgeRef;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::cmp::Reverse;
use std::io::Write as _;

/// Write a debug line to segment_debug.log (appended, not overwritten).
fn seg_log(msg: &str) {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "\\segment_debug.log");
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true).append(true).open(path)
    {
        let _ = writeln!(f, "[{}] {}", chrono_now(), msg);
    }
}

/// Simple timestamp without chrono dependency.
fn chrono_now() -> String {
    use std::time::SystemTime;
    let d = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default();
    format!("T+{}s", d.as_secs())
}

// ─── Constants ───────────────────────────────────────────────────
/// Minimum region size as fraction of total faces (0.2% of model).
/// Chosen so small models (500 faces) get min=10, large models (500k) get min=1000.
const MIN_REGION_FRACTION: f64 = 0.002;
const MIN_REGION_FLOOR: u32 = 10;
/// Cap on min_faces to prevent large models from having absurdly high thresholds.
/// E2E evidence (1.5M faces, 13k regions, avg 113 faces/region):
///   cap=500 → all regions merged to 1 (catastrophic)
///   cap=30  → only truly degenerate fragments (<30 faces) get merged
/// Value 30 chosen: a region with <30 faces can't form meaningful geometry.
const MIN_REGION_CAP: u32 = 30;
const MAX_MERGE_ITERATIONS: u32 = 10;

/// Normal-consistency merge threshold: dot product ≥ this → regions are "same surface".
/// 0.93 ≈ 22° — merges smooth-curve fragments that dihedral split broke apart.
/// E2E evidence: at 0.97 (14°) only 516/14460 regions merged on a 1.5M-face model;
///               at 0.93 (22°) substantially more smooth fragments are reassembled.
const MERGE_NORMAL_DOT_THRESHOLD: f64 = 0.93;

/// Maximum iterations for the normal-consistency merge (prevents runaway merging).
const MAX_NORMAL_MERGE_ITERATIONS: u32 = 50;

// ─── Region data for semantic merge ─────────────────────────────
struct RegionData {
    face_count: u32,
    /// Sum of all face normals in this region (divide by face_count to get avg, then normalize)
    sum_normals: [f64; 3],
}

impl RegionData {
    fn avg_normal(&self) -> [f64; 3] {
        let n = self.face_count as f64;
        let avg = [
            self.sum_normals[0] / n,
            self.sum_normals[1] / n,
            self.sum_normals[2] / n,
        ];
        // Normalize
        let len = (avg[0] * avg[0] + avg[1] * avg[1] + avg[2] * avg[2]).sqrt();
        if len > 1e-10 {
            [avg[0] / len, avg[1] / len, avg[2] / len]
        } else {
            [0.0, 0.0, 1.0]
        }
    }
}

/// Segment mesh by dihedral angle, then merge geometrically-similar adjacent regions.
pub fn segment_by_dihedral_angle(
    mesh: &mut MeshModel,
    angle_threshold: f32,
    on_progress: &ProgressFn,
) -> Vec<Segment> {
    let n_faces = mesh.faces.len();
    let threshold_rad = angle_threshold.to_radians();
    seg_log(&format!("=== segment START: {} faces, threshold={}deg ===", n_faces, angle_threshold));

    on_progress(0.05, &format!("正在分割 {} 个面...", n_faces));

    log::info!(
        "[segment] starting: {} faces, threshold={}° ({:.4} rad), graph_edges={}",
        n_faces,
        angle_threshold,
        threshold_rad,
        mesh.face_adjacency.edge_count()
    );

    // ── Phase 1: Dihedral angle split (Union-Find) ───────────────
    on_progress(0.10, "比较面法线...");
    let mut connected_pairs: Vec<(u32, u32)> = Vec::new();
    let mut skipped = 0u32;
    for edge in mesh.face_adjacency.edge_references() {
        let fi = mesh.face_adjacency[edge.source()];
        let fj = mesh.face_adjacency[edge.target()];
        let ni = &mesh.normals[fi as usize];
        let nj = &mesh.normals[fj as usize];
        let dot = (ni[0] * nj[0] + ni[1] * nj[1] + ni[2] * nj[2]).clamp(-1.0, 1.0);
        let angle = dot.acos();
        if angle < threshold_rad {
            connected_pairs.push((fi, fj));
        } else {
            skipped += 1;
        }
    }

    log::info!(
        "[segment] connected_pairs={}, skipped(sharp)={}, total_edges={}",
        connected_pairs.len(),
        skipped,
        mesh.face_adjacency.edge_count()
    );

    on_progress(0.30, &format!("从 {} 对中构建区域...", connected_pairs.len()));
    let mut parent: Vec<u32> = (0..n_faces as u32).collect();

    fn find(parent: &mut Vec<u32>, x: u32) -> u32 {
        if parent[x as usize] != x {
            parent[x as usize] = find(parent, parent[x as usize]);
        }
        parent[x as usize]
    }

    for &(a, b) in &connected_pairs {
        let ra = find(&mut parent, a);
        let rb = find(&mut parent, b);
        if ra != rb {
            parent[ra as usize] = rb;
        }
    }

    // Flatten parents
    on_progress(0.50, "展平区域标签...");
    for i in 0..n_faces {
        parent[i] = find(&mut parent, i as u32);
    }

    // Remap labels to contiguous IDs + build region data
    on_progress(0.55, "分配区域 ID...");
    let mut label_map: HashMap<u32, u32> = HashMap::new();
    let mut next_id = 0u32;
    let mut labels = vec![0u32; n_faces];
    let mut region_data: HashMap<u32, RegionData> = HashMap::new();
    let mut region_faces: HashMap<u32, Vec<u32>> = HashMap::new();

    for i in 0..n_faces {
        let root = parent[i];
        let id = *label_map.entry(root).or_insert_with(|| {
            let id = next_id;
            next_id += 1;
            id
        });
        labels[i] = id;
        let rd = region_data.entry(id).or_insert_with(|| RegionData {
            face_count: 0,
            sum_normals: [0.0; 3],
        });
        rd.face_count += 1;
        let n = &mesh.normals[i];
        rd.sum_normals[0] += n[0] as f64;
        rd.sum_normals[1] += n[1] as f64;
        rd.sum_normals[2] += n[2] as f64;
        region_faces.entry(id).or_default().push(i as u32);
    }

    let phase1_count = region_data.len();
    log::info!("[segment] Phase 1: {} regions (dihedral split)", phase1_count);
    seg_log(&format!("Phase1 done: {} regions (dihedral split)", phase1_count));

    // ── Phase 2: Build Region Adjacency Graph (RAG) ──────────────
    // One pass over edges: O(|edges|)
    on_progress(0.60, "构建区域邻接图...");
    let mut region_adj: HashMap<u32, HashMap<u32, u32>> = HashMap::new();

    for edge in mesh.face_adjacency.edge_references() {
        let fi = mesh.face_adjacency[edge.source()];
        let fj = mesh.face_adjacency[edge.target()];
        let li = labels[fi as usize];
        let lj = labels[fj as usize];
        if li != lj {
            *region_adj.entry(li).or_default().entry(lj).or_insert(0) += 1;
            *region_adj.entry(lj).or_default().entry(li).or_insert(0) += 1;
        }
    }

    // ── Phase 3: Normal-consistency merge (semantic grouping) ────
    // Merge adjacent regions whose weighted-average normals are very similar.
    // This reassembles smooth-surface fragments that dihedral split broke apart.
    on_progress(0.65, "按法线一致性归并语义区域...");
    seg_log(&format!("Phase3 start: normal-consistency merge ({} regions)", region_data.len()));
    normal_consistency_merge(
        &mut labels,
        &mut region_data,
        &mut region_faces,
        &mut region_adj,
    );

    let phase3_count = region_data.len();
    log::info!(
        "[segment] Phase 3: {} regions (after normal merge, was {})",
        phase3_count, phase1_count
    );
    seg_log(&format!("Phase3 done: {} regions (was {})", phase3_count, phase1_count));

    // ── Phase 4: Merge tiny leftover regions ─────────────────────
    on_progress(0.85, "合并残余小区域...");
    seg_log(&format!("Phase4 start: merge tiny regions ({} remain)", region_data.len()));
    let min_faces = std::cmp::min(
        MIN_REGION_CAP,
        std::cmp::max(
            MIN_REGION_FLOOR,
            (n_faces as f64 * MIN_REGION_FRACTION).ceil() as u32,
        ),
    );
    log::info!(
        "[segment] min_faces={} (total={})",
        min_faces, n_faces
    );
    merge_small_regions_fast(
        &mut labels,
        &mut region_data,
        &mut region_faces,
        &mut region_adj,
        min_faces,
    );

    // ── Final: Re-compact labels to contiguous IDs ───────────────
    on_progress(0.92, "重压缩区域 ID...");
    let mut compact_map: HashMap<u32, u32> = HashMap::new();
    let mut compact_next = 0u32;
    let mut compact_counts: HashMap<u32, u32> = HashMap::new();
    for i in 0..n_faces {
        let old = labels[i];
        let new_id = *compact_map.entry(old).or_insert_with(|| {
            let id = compact_next;
            compact_next += 1;
            id
        });
        labels[i] = new_id;
        *compact_counts.entry(new_id).or_insert(0) += 1;
    }

    mesh.segment_labels = labels;

    let mut segments = HashMap::new();
    for (&id, &count) in &compact_counts {
        segments.insert(
            id,
            Segment {
                id,
                name: format!("Region {}", id + 1),
                color: None,
                face_count: count,
            },
        );
    }
    mesh.segments = segments;

    let seg_count = mesh.segments.len();
    seg_log(&format!("=== segment DONE: {} final regions ===", seg_count));
    on_progress(0.95, &format!("找到 {} 个区域", seg_count));

    mesh.sorted_segments()
}

// ─── Phase 3: Normal-consistency merge ──────────────────────────
/// Greedy merge: repeatedly merge the most similar adjacent pair
/// (by normal dot product) until no pair exceeds the threshold.
/// Small region always merges into the larger neighbor.
fn normal_consistency_merge(
    labels: &mut Vec<u32>,
    region_data: &mut HashMap<u32, RegionData>,
    region_faces: &mut HashMap<u32, Vec<u32>>,
    region_adj: &mut HashMap<u32, HashMap<u32, u32>>,
) {
    for iter in 0..MAX_NORMAL_MERGE_ITERATIONS {
        // Build priority queue: similarity → list of (small_id, large_id) pairs
        let mut merge_candidates: BTreeMap<Reverse<u64>, Vec<(u32, u32)>> =
            BTreeMap::new();

        let mut visited_pairs: HashSet<(u32, u32)> = HashSet::new();
        for (&ra, neighbors) in region_adj.iter() {
            if !region_data.contains_key(&ra) {
                continue;
            }
            let na = match region_data.get(&ra) {
                Some(rd) => rd.avg_normal(),
                None => continue,
            };
            for (&rb, _) in neighbors {
                if !region_data.contains_key(&rb) {
                    continue;
                }
                let pair_key = (ra.min(rb), ra.max(rb));
                if visited_pairs.contains(&pair_key) {
                    continue;
                }
                visited_pairs.insert(pair_key);

                let nb = match region_data.get(&rb) {
                    Some(rd) => rd.avg_normal(),
                    None => continue,
                };
                let dot = (na[0] * nb[0] + na[1] * nb[1] + na[2] * nb[2]).clamp(-1.0, 1.0);
                if dot >= MERGE_NORMAL_DOT_THRESHOLD {
                    // Determine direction: small → large
                    let ca = region_data[&ra].face_count;
                    let cb = region_data[&rb].face_count;
                    let (small, large) = if ca <= cb { (ra, rb) } else { (rb, ra) };
                    // Use u64 key for ordering (higher dot = merge first)
                    let key = (dot * 1_000_000.0) as u64;
                    merge_candidates
                        .entry(Reverse(key))
                        .or_default()
                        .push((small, large));
                }
            }
        }

        if merge_candidates.is_empty() {
            log::info!("[segment] normal merge converged at iteration {}", iter);
            seg_log(&format!("normal merge converged at iteration {}", iter));
            break;
        }

        let mut merged_this_iter = 0u32;

        // Process from highest similarity to lowest
        for (_key, pairs) in &merge_candidates {
            for &(small_id, large_id) in pairs {
                // Skip if either region was already merged away
                if !region_data.contains_key(&small_id) || !region_data.contains_key(&large_id) {
                    continue;
                }

                // Merge small into large
                let small_faces = region_faces.remove(&small_id).unwrap_or_default();
                let small_rd = region_data.remove(&small_id).unwrap();

                // Update large region data: incremental sum_normals + face_count
                let large_rd = region_data.get_mut(&large_id).unwrap();
                large_rd.face_count += small_rd.face_count;
                large_rd.sum_normals[0] += small_rd.sum_normals[0];
                large_rd.sum_normals[1] += small_rd.sum_normals[1];
                large_rd.sum_normals[2] += small_rd.sum_normals[2];

                // Relabel faces: O(|small_region|) not O(|n_faces|)
                for &fi in &small_faces {
                    labels[fi as usize] = large_id;
                }
                region_faces
                    .entry(large_id)
                    .or_default()
                    .extend(small_faces);

                // Update RAG: transfer small's neighbors to large
                if let Some(small_neighbors) = region_adj.remove(&small_id) {
                    for (neighbor, edge_count) in small_neighbors {
                        if neighbor == large_id {
                            continue; // self-loop after merge
                        }
                        // Remove small from neighbor's adj
                        if let Some(n_adj) = region_adj.get_mut(&neighbor) {
                            n_adj.remove(&small_id);
                        }
                        // Add edge count to large↔neighbor
                        *region_adj
                            .entry(large_id)
                            .or_default()
                            .entry(neighbor)
                            .or_insert(0) += edge_count;
                        *region_adj
                            .entry(neighbor)
                            .or_default()
                            .entry(large_id)
                            .or_insert(0) += edge_count;
                    }
                }
                // Remove self-loop from large
                if let Some(l_adj) = region_adj.get_mut(&large_id) {
                    l_adj.remove(&large_id);
                }

                merged_this_iter += 1;
            }
        }

        log::info!(
            "[segment] normal merge iter {}: merged {} pairs",
            iter, merged_this_iter
        );
        seg_log(&format!("normal merge iter {}: {} pairs merged, {} regions remain",
            iter, merged_this_iter, region_data.len()));

        if merged_this_iter == 0 {
            break;
        }
    }
}

// ─── Phase 4: Merge tiny leftover regions ───────────────────────
/// Same greedy merge-by-strongest-neighbor, but using the pre-built
/// region_adj index instead of scanning all edges. O(|small_regions| × |avg_neighbors|).
fn merge_small_regions_fast(
    labels: &mut Vec<u32>,
    region_data: &mut HashMap<u32, RegionData>,
    region_faces: &mut HashMap<u32, Vec<u32>>,
    region_adj: &mut HashMap<u32, HashMap<u32, u32>>,
    min_faces: u32,
) {
    for iter in 0..MAX_MERGE_ITERATIONS {
        let small_ids: std::collections::BTreeSet<u32> = region_data
            .iter()
            .filter(|(_, rd)| rd.face_count < min_faces)
            .map(|(&id, _)| id)
            .collect();

        if small_ids.is_empty() {
            log::info!("[segment] small-region merge converged at iteration {}", iter);
            break;
        }

        let mut merged_count = 0u32;

        for &small_id in &small_ids {
            if !region_data.contains_key(&small_id) {
                continue; // already merged
            }

            // Find best neighbor from RAG (O(|neighbors|) not O(|all edges|))
            let best_neighbor = region_adj
                .get(&small_id)
                .and_then(|neighbors| {
                    neighbors
                        .iter()
                        .filter(|(&n, _)| region_data.contains_key(&n))
                        .max_by_key(|(_, &count)| count)
                        .map(|(&n, _)| n)
                });

            if let Some(large_id) = best_neighbor {
                // Merge small into large
                let small_faces = region_faces.remove(&small_id).unwrap_or_default();
                let small_rd = region_data.remove(&small_id).unwrap();

                let large_rd = region_data.get_mut(&large_id).unwrap();
                large_rd.face_count += small_rd.face_count;
                large_rd.sum_normals[0] += small_rd.sum_normals[0];
                large_rd.sum_normals[1] += small_rd.sum_normals[1];
                large_rd.sum_normals[2] += small_rd.sum_normals[2];

                for &fi in &small_faces {
                    labels[fi as usize] = large_id;
                }
                region_faces
                    .entry(large_id)
                    .or_default()
                    .extend(small_faces);

                // Update RAG
                if let Some(small_neighbors) = region_adj.remove(&small_id) {
                    for (neighbor, edge_count) in small_neighbors {
                        if neighbor == large_id {
                            continue;
                        }
                        if let Some(n_adj) = region_adj.get_mut(&neighbor) {
                            n_adj.remove(&small_id);
                        }
                        *region_adj
                            .entry(large_id)
                            .or_default()
                            .entry(neighbor)
                            .or_insert(0) += edge_count;
                        *region_adj
                            .entry(neighbor)
                            .or_default()
                            .entry(large_id)
                            .or_insert(0) += edge_count;
                    }
                }
                if let Some(l_adj) = region_adj.get_mut(&large_id) {
                    l_adj.remove(&large_id);
                }

                merged_count += 1;
            }
        }

        log::info!(
            "[segment] small merge iter {}: merged {} regions",
            iter, merged_count
        );

        if merged_count == 0 {
            break;
        }
    }
}
