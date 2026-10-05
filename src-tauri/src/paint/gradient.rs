//! Radial-gradient paint selector (0.2.0-P2, requirement 7).
//!
//! Like every module in `paint/`, this is a pure selector: it computes
//! `(face, color)` pairs without touching the buffers, so the caller funnels
//! the result through `apply_paint` and gets undo for free.
//!
//! Interpolation: `t = clamp(distance / radius, 0, 1)`, color =
//! `lerp(inner, outer, t)` — center paints `inner`, the rim paints `outer`.
//! Deliberately NOT `falloff_strength` (that returns center=1/edge=0 strength
//! for alpha mixing; reusing it here would swap the two colors).

use crate::mesh::kdtree::distance;
use crate::mesh::model::MeshModel;

/// Faces within `radius` of `center_face`, colored along an inner→outer ramp.
pub fn gradient_radial_hit(
    mesh: &MeshModel,
    center_face: u32,
    radius: f32,
    color_inner: [u8; 4],
    color_outer: [u8; 4],
) -> Vec<(u32, [u8; 4])> {
    let center = mesh.face_center(center_face);
    let candidates = mesh.faces_within_radius(&center, radius);
    let mut updates = Vec::with_capacity(candidates.len());
    for fid in candidates {
        let fc = mesh.face_center(fid);
        let d = distance(&center, &fc);
        let t = (d / radius).clamp(0.0, 1.0);
        let color = [
            (color_inner[0] as f32 + (color_outer[0] as f32 - color_inner[0] as f32) * t) as u8,
            (color_inner[1] as f32 + (color_outer[1] as f32 - color_inner[1] as f32) * t) as u8,
            (color_inner[2] as f32 + (color_outer[2] as f32 - color_inner[2] as f32) * t) as u8,
            255,
        ];
        updates.push((fid, color));
    }
    updates
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mesh::model::MANUAL_SEGMENT_OFFSET;

    fn quad_mesh() -> MeshModel {
        // Single 20x20 quad in the XY plane (two triangles).
        let mut m = MeshModel::new();
        m.vertices = vec![
            [0.0, 0.0, 0.0],
            [20.0, 0.0, 0.0],
            [20.0, 20.0, 0.0],
            [0.0, 20.0, 0.0],
        ];
        m.faces = vec![[0, 1, 2], [0, 2, 3]];
        m.compute_normals();
        m.compute_bbox();
        m.init_default_colors();
        m.segment_labels = vec![0; m.faces.len()];
        m.build_kdtree();
        m.build_vertex_kdtree();
        m.build_adjacency();
        m
    }

    const INNER: [u8; 4] = [255, 0, 0, 255];
    const OUTER: [u8; 4] = [0, 0, 255, 255];

    #[test]
    fn center_face_gets_inner_and_far_face_gets_outer() {
        let m = quad_mesh();
        // Face centers: t0 ≈ (6.7,6.7), t1 ≈ (13.3,13.3). Radius 12 covers
        // both; t1 = 9.33/12 ≈ 0.78 (near the rim → leans OUTER/blue).
        let updates = gradient_radial_hit(&m, 0, 12.0, INNER, OUTER);
        assert_eq!(updates.len(), 2);
        let by_face: std::collections::HashMap<u32, [u8; 4]> = updates.into_iter().collect();
        let c0 = by_face[&0];
        let c1 = by_face[&1];
        // Face 0 is nearer the center → closer to INNER (more red than blue).
        assert!(c0[0] > c0[2], "near face should lean red: {:?}", c0);
        // Face 1 is farther → closer to OUTER (more blue than red).
        assert!(c1[2] > c1[0], "far face should lean blue: {:?}", c1);
        assert_eq!(c0[3], 255);
    }

    #[test]
    fn tiny_radius_colors_only_the_center_face() {
        let m = quad_mesh();
        let updates = gradient_radial_hit(&m, 0, 2.0, INNER, OUTER);
        // Radius 2 covers nothing but the immediate neighborhood of face 0's
        // center — face 1's center is ~9.4mm away.
        assert_eq!(updates.len(), 1);
        assert_eq!(updates[0].0, 0);
        assert_eq!(updates[0].1, INNER);
    }

    #[test]
    fn interpolation_is_monotonic_in_distance() {
        // A 1D strip of faces at increasing distance: t (and thus the blend)
        // must be monotonic — regression for the falloff_strength swap trap.
        let mut m = MeshModel::new();
        for i in 0..5 {
            m.vertices.push([i as f32 * 4.0, 0.0, 0.0]);
            m.vertices.push([i as f32 * 4.0 + 4.0, 0.0, 0.0]);
            m.vertices.push([i as f32 * 4.0 + 2.0, 3.0, 0.0]);
        }
        for i in 0..5 {
            let a = (i * 3) as u32;
            m.faces.push([a, a + 1, a + 2]);
        }
        m.compute_normals();
        m.compute_bbox();
        m.init_default_colors();
        m.segment_labels = vec![0; m.faces.len()];
        m.build_kdtree();
        m.build_vertex_kdtree();
        m.build_adjacency();
        let _ = MANUAL_SEGMENT_OFFSET; // silence unused import in some configs

        let updates = gradient_radial_hit(&m, 0, 25.0, INNER, OUTER);
        let mut reds: Vec<(u32, u8)> = updates
            .iter()
            .map(|(f, c)| (*f, c[0]))
            .collect();
        reds.sort_by_key(|(f, _)| *f);
        // Red channel (INNER) must be non-increasing with face distance.
        for w in reds.windows(2) {
            assert!(w[0].1 >= w[1].1, "non-monotonic blend: {:?}", reds);
        }
    }
}
