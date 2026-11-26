//! Refine external 3MF surfaces before the editor assigns per-face state.

use std::collections::HashMap;

use super::{subdivide::Expanded, GeometryBudget};

// Coarse triangles can be wider than a default 2 mm-radius stroke. Refine
// them down to 1 mm edges, so a whole-face paint step follows that footprint.
const COARSE_EDGE_SQUARED: f64 = 4.0 * 4.0;
const TARGET_EDGE_SQUARED: f64 = 1.0;
// Bound the extra render and spatial-index work on already large plates.
const MAX_ADDED_FACES: usize = 200_000;
const UNSPLIT: u32 = u32::MAX;

pub(super) fn for_paint(mut mesh: Expanded, budget: GeometryBudget) -> Expanded {
    let mut midpoints = std::mem::take(&mut mesh.midpoints);
    let face_limit = budget.faces.min(mesh.faces.len() + MAX_ADDED_FACES);
    if mesh.faces.len() >= face_limit
        || !mesh.faces.iter().any(|face| {
            (0..3).any(|side| {
                let (a, b) = edge(*face, side);
                length_squared(&mesh.vertices, a, b) > COARSE_EDGE_SQUARED
            })
        })
    {
        return mesh;
    }

    loop {
        let vertex_count = mesh.vertices.len();
        let Some((cuts, face_count)) =
            plan_pass(&mut mesh, &mut midpoints, face_limit, budget.vertices)
        else {
            // A complete pass uses identical edge splits on both sides. Keep
            // the previous pass when the next one would exceed the budget.
            mesh.vertices.truncate(vertex_count);
            return mesh;
        };
        let mut faces = Vec::with_capacity(face_count);
        let mut slots = Vec::with_capacity(face_count);
        for ((&face, &slot), cuts) in mesh.faces.iter().zip(&mesh.slots).zip(cuts) {
            let old_len = faces.len();
            split_face(&mesh.vertices, face, cuts, &mut faces);
            slots.resize(slots.len() + faces.len() - old_len, slot);
        }
        mesh.faces = faces;
        mesh.slots = slots;
    }
}

fn edge(face: [u32; 3], side: usize) -> (u32, u32) {
    let a = face[side];
    let b = face[(side + 1) % 3];
    (a.min(b), a.max(b))
}

fn length_squared(vertices: &[[f32; 3]], a: u32, b: u32) -> f64 {
    let [a, b] = [a, b].map(|i| vertices[i as usize]);
    (0..3)
        .map(|i| (f64::from(a[i]) - f64::from(b[i])).powi(2))
        .sum()
}

fn plan_pass(
    mesh: &mut Expanded,
    midpoints: &mut HashMap<(u32, u32), u32>,
    face_limit: usize,
    vertex_limit: usize,
) -> Option<(Vec<[u32; 3]>, usize)> {
    let mut face_count = mesh.faces.len();
    let mut cuts = Vec::with_capacity(face_count);
    for &face in &mesh.faces {
        let mut face_cuts = [UNSPLIT; 3];
        for (side, cut) in face_cuts.iter_mut().enumerate() {
            let (a, b) = edge(face, side);
            if let Some(&index) = midpoints.get(&(a, b)) {
                // Native trees can split only one of two neighbours. Reuse
                // their vertices, including deeper splits in the next pass.
                *cut = index;
            } else if length_squared(&mesh.vertices, a, b) > TARGET_EDGE_SQUARED {
                let [a_point, b_point] = [a, b].map(|i| mesh.vertices[i as usize]);
                let mid = std::array::from_fn(|i| a_point[i].midpoint(b_point[i]));
                if mid == a_point || mid == b_point {
                    continue;
                }
                if mesh.vertices.len() == vertex_limit {
                    return None;
                }
                *cut = mesh.vertices.len() as u32;
                mesh.vertices.push(mid);
                midpoints.insert((a, b), *cut);
            }
            face_count += usize::from(*cut != UNSPLIT);
            if face_count > face_limit {
                return None;
            }
        }
        cuts.push(face_cuts);
    }
    (face_count > mesh.faces.len()).then_some((cuts, face_count))
}

fn split_face(
    points: &[[f32; 3]],
    mut face: [u32; 3],
    mut cuts: [u32; 3],
    output: &mut Vec<[u32; 3]>,
) {
    match cuts.iter().filter(|&&index| index != UNSPLIT).count() {
        0 => output.push(face),
        1 => {
            let side = cuts.iter().position(|&index| index != UNSPLIT).unwrap();
            face.rotate_left(side);
            let [a, b, c] = face;
            let mid = cuts[side];
            output.extend([[a, mid, c], [mid, b, c]]);
        }
        2 => {
            let side = (cuts.iter().position(|&index| index == UNSPLIT).unwrap() + 1) % 3;
            face.rotate_left(side);
            cuts.rotate_left(side);
            let [a, b, c] = face;
            let [ab, bc, _] = cuts;
            output.push([ab, b, bc]);
            // Use the shorter diagonal in the remaining quadrilateral.
            if length_squared(points, a, bc) < length_squared(points, ab, c) {
                output.extend([[a, ab, bc], [a, bc, c]]);
            } else {
                output.extend([[a, ab, c], [ab, bc, c]]);
            }
        }
        _ => {
            let [a, b, c] = face;
            let [ab, bc, ca] = cuts;
            output.extend([[a, ab, ca], [ab, b, bc], [ca, bc, c], [ab, bc, ca]]);
        }
    }
}
