use crate::mesh::face_colors::mix_color;
use crate::mesh::kdtree::distance;
use crate::mesh::model::MeshModel;
use crate::paint::brush::{brush_hit, falloff_strength};

/// Minimum cos(angle) between the clicked face normal and a candidate face
/// normal for the smart brush to paint it. cos 60° = 0.5 — a hemisphere of
/// surface orientations. See `smart_brush_hit` for why this matters.
const SMART_NORMAL_MIN_DOT: f32 = 0.5;

/// Smart brush: like regular brush but constrained to the same segment
pub fn smart_brush_hit(
    mesh: &mut MeshModel,
    center_face: u32,
    radius: f32,
    strength: f32,
    falloff_mode: &str,
    color: &[u8; 4],
) -> Vec<(u32, [u8; 4])> {
    let hits = brush_hit(mesh, center_face, radius);
    let center = mesh.face_center(center_face);
    // `segment_labels` is always length == face_count after load (iteration 18,
    // B1/B4 fix), so an unsegmented model reports label 0 for every face. The
    // smart brush therefore paints a LOCAL disk of the clicked segment (or the
    // whole model when unsegmented) instead of silently doing nothing.
    let segmented = !mesh.segment_labels.is_empty();
    let segment_id = if segmented {
        mesh.segment_labels[center_face as usize]
    } else {
        u32::MAX
    };
    // The kd-tree radius query is a 3D SPHERE around the clicked face center, so
    // on a thin wall (or between two nearby parts) it also returns faces on the
    // BACK side — the brush visibly "bleeds through" the model. That is the real
    // cause of "智能笔还是会超出界" (iteration 18, Issue 3): the segment filter
    // cannot catch it because the back faces belong to the SAME segment. Rejecting
    // candidates whose normal points away from the clicked face keeps the stroke
    // on the surface the user is actually looking at, at O(k) cost — no geodesic
    // BFS (which would reintroduce the iteration-16 stutter).
    let center_n = mesh.face_normal(center_face);
    let has_normal = center_n != [0.0, 0.0, 0.0];
    let mut results = Vec::new();

    for fid in hits {
        // Only color faces in the same segment (keeps the brush inside its part)
        if segmented && mesh.segment_labels[fid as usize] != segment_id {
            continue;
        }
        if has_normal {
            let n = mesh.face_normal(fid);
            let dot = center_n[0] * n[0] + center_n[1] * n[1] + center_n[2] * n[2];
            if dot < SMART_NORMAL_MIN_DOT {
                continue;
            }
        }
        let d = distance(&center, &mesh.face_center(fid));
        let s = falloff_strength(d, radius, falloff_mode) * strength;
        let new_color = mix_color(&mesh.face_colors[fid as usize], color, s);
        mesh.face_colors[fid as usize] = new_color;
        results.push((fid, new_color));
    }

    results
}
