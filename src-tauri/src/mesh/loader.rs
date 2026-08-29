use crate::mesh::model::MeshModel;
use std::collections::HashMap;
use std::path::Path;

/// Progress callback type: (fraction 0.0..1.0, stage description)
pub type ProgressFn = dyn Fn(f32, &str) + Send + Sync;

/// Load an STL file and build a MeshModel, reporting progress via callback
pub fn load_stl(path: &Path, on_progress: &ProgressFn) -> Result<MeshModel, String> {
    on_progress(0.05, "Parsing STL file...");
    log::info!("[loader] Loading STL: {:?}", path);

    let mut reader =
        std::fs::File::open(path).map_err(|e| format!("Failed to open file: {}", e))?;
    let stl_mesh =
        stl_io::read_stl(&mut reader).map_err(|e| format!("Failed to parse STL: {}", e))?;

    let tri_count = stl_mesh.faces.len();
    let raw_vert_count = stl_mesh.vertices.len();
    log::info!(
        "[loader] STL parsed: {} triangles, {} raw vertices",
        tri_count, raw_vert_count
    );
    on_progress(
        0.20,
        &format!("Parsed {} triangles, {} vertices", tri_count, raw_vert_count),
    );

    on_progress(0.25, "Merging duplicate vertices...");
    let mut model = MeshModel::new();

    // Merge duplicate vertices using a HashMap keyed by bit-pattern of f32 coords
    let mut vertex_map: HashMap<[u32; 3], u32> = HashMap::new();
    let mut merged_vertices: Vec<[f32; 3]> = Vec::new();
    let mut faces: Vec<[u32; 3]> = Vec::new();

    for triangle in &stl_mesh.faces {
        let mut face_indices = [0u32; 3];
        for (i, &vert_idx) in triangle.vertices.iter().enumerate() {
            let vert = stl_mesh.vertices[vert_idx];
            let key = [
                vert[0].to_bits(),
                vert[1].to_bits(),
                vert[2].to_bits(),
            ];
            if let Some(&idx) = vertex_map.get(&key) {
                face_indices[i] = idx;
            } else {
                let idx = merged_vertices.len() as u32;
                vertex_map.insert(key, idx);
                merged_vertices.push([vert[0], vert[1], vert[2]]);
                face_indices[i] = idx;
            }
        }
        faces.push(face_indices);
    }

    let vert_count = merged_vertices.len();
    on_progress(
        0.45,
        &format!("Merged to {} unique vertices", vert_count),
    );

    model.vertices = merged_vertices;
    model.faces = faces;

    // Post-process
    on_progress(0.50, "Computing face normals...");
    model.compute_normals();

    on_progress(0.55, "Computing bounding box...");
    model.compute_bbox();

    on_progress(0.60, "Initializing default colors...");
    model.init_default_colors();
    // Initialize per-face segment labels to a valid length (all 0 = unsegmented
    // background). Without this the vector is empty and any paint/segment/undo
    // op that indexes `segment_labels[face_id]` panics (index out of bounds) and
    // poisons the mesh Mutex — the real root cause of "分区笔点一下崩溃" and
    // "undo 崩溃" (iteration 18, REFUTE B1/B4). `restore_paint_state`'s
    // `assert_eq!(segment_labels.len(), faces.len())` also requires this length.
    model.segment_labels = vec![0u32; model.faces.len()];

    on_progress(0.65, "Building spatial index (KD-Tree)...");
    model.build_kdtree();
    model.build_vertex_kdtree();

    on_progress(0.75, "Building adjacency graph...");
    model.build_adjacency();

    on_progress(0.95, "Finalizing...");

    log::info!(
        "[loader] Mesh ready: {} verts, {} faces, bbox=[{:.1},{:.1},{:.1}]→[{:.1},{:.1},{:.1}]",
        model.vertices.len(),
        model.faces.len(),
        model.bbox.min[0], model.bbox.min[1], model.bbox.min[2],
        model.bbox.max[0], model.bbox.max[1], model.bbox.max[2]
    );

    Ok(model)
}

#[cfg(test)]
mod bench {
    use super::*;
    use std::sync::Mutex;
    use std::time::Instant;

    /// Ignored benchmark (run with:
    /// `cargo test --release --lib bench_import -- --ignored --nocapture`).
    /// Times every load_stl stage via the existing progress markers plus the
    /// DTO conversion and JSON serialization that bridge to the frontend, and
    /// the import-time auto segmentation. Numbers are the evidence baseline
    /// for import-pipeline performance work.
    #[test]
    #[ignore]
    fn bench_import_stages() {
        let _ = env_logger::try_init();
        let path = std::env::var("CYM_BENCH_STL")
            .unwrap_or_else(|_| "C:/selfDIr/Blender3D/Sanji+Diorama+Detailed_U1.stl".to_string());

        let started = Instant::now();
        // Instant is Copy, so the outer scope keeps its own copy for totals.
        let total = std::sync::Arc::new(Mutex::new(started));
        let last = std::sync::Arc::new(Mutex::new(started));
        // 'static required by ProgressFn, so the timers move into the closure.
        let progress = move |fraction: f32, stage: &str| {
            let mut last = last.lock().unwrap();
            eprintln!(
                "[stage {:>5.2}] {:<45} +{:>8.2?}  (total {:>8.2?})",
                fraction,
                stage,
                last.elapsed(),
                total.lock().unwrap().elapsed()
            );
            *last = Instant::now();
        };

        let model = load_stl(std::path::Path::new(&path), &progress).expect("load");
        eprintln!("[load_stl total] {:?}", started.elapsed());

        let t = Instant::now();
        let dto = model.to_dto();
        eprintln!("[to_dto] +{:?}", t.elapsed());

        let t = Instant::now();
        let json = serde_json::to_vec(&dto).expect("serialize");
        eprintln!(
            "[serde_json to_vec] +{:?}  size={} MB",
            t.elapsed(),
            json.len() / 1024 / 1024
        );

        // Import-time auto segmentation as shipped: curvatureKMeans default.
        // MeshModel is not Clone (history + graphs), so each algorithm gets a
        // fresh load - the per-load cost is already measured above.
        let t = Instant::now();
        let mut m2 = load_stl(std::path::Path::new(&path), &|_, _| {}).expect("load 2");
        let load2 = t.elapsed();
        let t = Instant::now();
        let segs = crate::segment::run_segmentation(
            &mut m2,
            &crate::segment::SegmentationAlgorithm::CurvatureKMeans {
                k: 6,
                smoothing_iters: 2,
                use_sdf: true,
                crease_threshold_deg: 45.0,
            },
            true,
            &progress,
        );
        eprintln!(
            "[auto_segment curvatureKMeans] +{:?}  (fresh load {:?})  regions={}",
            t.elapsed(),
            load2,
            segs.len()
        );

        let t = Instant::now();
        let mut m3 = load_stl(std::path::Path::new(&path), &|_, _| {}).expect("load 3");
        let load3 = t.elapsed();
        let t = Instant::now();
        let segs = crate::segment::run_segmentation(
            &mut m3,
            &crate::segment::SegmentationAlgorithm::Dihedral {
                angle_threshold: 30.0,
            },
            true,
            &progress,
        );
        eprintln!(
            "[auto_segment dihedral 30] +{:?}  (fresh load {:?})  regions={}",
            t.elapsed(),
            load3,
            segs.len()
        );

        eprintln!("[bench total] {:?}", started.elapsed());
    }
}


#[cfg(test)]
mod bench_kdtree {
    use super::*;

    /// Decision data for the kiddo question (2026-08-28 adversarial review F5),
    /// MEASURED on the 1.88M-face Sanji model: incremental `add` = 439 ms,
    /// `ImmutableKdTree::new_from_slice` = **194.4 s** — kiddo's docs warn
    /// immutable construction can be "perhaps prohibitively slower" and that is
    /// exactly what happens here (~440x slower). Verdict: keep the mutable
    /// `KdTree` and its incremental `add`; the swap was rejected on evidence.
    /// Query parity was confirmed (512/512 candidates, nearest distance diff 0e0),
    /// so nothing is lost by rejecting it beyond build time.
    #[test]
    #[ignore]
    fn bench_kdtree_build_variants() {
        let path = std::env::var("CYM_BENCH_STL")
            .unwrap_or_else(|_| "C:/selfDIr/Blender3D/Sanji+Diorama+Detailed_U1.stl".to_string());
        let model = load_stl(std::path::Path::new(&path), &|_, _| {}).expect("load");

        let centers = model.face_centers();
        let perturbed: Vec<[f32; 3]> = centers
            .iter()
            .enumerate()
            .map(|(i, c)| MeshModel::kd_point(*c, i as u64))
            .collect();
        eprintln!("[points] {}", perturbed.len());

        let t = std::time::Instant::now();
        let mut tree = kiddo::KdTree::new();
        for (i, c) in perturbed.iter().enumerate() {
            tree.add(c, i as u64);
        }
        eprintln!("[kdtree incremental add] {:?}", t.elapsed());

        let t = std::time::Instant::now();
        let itree =
            kiddo::immutable::float::kdtree::ImmutableKdTree::<f32, u32, 3, 32>::new_from_slice(
                &perturbed,
            );
        eprintln!("[kdtree immutable bulk]  {:?}", t.elapsed());

        let probe = perturbed[12345];
        let a = tree.nearest_n::<kiddo::SquaredEuclidean>(&probe, 512);
        let b = itree.nearest_n::<kiddo::SquaredEuclidean>(&probe, 512);
        let d0 = (a[0].distance - b[0].distance).abs();
        eprintln!(
            "[query parity] count {}/{}  nearest-dist diff={:e}",
            a.len(),
            b.len(),
            d0
        );
    }
}

#[cfg(test)]
mod bench_determinism {
    use super::*;

    /// Nondeterminism probe (2026-08-28): after the SDF/curvature
    /// parallelisation, the Sanji benchmark reported 1160 regions where the
    /// pre-parallel baseline reported 1274. Per-face float math is unchanged
    /// and bitwise-reproducible, so the suspect is PRE-EXISTING run-to-run
    /// nondeterminism: build_adjacency's `edge_to_face.values()` iterates a
    /// HashMap with a random per-instance order, so petgraph edge insertion
    /// order (and hence `edges(node)` order) differs between loads. Curvature
    /// and smoothing sum neighbour contributions in that order, and float
    /// addition is not associative — 1-ulp drift moves k-means boundaries.
    /// This probe loads the model twice (fresh HashMap per load) and runs the
    /// identical pipeline twice. If the label vectors differ, the variance is
    /// confirmed as pre-existing and independent of rayon.
    #[test]
    #[ignore]
    fn probe_segmentation_determinism() {
        let path = std::env::var("CYM_BENCH_STL")
            .unwrap_or_else(|_| "C:/selfDIr/Blender3D/Sanji+Diorama+Detailed_U1.stl".to_string());
        let algo = crate::segment::SegmentationAlgorithm::CurvatureKMeans {
            k: 6,
            smoothing_iters: 2,
            use_sdf: true,
            crease_threshold_deg: 45.0,
        };

        let mut m1 = load_stl(std::path::Path::new(&path), &|_, _| {}).expect("load 1");
        let s1 = crate::segment::run_segmentation(&mut m1, &algo, true, &|_, _| {});
        let labels1 = m1.segment_labels.clone();
        eprintln!("[probe run1] regions={}", s1.len());

        let mut m2 = load_stl(std::path::Path::new(&path), &|_, _| {}).expect("load 2");
        let s2 = crate::segment::run_segmentation(&mut m2, &algo, true, &|_, _| {});
        let labels2 = m2.segment_labels.clone();
        eprintln!("[probe run2] regions={}", s2.len());

        eprintln!(
            "[probe verdict] label vectors identical: {}",
            labels1 == labels2
        );
    }
}
