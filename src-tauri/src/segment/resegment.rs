//! Re-segment a single existing region with any algorithm.
//!
//! The selected region's faces are extracted into a sub-mesh, run through the
//! unified `run_segmentation` pipeline (so every algorithm available globally is
//! also available locally), and the result is mapped back onto fresh manual
//! labels — letting the user iteratively refine just the part an automatic pass
//! (V-HACD, concavity, …) left as one blob. This is the escape hatch for the
//! classic failure mode on armoured characters: the convex decomposition cannot
//! split a near-convex tubular limb because it has no concave seam to cut along,
//! so the user selects that limb and re-runs a finer algorithm on it alone, where
//! the bend (elbow / knee / wrist) finally becomes a meaningful cut.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::mesh::history::OpKind;
use crate::mesh::loader::ProgressFn;
use crate::mesh::model::{MeshModel, Segment};
use crate::segment::{run_segmentation, SegmentationAlgorithm};

/// Result of re-segmenting one region. Mirrors [`crate::commands::segment::SegmentResult`]
/// enough that the frontend can reuse `updateSegmentLabels`, plus `base_label` /
/// `region_count` so the panel can highlight the new pieces.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResegmentResult {
    pub segments: Vec<Segment>,
    pub segment_labels: Vec<u32>,
    pub moved_faces: usize,
    /// First freshly allocated manual label (the largest sub-region lands here);
    /// convenient for re-selecting after the operation.
    pub base_label: u32,
    /// Number of sub-regions the algorithm produced inside the target region.
    pub region_count: u32,
}

/// Re-run `algorithm` on the faces of `label` only, replacing that region with
/// the resulting sub-regions.
pub fn resegment_region(
    mesh: &mut MeshModel,
    label: u32,
    algorithm: &SegmentationAlgorithm,
    progress: &ProgressFn,
) -> Result<ResegmentResult, String> {
    if !mesh.segments.contains_key(&label) {
        return Err(format!("Segment {} does not exist", label));
    }

    // 1. Collect the faces of the target region.
    let region_faces: Vec<u32> = (0..mesh.segment_labels.len() as u32)
        .filter(|&i| mesh.segment_labels[i as usize] == label)
        .collect();
    if region_faces.is_empty() {
        return Err(format!("Segment {} has no faces", label));
    }

    // 2. Build a sub-mesh containing only those faces (and their vertices).
    //    Vertex indices are re-numbered into the compact sub-mesh; global
    //    coordinates are preserved so geometric algorithms behave identically.
    let mut old_to_new: HashMap<u32, u32> = HashMap::new();
    let mut new_vertices: Vec<[f32; 3]> = Vec::with_capacity(region_faces.len());
    let mut new_faces: Vec<[u32; 3]> = Vec::with_capacity(region_faces.len());
    for &f in &region_faces {
        let tri = mesh.faces[f as usize];
        let mut new_tri = [0u32; 3];
        for (slot, &v) in tri.iter().enumerate() {
            let nv = if let Some(&n) = old_to_new.get(&v) {
                n
            } else {
                let n = new_vertices.len() as u32;
                old_to_new.insert(v, n);
                new_vertices.push(mesh.vertices[v as usize]);
                n
            };
            new_tri[slot] = nv;
        }
        new_faces.push(new_tri);
    }

    let mut sub = MeshModel::new();
    sub.vertices = new_vertices;
    sub.faces = new_faces;
    sub.segment_labels = vec![0u32; region_faces.len()];
    sub.compute_normals();
    sub.build_adjacency();

    // 3. Run the chosen algorithm on the sub-mesh only. `preserve_manual` is
    //    false because the sub-mesh is a single region (label 0).
    let _ = run_segmentation(&mut sub, algorithm, false, progress);

    // 4. Count distinct sub-labels produced (labels are 0-based and contiguous).
    let mut max_sub = 0u32;
    for &l in &sub.segment_labels {
        if l > max_sub {
            max_sub = l;
        }
    }
    let k = max_sub + 1;
    if k <= 1 {
        return Err(
            "所选算法在该区域内未产生进一步划分（整块仍为一个区域）".to_string(),
        );
    }

    // 5. Allocate one fresh manual label per sub-region. Allocating through
    //    `alloc_manual_label` keeps the new labels unique and clear of the
    //    auto-segmentation namespace, so a later global auto-run never collides
    //    with them.
    let mut new_labels = Vec::with_capacity(k as usize);
    for _ in 0..k {
        new_labels.push(mesh.alloc_manual_label());
    }

    // Carry the old region's display name onto the first (often largest) piece.
    if let Some(name) = mesh.segment_names.remove(&label) {
        mesh.segment_names.insert(new_labels[0], name);
    }

    // 6. Map sub-labels back onto the original faces, recording a single undo op.
    let mut prev_labels: Vec<(u32, u32)> = Vec::with_capacity(region_faces.len());
    let mut writes: Vec<(u32, u32)> = Vec::with_capacity(region_faces.len());
    for (i, &f) in region_faces.iter().enumerate() {
        let target = new_labels[sub.segment_labels[i] as usize];
        if mesh.segment_labels[f as usize] != target {
            prev_labels.push((f, label));
            writes.push((f, target));
        }
    }
    mesh.history.record(OpKind::Split, None, &[], &prev_labels);
    for (f, t) in writes {
        mesh.segment_labels[f as usize] = t;
    }
    mesh.rebuild_segments();

    Ok(ResegmentResult {
        segments: mesh.sorted_segments(),
        segment_labels: mesh.segment_labels.clone(),
        moved_faces: prev_labels.len(),
        base_label: new_labels[0],
        region_count: k,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mesh::model::MeshModel;
    use crate::segment::SegmentationAlgorithm;

    fn noop(_: f32, _: &str) {}

    /// Two coplanar triangles (flat square) sharing one edge — a single region
    /// with no internal crease for `dihedral` to cut along.
    fn flat_square(label: u32) -> MeshModel {
        let mut m = MeshModel::new();
        m.vertices = vec![
            [0.0, 0.0, 0.0],
            [1.0, 0.0, 0.0],
            [0.0, 1.0, 0.0],
            [1.0, 1.0, 0.0],
        ];
        m.faces = vec![[0, 1, 2], [1, 3, 2]];
        m.segment_labels = vec![label, label];
        m.compute_normals();
        m.build_adjacency();
        m.rebuild_segments();
        m
    }

    #[test]
    fn a_missing_region_is_rejected() {
        let mut m = flat_square(5);
        let err = resegment_region(
            &mut m,
            999_999,
            &SegmentationAlgorithm::Dihedral { angle_threshold: 30.0 },
            &noop,
        )
        .expect_err("region must exist");
        assert!(err.contains("does not exist"), "error: {err}");
    }

    #[test]
    fn a_flat_region_cannot_be_subdivided() {
        let mut m = flat_square(5);
        let err = resegment_region(
            &mut m,
            5,
            &SegmentationAlgorithm::Dihedral { angle_threshold: 30.0 },
            &noop,
        )
        .expect_err("flat region has no cut, must error");
        assert!(
            err.contains("未产生进一步划分") || err.contains("no further"),
            "error: {err}"
        );
    }
}
