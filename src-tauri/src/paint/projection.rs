//! Image-projection paint (0.2.0-P2, requirement 9): bake a picture onto the
//! model by orthographically projecting along a fixed axis and sampling the
//! image at each face centre.
//!
//! Deliberate v1 limits (documented in the plan): per-face single colour (no
//! barycentric interpolation — the mesh has no UVs), nearest-neighbour
//! sampling, and faces whose normal points AWAY from the projection axis are
//! culled (`normal · axis <= 0`), which resolves depth for convex models;
//! concave models can paint both walls of a pocket with the same pixel — an
//! accepted, documented limitation.

use crate::mesh::model::MeshModel;

/// Projection axis, STL-local. `+z` = "top-down" for the default import
/// orientation (Z-up).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Axis {
    /// Unit vector towards the viewer.
    pub dir: [f32; 3],
}

pub fn parse_axis(s: &str) -> Result<Axis, String> {
    let dir = match s {
        "+x" => [1.0, 0.0, 0.0],
        "-x" => [-1.0, 0.0, 0.0],
        "+y" => [0.0, 1.0, 0.0],
        "-y" => [0.0, -1.0, 0.0],
        "+z" => [0.0, 0.0, 1.0],
        "-z" => [0.0, 0.0, -1.0],
        _ => return Err(format!("project: unknown axis {s}")),
    };
    Ok(Axis { dir })
}

/// Sample `img` at normalized (u, v) with v flipped (image y grows downward).
fn sample_nearest(img: &image::RgbaImage, u: f32, v: f32) -> [u8; 4] {
    let (w, h) = img.dimensions();
    let x = ((u * (w - 1) as f32).round() as u32).min(w - 1);
    let y = ((v * (h - 1) as f32).round() as u32).min(h - 1);
    let p = img.get_pixel(x, y);
    [p[0], p[1], p[2], 255]
}

/// Project `img` onto the mesh along `axis` and return per-face colors for
/// every front-facing face whose centre falls inside the image footprint.
pub fn project_image_paint(
    mesh: &MeshModel,
    img: &image::RgbaImage,
    axis: Axis,
) -> Vec<(u32, [u8; 4])> {
    let bbox = &mesh.bbox;
    let span = [
        (bbox.max[0] - bbox.min[0]).max(1e-6),
        (bbox.max[1] - bbox.min[1]).max(1e-6),
        (bbox.max[2] - bbox.min[2]).max(1e-6),
    ];
    // The two in-plane axes: the ones with the smallest |dir| component.
    let mut axes: Vec<usize> = (0..3).collect();
    axes.sort_by_key(|&i| (axis.dir[i].abs() * 1000.0) as u32);
    let (u_ax, v_ax) = (axes[0], axes[1]);

    let mut updates = Vec::new();
    for (fi, tri) in mesh.faces.iter().enumerate() {
        let n = mesh.face_normal(fi as u32);
        if n[0] * axis.dir[0] + n[1] * axis.dir[1] + n[2] * axis.dir[2] <= 0.0 {
            continue; // back face — culled (depth resolution for convex models)
        }
        let c = mesh.face_center(fi as u32);
        let u = ((c[u_ax] - bbox.min[u_ax]) / span[u_ax]).clamp(0.0, 1.0);
        // v flip: image rows grow downward, world +axis grows "up" on screen.
        let v = (1.0 - (c[v_ax] - bbox.min[v_ax]) / span[v_ax]).clamp(0.0, 1.0);
        let color = sample_nearest(img, u, v);
        updates.push((fi as u32, color));
    }
    updates
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mesh::model::MeshModel;

    /// A single quad in the XY plane at z=0, normal +z (CCW winding).
    fn xy_quad() -> MeshModel {
        let mut m = MeshModel::new();
        m.vertices = vec![
            [0.0, 0.0, 0.0],
            [10.0, 0.0, 0.0],
            [10.0, 10.0, 0.0],
            [0.0, 10.0, 0.0],
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

    fn img_2x2() -> image::RgbaImage {
        // (0,0)=red top-left, (1,0)=green top-right, (0,1)=blue, (1,1)=white
        let mut img = image::RgbaImage::new(2, 2);
        img.get_pixel_mut(0, 0).0 = [255, 0, 0, 255];
        img.get_pixel_mut(1, 0).0 = [0, 255, 0, 255];
        img.get_pixel_mut(0, 1).0 = [0, 0, 255, 255];
        img.get_pixel_mut(1, 1).0 = [255, 255, 255, 255];
        img
    }

    #[test]
    fn plus_z_projects_top_down_with_v_flip() {
        let m = xy_quad();
        // Face centres: f0=(6.67,3.33), f1=(3.33,6.67) — u∈{0.67,0.33},
        // v flipped {0.67,0.33}: f0 → image (1,1) white, f1 → image (0,0) red.
        let updates = project_image_paint(&m, &img_2x2(), parse_axis("+z").unwrap());
        assert_eq!(updates.len(), 2, "both front faces painted");
        let by_face: std::collections::HashMap<u32, [u8; 4]> = updates.into_iter().collect();
        assert_eq!(by_face[&0], [255, 255, 255, 255], "f0 → white (u high, v flipped high)");
        assert_eq!(by_face[&1], [255, 0, 0, 255], "f1 → red (u low, v flipped low)");
    }

    #[test]
    fn minus_z_culls_everything_front_face_only() {
        let m = xy_quad();
        let updates = project_image_paint(&m, &img_2x2(), parse_axis("-z").unwrap());
        assert!(updates.is_empty(), "quad's normals are +z; -z sees only backs");
    }

    #[test]
    fn unknown_axis_is_an_error() {
        assert!(parse_axis("up").is_err());
        assert!(parse_axis("+z").is_ok());
    }
}
