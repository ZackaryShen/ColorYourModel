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
        .manage(AppState::new())
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
            commands::paint::brush_paint,
            commands::paint::fill_paint,
            commands::paint::fill_segment_paint,
            commands::paint::spray_paint,
            commands::paint::smart_brush_paint,
            commands::paint::erase_paint,
            commands::paint::pick_color,
            // Unified undo/redo — the single timeline covering paint, fill,
            // eraser and lasso. Replaced the former `manual_region_undo` and
            // `restore_face_colors` commands, both deleted in S2.
            commands::history::undo,
            commands::history::redo,
            commands::history::history_state,
            commands::export::export_3mf_command,
            commands::export::list_export_presets,
            commands::export::export_palette_preview,
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
}
