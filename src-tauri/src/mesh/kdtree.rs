/// KD-Tree spatial index utilities
/// The actual KD-Tree is embedded in MeshModel via kiddo::KdTree

/// Compute euclidean distance between two 3D points
pub fn distance(a: &[f32; 3], b: &[f32; 3]) -> f32 {
    let dx = a[0] - b[0];
    let dy = a[1] - b[1];
    let dz = a[2] - b[2];
    (dx * dx + dy * dy + dz * dz).sqrt()
}
