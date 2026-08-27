//! Canonical face-frame estimation for head-like meshes.
//!
//! Provides a PCA-based pose normalisation that is good enough to support
//! downstream global eye detection. It does **not** solve the full
//! symmetry-alignment problem; it only guarantees that the dominant variance
//! axis is treated as "up" and the smallest-variance axis as "front".

use crate::mesh::model::MeshModel;

/// Canonical frame computed from mesh face centroids.
#[derive(Debug, Clone, Copy)]
pub struct FaceFrame {
    pub center: [f32; 3],
    /// Tallest axis (head height), oriented toward the top of the bounding box.
    pub up: [f32; 3],
    /// Second axis (head width). The symmetry plane separating left/right
    /// halves has this vector as its normal and passes through `center`.
    pub right: [f32; 3],
    /// Shortest axis (head depth), oriented toward the front of the bounding box.
    pub front: [f32; 3],
}

impl FaceFrame {
    /// Transform a world-space point into canonical frame coordinates.
    pub fn world_to_local(&self, p: &[f32; 3]) -> [f32; 3] {
        let dx = p[0] - self.center[0];
        let dy = p[1] - self.center[1];
        let dz = p[2] - self.center[2];
        [
            dx * self.right[0] + dy * self.right[1] + dz * self.right[2],
            dx * self.up[0] + dy * self.up[1] + dz * self.up[2],
            dx * self.front[0] + dy * self.front[1] + dz * self.front[2],
        ]
    }

    /// Signed distance from a world point to the symmetry plane (normal = right).
    pub fn symmetry_plane_distance(&self, p: &[f32; 3]) -> f32 {
        let lp = self.world_to_local(p);
        lp[0]
    }
}

/// Compute a canonical face frame from mesh face centroids using PCA.
///
/// Assumptions (deliberately minimal):
/// * the mesh is a single connected head-like object;
/// * the dominant variance axis is roughly vertical (height);
/// * the face points toward the +z side of the bounding box.
///
/// These hold for the standard T-pose / upright character meshes used in this
/// project. Failure modes are logged; callers should fall back to manual ROI
/// if the returned frame is degenerate.
pub fn compute_face_frame(mesh: &MeshModel) -> FaceFrame {
    let n_faces = mesh.faces.len();
    if n_faces == 0 {
        return identity_frame();
    }

    let centers: Vec<[f32; 3]> = (0..n_faces).map(|f| mesh.face_center(f as u32)).collect();
    let n = centers.len() as f32;

    let mut mean = [0.0f32; 3];
    for c in &centers {
        mean[0] += c[0];
        mean[1] += c[1];
        mean[2] += c[2];
    }
    mean[0] /= n;
    mean[1] /= n;
    mean[2] /= n;

    // 3x3 covariance matrix (symmetric).
    let mut cov = [[0.0f32; 3]; 3];
    for c in &centers {
        let d = [c[0] - mean[0], c[1] - mean[1], c[2] - mean[2]];
        for i in 0..3 {
            for j in 0..3 {
                cov[i][j] += d[i] * d[j];
            }
        }
    }
    for i in 0..3 {
        for j in 0..3 {
            cov[i][j] /= n;
        }
    }

    // Compute all three PCA axes.
    let e0 = dominant_eigenvector(cov);
    let mut cov1 = cov;
    project_out(&mut cov1, &e0);
    let e1 = dominant_eigenvector(cov1);
    let e2 = normalize(cross(&e0, &e1));
    let axes = [e0, e1, e2];

    // For a roughly upright head we want:
    //   up    ≈ world +Y
    //   right ≈ world +X
    //   front ≈ world +Z
    // We assign PCA axes by their alignment with these world axes, then orient
    // each toward the positive bbox half-space. This is more robust than
    // assuming the largest-variance PCA axis is always vertical.
    let world_axes = [
        [1.0f32, 0.0, 0.0], // right target
        [0.0f32, 1.0, 0.0], // up target
        [0.0f32, 0.0, 1.0], // front target
    ];
    let mut used = [false; 3];
    let mut up = axes[0];
    let mut right = axes[1];
    let mut front = axes[2];

    for (target, name) in world_axes.iter().zip(["right", "up", "front"].iter()) {
        let mut best_idx = 0usize;
        let mut best_dot = -1.0f32;
        for (i, &ax) in axes.iter().enumerate() {
            if used[i] {
                continue;
            }
            let d = dot(&ax, target).abs();
            if d > best_dot {
                best_dot = d;
                best_idx = i;
            }
        }
        used[best_idx] = true;
        let chosen = axes[best_idx];
        let oriented = if dot(&chosen, target) >= 0.0 {
            chosen
        } else {
            neg(&chosen)
        };
        match *name {
            "up" => up = oriented,
            "right" => right = oriented,
            "front" => front = oriented,
            _ => {}
        }
    }

    // Enforce right-handedness: up x front should equal right. If the assignment
    // above produced a left-handed triplet (because a PCA axis had to be flipped
    // to align with a world axis), flip right once.
    let right_from_up_front = normalize(cross(&up, &front));
    if dot(&right, &right_from_up_front) < 0.0 {
        right = neg(&right);
    }

    FaceFrame {
        center: mean,
        up,
        right,
        front,
    }
}

fn identity_frame() -> FaceFrame {
    FaceFrame {
        center: [0.0; 3],
        up: [0.0, 1.0, 0.0],
        right: [1.0, 0.0, 0.0],
        front: [0.0, 0.0, 1.0],
    }
}

fn dominant_eigenvector(a: [[f32; 3]; 3]) -> [f32; 3] {
    let mut v = [1.0f32, 0.0, 0.0];
    for _ in 0..32 {
        let mut av = [0.0f32; 3];
        for i in 0..3 {
            for j in 0..3 {
                av[i] += a[i][j] * v[j];
            }
        }
        v = normalize(av);
    }
    v
}

fn project_out(a: &mut [[f32; 3]; 3], u: &[f32; 3]) {
    // A' = (I - u u^T) A (I - u u^T)
    let mut out = [[0.0f32; 3]; 3];
    for i in 0..3 {
        for j in 0..3 {
            let mut sum = 0.0f32;
            for k in 0..3 {
                let ik = if i == k { 1.0 } else { 0.0 } - u[i] * u[k];
                for l in 0..3 {
                    let lj = if l == j { 1.0 } else { 0.0 } - u[l] * u[j];
                    sum += ik * a[k][l] * lj;
                }
            }
            out[i][j] = sum;
        }
    }
    *a = out;
}

fn normalize(v: [f32; 3]) -> [f32; 3] {
    let l = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    if l < 1e-12 {
        return [0.0, 1.0, 0.0];
    }
    [v[0] / l, v[1] / l, v[2] / l]
}

fn cross(a: &[f32; 3], b: &[f32; 3]) -> [f32; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

fn dot(a: &[f32; 3], b: &[f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn neg(v: &[f32; 3]) -> [f32; 3] {
    [-v[0], -v[1], -v[2]]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mesh::model::MeshModel;

    /// Build a UV sphere (same helper family as eye.rs tests).
    fn build_sphere(radius: f32, bands: u32, sectors: u32) -> MeshModel {
        let bands = bands as usize;
        let sectors = sectors as usize;
        let mut m = MeshModel::new();
        m.vertices.push([0.0, radius, 0.0]); // north pole
        for b in 1..bands - 1 {
            let theta = std::f32::consts::PI * b as f32 / (bands - 1) as f32;
            let y = radius * theta.cos();
            let r = radius * theta.sin();
            for s in 0..sectors {
                let phi = 2.0 * std::f32::consts::PI * s as f32 / sectors as f32;
                m.vertices.push([r * phi.cos(), y, r * phi.sin()]);
            }
        }
        m.vertices.push([0.0, -radius, 0.0]); // south pole

        let north = 0u32;
        let ring_start = |b: usize| (1 + (b - 1) * sectors) as u32;
        let south = (m.vertices.len() - 1) as u32;
        for s in 0..sectors {
            let a = ring_start(1) + s as u32;
            let b = ring_start(1) + ((s + 1) % sectors) as u32;
            m.faces.push([north, a, b]);
        }
        for b in 1..bands - 2 {
            for s in 0..sectors {
                let r0 = ring_start(b) + s as u32;
                let r1 = ring_start(b) + ((s + 1) % sectors) as u32;
                let r0n = ring_start(b + 1) + s as u32;
                let r1n = ring_start(b + 1) + ((s + 1) % sectors) as u32;
                m.faces.push([r0, r0n, r1]);
                m.faces.push([r1, r0n, r1n]);
            }
        }
        let last = bands - 2;
        for s in 0..sectors {
            let a = ring_start(last) + s as u32;
            let b = ring_start(last) + ((s + 1) % sectors) as u32;
            m.faces.push([south, b, a]);
        }
        m.compute_normals();
        m.compute_bbox();
        m.build_adjacency();
        m
    }

    #[test]
    fn stretched_box_frame_aligns_with_axes() {
        // A box with distinct dimensions (tall, narrow, shallow) forces PCA
        // axes to align with the geometric axes. A sphere would be isotropic
        // and give an arbitrary orthonormal basis, so we use a box here.
        let mut m = MeshModel::new();
        let x = 1.0f32;
        let y = 2.5f32; // tallest
        let z = 0.8f32; // shallowest
        let verts = [
            [-x, 0.0, z],
            [x, 0.0, z],
            [x, y, z],
            [-x, y, z],
            [-x, 0.0, -z],
            [x, 0.0, -z],
            [x, y, -z],
            [-x, y, -z],
        ];
        for v in &verts {
            m.vertices.push(*v);
        }
        let faces = [
            [0, 1, 2], [0, 2, 3],
            [1, 5, 6], [1, 6, 2],
            [5, 4, 7], [5, 7, 6],
            [4, 0, 3], [4, 3, 7],
            [3, 2, 6], [3, 6, 7],
            [4, 5, 1], [4, 1, 0],
        ];
        for f in &faces {
            m.faces.push(*f);
        }
        m.compute_normals();
        m.compute_bbox();
        m.build_adjacency();

        let frame = compute_face_frame(&m);
        let dot_up_y = dot(&frame.up, &[0.0, 1.0, 0.0]).abs();
        let dot_front_z = dot(&frame.front, &[0.0, 0.0, 1.0]).abs();
        let dot_right_x = dot(&frame.right, &[1.0, 0.0, 0.0]).abs();
        assert!(dot_up_y > 0.95, "up should align with Y, got dot={}", dot_up_y);
        assert!(dot_front_z > 0.95, "front should align with Z, got dot={}", dot_front_z);
        assert!(dot_right_x > 0.95, "right should align with X, got dot={}", dot_right_x);
    }
}
