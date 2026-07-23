use crate::mesh::face_colors::mix_color;
use crate::mesh::kdtree::distance;
use crate::mesh::model::MeshModel;
use crate::paint::brush::{brush_hit, falloff_strength};

/// Smart brush: like regular brush but constrained to the same segment
pub fn smart_brush_hit(
    mesh: &mut MeshModel,
    center_face: u32,
    radius: f32,
    strength: f32,
    falloff_mode: &str,
    color: &[u8; 4],
) -> Vec<(u32, [u8; 4])> {
    let segment_id = if !mesh.segment_labels.is_empty() {
        mesh.segment_labels[center_face as usize]
    } else {
        return Vec::new();
    };

    let hits = brush_hit(mesh, center_face, radius);
    let center = mesh.face_center(center_face);
    let mut results = Vec::new();

    for fid in hits {
        // Only color faces in the same segment
        if mesh.segment_labels[fid as usize] != segment_id {
            continue;
        }
        let d = distance(&center, &mesh.face_center(fid));
        let s = falloff_strength(d, radius, falloff_mode) * strength;
        let new_color = mix_color(&mesh.face_colors[fid as usize], color, s);
        mesh.face_colors[fid as usize] = new_color;
        results.push((fid, new_color));
    }

    results
}
