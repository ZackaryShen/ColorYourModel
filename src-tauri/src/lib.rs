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
            commands::segment::paint_segment_face,
            commands::segment::finalize_segment,
            commands::segment::manual_region_add_point,
            commands::segment::finalize_manual_region,
            commands::segment::manual_region_undo,
            commands::segment::restore_face_colors,
            commands::paint::brush_paint,
            commands::paint::fill_paint,
            commands::paint::fill_segment_paint,
            commands::paint::spray_paint,
            commands::paint::smart_brush_paint,
            commands::paint::erase_paint,
            commands::paint::pick_color,
            commands::export::export_3mf_command,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
