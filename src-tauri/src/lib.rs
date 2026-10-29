mod commands;
mod export;
mod mesh;
mod paint;
mod segment;

use commands::mesh::AppState;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // Initialize logging: RUST_LOG=debug cargo run
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info"))
        .format_timestamp_millis()
        .init();

    log::info!("=== ColorYourModel starting ===");

    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_fs::init())
        .plugin(tauri_plugin_opener::init())
        .manage(AppState::new())
        .manage(commands::update::UpdateState::new())
        .invoke_handler(tauri::generate_handler![
            commands::mesh::load_model,
            commands::mesh::get_face_color,
            commands::segment::auto_segment,
            commands::segment::auto_segment_smart,
            commands::segment::auto_segment_v2,
            commands::segment::paint_segment_face,
            commands::segment::finalize_segment,
            commands::segment::manual_region_add_point,
            commands::segment::finalize_manual_region,
            commands::segment::rename_segment,
            commands::segment::merge_segments,
            commands::segment::split_segment,
            commands::segment::resegment_region,
            commands::segment::seed_grow,
            commands::segment::recommend_seeds,
            commands::segment::detect_planar_regions,
            commands::segment::detect_eye_regions,
            commands::segment::detect_eye_regions_auto,
            commands::segment::detect_multiview_regions,
            commands::segment::detect_cross_section_features,
            commands::segment::fuse_segmentation,
            commands::segment::reset_segmentation,
            commands::paint::brush_paint,
            commands::paint::fill_paint,
            commands::paint::fill_segment_paint,
            commands::paint::spray_paint,
            commands::paint::smart_brush_paint,
            commands::paint::erase_paint,
            commands::paint::pick_color,
            // Colour panel "used colours" (issue #23).
            commands::paint::used_colors,
            // Colour find & replace (issue #29).
            commands::paint::replace_face_color,
            // Unified undo/redo — the single timeline covering paint, fill,
            // eraser and lasso. Replaced the former `manual_region_undo` and
            // `restore_face_colors` commands, both deleted in S2.
            commands::history::undo,
            commands::history::redo,
            commands::history::history_state,
            // History panel (issue #22): read-only listing + one-call jump.
            commands::history::history_entries,
            commands::history::history_jump,
            commands::export::export_3mf_command,
            commands::export::export_obj_command,
            commands::export::list_export_presets,
            commands::export::export_palette_preview,
            commands::js_bridge::report_js_error,
            commands::js_bridge::report_app_ready,
            // Update module: GitHub Releases check / download / passive install.
            commands::update::check_update,
            commands::update::download_update,
            commands::update::cancel_update_download,
            commands::update::discard_update,
            commands::update::install_update,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

#[cfg(test)]
mod repro_tests {
    use crate::mesh::model::MeshModel;

    /// Build a UV sphere whose triangles are arranged in latitude rings. Every
    /// triangle in a ring shares the SAME Y coordinate on its face center, so a
    /// KdTree that can't split a leaf with >BUCKET identical-axis points would
    /// panic on exactly this topology (the Sphere.stl import crash).
    fn make_uv_sphere(bands: u32, segs: u32) -> MeshModel {
        let mut m = MeshModel::new();
        let mut idx = vec![vec![0u32; segs as usize]; (bands + 1) as usize];
        for i in 0..=bands {
            let theta = (i as f32 / bands as f32) * std::f32::consts::PI;
            let y = theta.sin();
            let r = theta.cos();
            for j in 0..segs {
                let phi = (j as f32 / segs as f32) * 2.0 * std::f32::consts::PI;
                let x = r * phi.cos();
                let z = r * phi.sin();
                idx[i as usize][j as usize] = m.vertices.len() as u32;
                m.vertices.push([x, y, z]);
            }
        }
        for i in 0..bands {
            for j in 0..segs {
                let j2 = (j + 1) % segs;
                let a = idx[i as usize][j as usize];
                let b = idx[i as usize][j2 as usize];
                let c = idx[(i + 1) as usize][j as usize];
                let d = idx[(i + 1) as usize][j2 as usize];
                m.faces.push([a, c, b]);
                m.faces.push([b, c, d]);
            }
        }
        m
    }

    #[test]
    fn kdtree_coincident_axis_no_panic() {
        // 50 bands x 320 segs => 640 faces per latitude ring sharing one axis,
        // far beyond kiddo's default bucket of 32. Without the perturbation fix
        // this panics with "Too many items with the same position on one axis".
        let mut m = make_uv_sphere(50, 320);
        m.compute_normals();
        m.compute_bbox();
        m.init_default_colors();
        m.segment_labels = vec![0u32; m.faces.len()];
        m.build_kdtree();
        m.build_vertex_kdtree();
        m.build_adjacency();
        // Tree must be functional, not just non-panicking. The sphere has unit
        // radius, so all face centers lie within 2.0 of the origin.
        let near = m.faces_within_radius(&[0.0, 0.0, 0.0], 2.0);
        assert!(
            !near.is_empty(),
            "kdtree query should return faces near the origin"
        );
    }

    /// Flat YZ wall of 32x32 quads (2048 faces) whose vertices all sit at
    /// X=1232.0 — every face center shares that exact f32 X. Coordinates past
    /// ~1024 have ulp > the OLD fixed ±1e-4 kd_point jitter, so the offset was
    /// rounded away and kiddo re-panicked: the P0 dzqb9_U1.stl multi-model
    /// plate import crash (2026-10-12). The scale-aware jitter must separate
    /// the wall, keep the tree healthy (not degraded), and answer radius
    /// queries exactly like brute force.
    #[test]
    fn kdtree_large_offset_coplanar_wall_no_panic() {
        let mut m = MeshModel::new();
        let n = 33usize; // 32x32 quads -> 2048 triangles
        let step = 0.5f32;
        for i in 0..n {
            for j in 0..n {
                m.vertices.push([1232.0, i as f32 * step, j as f32 * step]);
            }
        }
        let v = |i: usize, j: usize| (i * n + j) as u32;
        for i in 0..n - 1 {
            for j in 0..n - 1 {
                let (a, b) = (v(i, j), v(i, j + 1));
                let (c, d) = (v(i + 1, j), v(i + 1, j + 1));
                m.faces.push([a, c, b]);
                m.faces.push([b, c, d]);
            }
        }
        m.compute_bbox();
        m.build_kdtree();
        m.build_vertex_kdtree();
        assert!(!m.face_kdtree_degraded, "healthy tree expected: the scale-aware jitter must separate the coplanar wall");

        // Caller contract: query candidates + true-distance filter == brute
        // force. The margin-widened query must never MISS an in-radius face.
        let probe = [1232.1, 6.13, 7.77];
        let radius = 2.4f32;
        let centers = m.face_centers();
        let true_dist = |i: usize| -> f32 {
            let c = centers[i];
            let dx = c[0] - probe[0];
            let dy = c[1] - probe[1];
            let dz = c[2] - probe[2];
            (dx * dx + dy * dy + dz * dz).sqrt()
        };
        let mut expected: Vec<u32> = (0..centers.len())
            .filter(|&i| true_dist(i) < radius)
            .map(|i| i as u32)
            .collect();
        expected.sort_unstable();
        let mut got: Vec<u32> = m
            .faces_within_radius(&probe, radius)
            .into_iter()
            .filter(|&f| true_dist(f as usize) < radius)
            .collect();
        got.sort_unstable();
        assert_eq!(got, expected, "radius query must match brute force");
    }

    /// A mesh too degenerate even for the scale-aware jitter (here: the flags
    /// are set directly to simulate the worst case) must still answer queries
    /// EXACTLY via the linear-scan fallback — degrade, never crash or corrupt.
    #[test]
    fn kdtree_degraded_fallback_queries_exact() {
        let mut m = MeshModel::new();
        let n = 9usize;
        for i in 0..n {
            for j in 0..n {
                m.vertices.push([1200.0, i as f32, j as f32]);
            }
        }
        let v = |i: usize, j: usize| (i * n + j) as u32;
        for i in 0..n - 1 {
            for j in 0..n - 1 {
                let (a, b) = (v(i, j), v(i, j + 1));
                let (c, d) = (v(i + 1, j), v(i + 1, j + 1));
                m.faces.push([a, c, b]);
                m.faces.push([b, c, d]);
            }
        }
        m.compute_bbox();
        m.build_kdtree();
        m.build_vertex_kdtree();
        m.face_kdtree_degraded = true;
        m.vertex_kdtree_degraded = true;

        // Radius parity with brute force.
        let probe = [1200.4, 3.2, 5.7];
        let radius = 2.1f32;
        let mut expected: Vec<u32> = m
            .face_centers()
            .iter()
            .enumerate()
            .filter(|(_, c)| {
                let dx = c[0] - probe[0];
                let dy = c[1] - probe[1];
                let dz = c[2] - probe[2];
                (dx * dx + dy * dy + dz * dz).sqrt() < radius
            })
            .map(|(i, _)| i as u32)
            .collect();
        expected.sort_unstable();
        let mut got = m.faces_within_radius(&probe, radius);
        got.sort_unstable();
        assert_eq!(got, expected, "degraded radius query must be exact");

        // Nearest vertex: exact index and TRUE (unjittered) SQUARED distance
        // — the SquaredEuclidean contract predates this fix, keep it.
        let (vi, vd) = m.nearest_vertex(&[1200.0, 4.9, 5.1]).expect("vertex");
        let q = m.vertices[vi as usize];
        let true_d2 = (q[1] - 4.9).powi(2) + (q[2] - 5.1).powi(2);
        assert_eq!(vi, v(5, 5), "nearest vertex index");
        assert!((vd - true_d2).abs() < 1e-6, "distance must be unjittered");

        // nearest_face_centers: exactly the K truly-nearest faces as a SET
        // (ordering is unspecified by contract — the only consumer scans all).
        let k = 4;
        let mut got_k = m.nearest_face_centers(&probe, k);
        got_k.sort_unstable();
        let mut scored: Vec<(f32, u32)> = m
            .face_centers()
            .iter()
            .enumerate()
            .map(|(i, c)| {
                let dx = c[0] - probe[0];
                let dy = c[1] - probe[1];
                let dz = c[2] - probe[2];
                (dx * dx + dy * dy + dz * dz, i as u32)
            })
            .collect();
        scored.sort_unstable_by(|a, b| a.0.partial_cmp(&b.0).unwrap());
        let mut expected_k: Vec<u32> = scored[..k].iter().map(|(_, i)| *i).collect();
        expected_k.sort_unstable();
        assert_eq!(got_k, expected_k, "degraded top-K must be the true top-K set");
    }
}
