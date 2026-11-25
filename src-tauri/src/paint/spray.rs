use crate::mesh::kdtree::distance;
use crate::mesh::model::MeshModel;
use crate::paint::brush::brush_hit;

/// Spray paint: randomly sample `density` faces within radius, each with random intensity.
/// Unlike brush (deterministic falloff), spray simulates a paint-can scatter pattern.
/// Returns the selected faces and their individual dab strengths.
pub fn spray_hit(
    mesh: &MeshModel,
    center_face: u32,
    radius: f32,
    strength: f32,
    density: u32,
) -> Vec<(u32, f32)> {
    let hits = brush_hit(mesh, center_face, radius);
    if hits.is_empty() {
        return Vec::new();
    }

    let center = mesh.face_center(center_face);

    // Sample up to `density` random faces from hits
    let sample_count = std::cmp::min(density as usize, hits.len());

    // Use Fisher-Yates partial shuffle to pick sample_count random elements
    let mut indices: Vec<usize> = (0..hits.len()).collect();
    let mut rng_state: u64 = (center_face as u64).wrapping_mul(6364136223846793005).wrapping_add(1);
    for i in 0..sample_count {
        // Simple LCG PRNG (deterministic per center_face for reproducibility)
        rng_state = rng_state.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        let j = i + ((rng_state >> 33) as usize % (indices.len() - i).max(1));
        indices.swap(i, j);
    }

    let mut results = Vec::with_capacity(sample_count);
    for i in 0..sample_count {
        let fid = hits[indices[i]];
        let d = distance(&center, &mesh.face_center(fid));

        // Random intensity: base falloff * random jitter [0.3, 1.0]
        let t = (d / radius).clamp(0.0, 1.0);
        let base_falloff = 1.0 - t * t; // quadratic falloff as base
        rng_state = rng_state.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        let jitter = 0.3 + 0.7 * ((rng_state >> 33) as f32 / (u32::MAX as f32));
        let s = base_falloff * jitter * strength;

        results.push((fid, s));
    }

    results
}
