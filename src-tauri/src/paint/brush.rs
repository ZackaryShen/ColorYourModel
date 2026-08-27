use crate::mesh::kdtree::distance;
use crate::mesh::model::MeshModel;

/// Get all face indices hit by a brush at the given center face with the given radius
pub fn brush_hit(mesh: &MeshModel, center_face: u32, radius: f32) -> Vec<u32> {
    let center = mesh.face_center(center_face);
    let candidates = mesh.faces_within_radius(&center, radius);
    let cand_count = candidates.len();
    let results: Vec<u32> = candidates
        .into_iter()
        .filter(|&fid| distance(&center, &mesh.face_center(fid)) < radius)
        .collect();

    // [DIAG-D] Log what the backend actually paints
    log::info!(
        "[DIAG-D] brush_hit: center_face={}, center=({:.1},{:.1},{:.1}), radius={:.2}, kdtree_candidates={}, filtered={}",
        center_face, center[0], center[1], center[2], radius,
        cand_count, results.len()
    );
    if !results.is_empty() {
        let first5: Vec<u32> = results.iter().take(5).copied().collect();
        log::info!("[DIAG-D]   first 5 face indices: {:?}", first5);
    }

    results
}

/// Compute falloff strength for a face at a given distance from brush center
pub fn falloff_strength(distance: f32, radius: f32, mode: &str) -> f32 {
    let t = (distance / radius).clamp(0.0, 1.0);
    match mode {
        "linear" => 1.0 - t,
        "smooth" => 1.0 - t * t * (3.0 - 2.0 * t),
        "step" => {
            if t < 0.5 {
                1.0
            } else {
                0.0
            }
        }
        _ => 1.0 - t,
    }
}
