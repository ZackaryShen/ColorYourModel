//! Layer 2 of the planar-region fusion study (`docs/09`): the **ray / plane
//! enhancement evidence channel**.
//!
//! Strategy (截面 marching — "marching plane" from docs/09 §1.2):
//!
//!   1. **切片** — for each principal axis (x, y, z) we march a plane
//!      `p[axis] = s` across the mesh at `planes_per_axis` evenly-spaced
//!      positions, staying just inside the bounding box (the very ends where the
//!      cross-section collapses to a point are the outer faces — already covered
//!      by Layer 1/Layer 3, so we sample the interior only).
//!   2. **轮廓** — every triangle straddling the slice contributes one line
//!      segment (its intersection with the plane). The union of these segments is
//!      the cross-section contour at that `s` (a direct, literal answer to the
//!      user's research question #2: "能否通过射线的方式，来确定…特征").
//!   3. **剖面指标** — we use the *total contour length* at each slice as the
//!      cross-sectional-profile metric. It is exact (sum of segment lengths,
//!      O(faces)) and its derivative peaks exactly where the shape changes fast.
//!      This is a robust, closed-loop-free proxy for the "面积对路径的导数"
//!      (cross-sectional-area derivative) criterion cited in docs/09 §1.2; the
//!      area and its length proxy share the same derivative signature at a
//!      feature, and length avoids the fragile contour-stitching a true shoelace
//!      area would require.
//!   4. **特征定位** — a slice is a *feature cross-section* when its metric
//!      derivative is a local maximum and exceeds `feature_threshold · max|dM/ds|`
//!      (a relative threshold so it scales with model size). Each feature yields
//!      one `CrossSectionRegion` holding the actual contour (`boundary_edges`)
//!      plus a `winding` inside/outside confidence from the generalized winding
//!      number (Jacobson 2013), which stays well-defined on *open* meshes where a
//!      parity / SDF test would break.
//!
//! This module is **evidence only**: it never mutates the mesh and its output is
//! purely a *visual* overlay (the slice contours), NOT a partition seed. A
//! cross-section is a plane, not a face, so folding it into `suggestedSeeds` →
//! `seed_grow` would be dishonest (docs/09 Layer 2 = "不裁决"). The user sees
//! the green feature cross-sections and decides; nothing reaches `seed_grow`.

use serde::{Deserialize, Serialize};

use crate::mesh::model::MeshModel;

/// Tunable knobs for [`detect_cross_section_features`].
#[derive(Debug, Clone, Copy)]
pub struct CrossSectionParams {
    /// Number of marcher slices per principal axis (x, y, z all get this many).
    /// More slices = finer feature localisation, at O(axes · slices · faces).
    /// Default 24.
    pub planes_per_axis: usize,
    /// A slice is reported as a feature iff its profile-derivative magnitude is
    /// ≥ `feature_threshold` × the axis's maximum derivative magnitude. 0 = every
    /// local-max derivative bump (very noisy); 1 = only the single sharpest
    /// change per axis. Default 0.5 (report changes ≥ half the sharpest one).
    pub feature_threshold: f32,
}

impl Default for CrossSectionParams {
    fn default() -> Self {
        Self {
            planes_per_axis: 24,
            feature_threshold: 0.5,
        }
    }
}

/// One detected feature cross-section (a plane where the shape changes sharply).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CrossSectionRegion {
    /// The slice plane `[a, b, c, d]` with `a·x + b·y + c·z + d = 0`, axis-aligned
    /// (exactly one of a/b/c is 1, d = −position).
    pub plane: [f32; 4],
    /// Which axis was marched: 0 = x, 1 = y, 2 = z.
    pub axis: u8,
    /// Slice coordinate along `axis` (the `s` in `p[axis] = s`).
    pub position: f32,
    /// Cross-sectional profile metric at this slice (total contour length). Larger
    /// = bigger cross-section at the feature.
    pub area_metric: f32,
    /// Generalized winding number sampled at the slice centre: ≈ +1 inside a
    /// closed solid, ≈ 0 outside / near open edges (Jacobson 2013). An
    /// *advisory* inside/outside confidence, nothing more.
    pub winding: f32,
    /// The cross-section contour as 3D line segments `[[x,y,z],[x,y,z]]`, drawn as
    /// `lineSegments` (the literal "截面" outline).
    pub boundary_edges: Vec<[[f32; 3]; 2]>,
}

// ─── vector helpers ────────────────────────────────────────────────────
#[inline]
fn sub(a: &[f32; 3], b: &[f32; 3]) -> [f32; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
#[inline]
fn dot(a: &[f32; 3], b: &[f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
#[inline]
fn cross(a: &[f32; 3], b: &[f32; 3]) -> [f32; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}
#[inline]
fn len(a: &[f32; 3]) -> f32 {
    (a[0] * a[0] + a[1] * a[1] + a[2] * a[2]).max(1e-12).sqrt()
}
#[inline]
fn dist(a: &[f32; 3], b: &[f32; 3]) -> f32 {
    len(&sub(a, b))
}

/// Clip one triangle `[v0,v1,v2]` against the plane `p[axis] = s`.
///
/// Returns the (at most one) intersection segment as two 3D points, or `None`
/// if the triangle does not cross the plane. Degenerate on-plane vertices are
/// absorbed as segment endpoints so a triangle with one vertex on the plane and
/// the other two on opposite sides still yields its correct segment.
fn tri_plane_segment(v: &[[f32; 3]; 3], s: f32, axis: usize) -> Option<([f32; 3], [f32; 3])> {
    let d = [v[0][axis] - s, v[1][axis] - s, v[2][axis] - s];
    let eps = 1e-7f32;
    let mut pts: Vec<[f32; 3]> = Vec::with_capacity(3);
    for k in 0..3 {
        let i = k;
        let j = (k + 1) % 3;
        let (di, dj) = (d[i], d[j]);
        // strict sign change → one interior intersection point on edge (i, j)
        if (di < -eps && dj > eps) || (di > eps && dj < -eps) {
            let t = di / (di - dj);
            pts.push([
                v[i][0] + t * (v[j][0] - v[i][0]),
                v[i][1] + t * (v[j][1] - v[i][1]),
                v[i][2] + t * (v[j][2] - v[i][2]),
            ]);
        }
        // vertex exactly on the plane → include it (forms the segment with the
        // crossing point from the adjacent edge)
        if di.abs() <= eps {
            pts.push(v[i]);
        }
        if dj.abs() <= eps {
            pts.push(v[j]);
        }
    }
    // de-duplicate coincident points
    let mut out: Vec<[f32; 3]> = Vec::with_capacity(pts.len());
    for p in pts {
        if !out.iter().any(|q| dist(q, &p) < 1e-6) {
            out.push(p);
        }
    }
    if out.len() >= 2 {
        Some((out[0], out[1]))
    } else {
        None
    }
}

/// All contour segments of the whole mesh at slice `p[axis] = s`.
fn slice_contour(mesh: &MeshModel, s: f32, axis: usize) -> Vec<([f32; 3], [f32; 3])> {
    let mut segs = Vec::new();
    for tri in &mesh.faces {
        let v = [
            mesh.vertices[tri[0] as usize],
            mesh.vertices[tri[1] as usize],
            mesh.vertices[tri[2] as usize],
        ];
        if let Some(seg) = tri_plane_segment(&v, s, axis) {
            segs.push(seg);
        }
    }
    segs
}

/// Bounding box `[min, max]` of the mesh vertices.
fn bbox(mesh: &MeshModel) -> ([f32; 3], [f32; 3]) {
    if mesh.vertices.is_empty() {
        return ([0.0; 3], [0.0; 3]);
    }
    let mut mn = [f32::MAX; 3];
    let mut mx = [f32::MIN; 3];
    for v in &mesh.vertices {
        for i in 0..3 {
            mn[i] = mn[i].min(v[i]);
            mx[i] = mx[i].max(v[i]);
        }
    }
    (mn, mx)
}

/// Signed solid angle (steradians, in [−2π, 2π]) subtended by triangle
/// (a, b, c) at point `p` — the building block of the generalized winding
/// number (Jacobson et al. 2013, TOG 32(4)). Uses the robust atan2 form so it
/// degrades gracefully on non-closed / open meshes.
fn signed_solid_angle(a: &[f32; 3], b: &[f32; 3], c: &[f32; 3], p: &[f32; 3]) -> f32 {
    let ba = sub(a, p);
    let bb = sub(b, p);
    let bc = sub(c, p);
    let la = len(&ba);
    let lb = len(&bb);
    let lc = len(&bc);
    let det = dot(&ba, &cross(&bb, &bc));
    let denom = la * lb * lc
        + dot(&ba, &bb) * lc
        + dot(&bb, &bc) * la
        + dot(&bc, &ba) * lb;
    // Van Oosterom & Strackee: tan(ω/2) = det / denom, so the solid angle is
    // ω = 2·atan2(det, denom). (Forgetting the ×2 is the classic off-by-half bug
    // that yields winding ≈ 0.5 for a closed cube.)
    2.0 * det.atan2(denom)
}

/// Generalized winding number of `p` w.r.t. the mesh: Σ solid-angle / 4π.
/// ≈ +1 deep inside a closed surface, ≈ 0 far outside, smooth (and fractional)
/// everywhere else — unlike parity / SDF it stays well-defined on open meshes.
pub fn generalized_winding_number(mesh: &MeshModel, p: [f32; 3]) -> f32 {
    let mut sum = 0.0f32;
    for tri in &mesh.faces {
        let a = mesh.vertices[tri[0] as usize];
        let b = mesh.vertices[tri[1] as usize];
        let c = mesh.vertices[tri[2] as usize];
        sum += signed_solid_angle(&a, &b, &c, &p);
    }
    sum / (4.0 * std::f32::consts::PI)
}

/// Detect feature cross-sections by marching a plane along each principal axis.
/// See module docs for the algorithm. Returns an *advisory, visual-only* set of
/// cross-section contours (docs/09 Layer 2 = evidence, never a verdict).
pub fn detect_cross_section_features(
    mesh: &MeshModel,
    params: &CrossSectionParams,
) -> Vec<CrossSectionRegion> {
    let n = mesh.faces.len();
    if n == 0 || mesh.vertices.is_empty() {
        return Vec::new();
    }
    let k = params.planes_per_axis.max(2);
    let thr = params.feature_threshold.clamp(0.0, 1.0);
    let (mn, mx) = bbox(mesh);
    let center = [
        (mn[0] + mx[0]) / 2.0,
        (mn[1] + mx[1]) / 2.0,
        (mn[2] + mx[2]) / 2.0,
    ];

    let mut out: Vec<CrossSectionRegion> = Vec::new();

    for axis in 0..3 {
        let lo = mn[axis];
        let hi = mx[axis];
        let span = (hi - lo).max(1e-9);
        // Sample just inside [lo, hi]; s[t] = lo + span * (t + 0.5) / k.
        let s_vals: Vec<f32> = (0..k)
            .map(|t| lo + span * ((t as f32) + 0.5) / (k as f32))
            .collect();
        let ds = span / (k as f32); // uniform spacing

        // Profile: total contour length at each slice.
        let mut metric = vec![0.0f32; k];
        for (t, &s) in s_vals.iter().enumerate() {
            let segs = slice_contour(mesh, s, axis);
            let l: f32 = segs.iter().map(|(a, b)| dist(a, b)).sum();
            metric[t] = l;
        }

        // Derivative profile (central difference on interior samples).
        let mut deriv = vec![0.0f32; k];
        for t in 1..k - 1 {
            deriv[t] = (metric[t + 1] - metric[t - 1]) / (2.0 * ds);
        }
        let max_mag = deriv.iter().map(|d| d.abs()).fold(0.0f32, |a, b| a.max(b));
        if max_mag < 1e-6 {
            continue; // uniform cross-section along this axis → no interior feature
        }

        // Local maxima of |deriv| above the relative threshold = feature slices.
        for t in 1..k - 1 {
            let mag = deriv[t].abs();
            if mag < thr * max_mag {
                continue;
            }
            let prev = deriv[t - 1].abs();
            let next = if t + 1 < k { deriv[t + 1].abs() } else { 0.0 };
            if prev <= mag && mag >= next {
                let s = s_vals[t];
                let segs = slice_contour(mesh, s, axis);
                let mut plane = [0.0f32; 4];
                plane[axis] = 1.0;
                plane[3] = -s;
                // Inside/outside confidence at the slice centre.
                let mut probe = center;
                probe[axis] = s;
                let winding = generalized_winding_number(mesh, probe);
                let mut boundary_edges = Vec::with_capacity(segs.len());
                for (a, b) in segs {
                    boundary_edges.push([a, b]);
                }
                out.push(CrossSectionRegion {
                    plane,
                    axis: axis as u8,
                    position: s,
                    area_metric: metric[t],
                    winding,
                    boundary_edges,
                });
            }
        }
    }

    out.sort_by(|a, b| {
        a.axis
            .cmp(&b.axis)
            .then_with(|| a.position.partial_cmp(&b.position).unwrap_or(std::cmp::Ordering::Equal))
    });
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mesh::model::MeshModel;
    use crate::segment::metrics::{build_mesh, unit_cube};

    /// Axis-aligned box as 12 triangles, mirroring `unit_cube`'s layout.
    fn aabb(min: [f32; 3], max: [f32; 3]) -> (Vec<[f32; 3]>, Vec<u32>) {
        let v = vec![
            [min[0], min[1], min[2]],
            [max[0], min[1], min[2]],
            [max[0], max[1], min[2]],
            [min[0], max[1], min[2]],
            [min[0], min[1], max[2]],
            [max[0], min[1], max[2]],
            [max[0], max[1], max[2]],
            [min[0], max[1], max[2]],
        ];
        let f = vec![
            0, 3, 2, 0, 2, 1, // -Z
            4, 5, 6, 4, 6, 7, // +Z
            0, 1, 5, 0, 5, 4, // -Y
            3, 7, 6, 3, 6, 2, // +Y
            0, 4, 7, 0, 7, 3, // -X
            1, 2, 6, 1, 6, 5, // +X
        ];
        (v, f)
    }

    /// Two stacked boxes with a step: along Y the cross-section jumps from
    /// width 2 (y∈[0,1]) to width 1 (y∈[1,2]) at y=1 → one interior feature.
    fn stepped_mesh() -> MeshModel {
        let (v1, f1) = aabb([0.0, 0.0, 0.0], [2.0, 1.0, 1.0]);
        let (v2, f2) = aabb([0.0, 1.0, 0.0], [1.0, 2.0, 1.0]);
        let mut v = v1;
        let mut f = f1;
        let off = (v.len()) as u32;
        for p in v2 {
            v.push(p);
        }
        for idx in f2 {
            f.push(idx + off);
        }
        build_mesh(&v, &f)
    }

    fn params() -> CrossSectionParams {
        CrossSectionParams {
            planes_per_axis: 40,
            feature_threshold: 0.4,
        }
    }

    #[test]
    fn empty_mesh_is_safe() {
        let mesh = MeshModel::new();
        assert!(detect_cross_section_features(&mesh, &CrossSectionParams::default()).is_empty());
    }

    #[test]
    fn unit_cube_has_no_interior_cross_section_feature() {
        // A cube has a constant cross-section along every interior slice → no
        // interior feature (its only features are the 6 outer faces, already
        // handled by Layer 1 / Layer 3). This is the honest, expected result.
        let mesh = unit_cube();
        let regions = detect_cross_section_features(&mesh, &params());
        assert!(
            regions.is_empty(),
            "a plain cube should yield 0 interior cross-section features, got {}",
            regions.len()
        );
    }

    #[test]
    fn stepped_mesh_reports_a_y_axis_feature() {
        let mesh = stepped_mesh();
        let regions = detect_cross_section_features(&mesh, &params());
        // Expect at least one feature on the Y axis (the step at y=1).
        let y_features = regions.iter().filter(|r| r.axis == 1).count();
        assert!(
            y_features >= 1,
            "stepped mesh should expose ≥1 Y-axis feature at the step, got {y_features} (total {})",
            regions.len()
        );
        // Every feature must carry a non-empty contour (the actual 截面 outline).
        for r in &regions {
            assert!(
                !r.boundary_edges.is_empty(),
                "feature cross-section must have a contour"
            );
            for e in &r.boundary_edges {
                assert!(e[0].iter().all(|x| x.is_finite()) && e[1].iter().all(|x| x.is_finite()));
            }
        }
    }

    #[test]
    fn feature_position_lands_near_the_step() {
        let mesh = stepped_mesh();
        let regions = detect_cross_section_features(&mesh, &params());
        let y = regions.iter().find(|r| r.axis == 1).expect("Y feature present");
        assert!(
            (y.position - 1.0).abs() < 0.2,
            "Y feature should sit near the step at y=1, got y={}",
            y.position
        );
    }

    #[test]
    fn generalized_winding_number_inside_vs_outside() {
        let mesh = unit_cube();
        let inside = generalized_winding_number(&mesh, [0.0, 0.0, 0.0]);
        let outside = generalized_winding_number(&mesh, [5.0, 5.0, 5.0]);
        assert!(
            (inside - 1.0).abs() < 1e-3,
            "winding at cube centre should be ≈ +1, got {inside}"
        );
        assert!(
            outside.abs() < 1e-3,
            "winding far outside should be ≈ 0, got {outside}"
        );
    }
}
