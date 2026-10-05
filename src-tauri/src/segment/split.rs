//! Split one existing region into several sub-regions.
//!
//! "Split" partitions a single segment into connected pieces. The Crease method
//! cuts along the region's internal creases using the *same* crease definition
//! the auto-segmentation pipeline uses (`postprocess::split_connected_components`
//! + `sdf::oriented_normals`), so a hand-split boundary lands exactly where the
//! user saw the auto-segmenter draw one — and crucially uses orientation-correct
//! normals, not the raw winding normals that would treat a flipped triangle as a
//! 180° crease on an unreliable-STL-winding mesh.
//!
//! A `Plane` variant exists for wire-format stability but is not yet wired to a
//! frontend gesture (backlog: it needs a viewport cut interaction that
//! constructs the plane in model-local space — see the adversarial review notes).

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::mesh::history::OpKind;
use crate::mesh::model::{MeshModel, Segment, MANUAL_SEGMENT_OFFSET};
use crate::segment::postprocess::split_connected_components;
use crate::segment::sdf::oriented_normals;

/// How a region is divided.
///
/// Wire format is internally tagged (`{"type":"crease","thresholdDeg":30}`),
/// mirroring [`crate::segment::SegmentationAlgorithm`] so the TypeScript side is
/// a plain discriminated union. A container-level `rename_all` only renames
/// variants; the per-variant attribute is what camelCases the fields, and
/// dropping it would silently ship `threshold_deg` to a frontend that sends
/// `thresholdDeg`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum SplitMethod {
    /// Not yet wired to a frontend gesture. Included for wire-format stability;
    /// returns an explicit "not implemented" error until a viewport cut
    /// interaction is built.
    #[serde(rename_all = "camelCase")]
    Plane { point: [f32; 3], normal: [f32; 3] },
    /// Cut the region along its internal creases. Adjacent region faces whose
    /// shared border scores `>= threshold_deg` on `crease_strength_deg` are
    /// disconnected; each connected piece becomes its own region.
    #[serde(rename_all = "camelCase")]
    Crease { threshold_deg: f32 },
}

/// What the frontend has to replace after a split.
///
/// No `face_colors`: a split moves labels and leaves the paint alone, so
/// shipping the whole colour buffer back would be several megabytes of JSON
/// describing a buffer that did not change. The command layer resends the full
/// buffer on `labels_changed` anyway.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SplitResult {
    pub segments: Vec<Segment>,
    pub segment_labels: Vec<u32>,
    pub moved_faces: usize,
    /// The label the largest (kept) piece ends up on. May differ from the input
    /// `label` when the input was an auto region: split promotes both halves to
    /// the manual namespace so the cut survives a later auto-segmentation re-run,
    /// otherwise the user gets a half-disappearing split.
    pub kept_label: u32,
    /// The first new manual label allocated for a split-off piece (others follow
    /// sequentially). Equals `kept_label` only when nothing was detached.
    pub new_label: u32,
}

/// Partition `label` into sub-regions and return what changed.
pub fn split_segment(
    mesh: &mut MeshModel,
    label: u32,
    method: &SplitMethod,
) -> Result<SplitResult, String> {
    // Plane is reserved for a future viewport gesture; see enum docs.
    let threshold_deg = match method {
        SplitMethod::Plane { .. } => {
            return Err(
                "plane-cut split is not implemented yet (needs a viewport cut gesture)".to_string(),
            )
        }
        SplitMethod::Crease { threshold_deg } => *threshold_deg,
    };

    if !mesh.segments.contains_key(&label) {
        return Err(format!("Segment {} does not exist", label));
    }
    // A non-positive threshold would make every edge a crease (crease_strength_deg
    // is always >= 0, so `>= 0` is trivially true), exploding the region into one
    // face per component. Require a strictly positive angle.
    if threshold_deg <= 0.0 {
        return Err("crease threshold must be greater than 0 degrees".to_string());
    }

    let region_faces: Vec<u32> = (0..mesh.segment_labels.len() as u32)
        .filter(|&i| mesh.segment_labels[i as usize] == label)
        .collect();
    if region_faces.is_empty() {
        return Err(format!("Segment {} has no faces", label));
    }

    // Orientation-correct normals first — `oriented_normals` clones `mesh.normals`,
    // so it must exist. Recompute defensively; it is cheap for a user gesture and
    // guarantees the clone below is the right length.
    mesh.compute_normals();
    let normals = oriented_normals(mesh);

    // Reuse the segmentation's own crease partitioner so split boundaries match
    // what the user already sees. It grows only within the target label and never
    // crosses a crease >= threshold, yielding one component id per sub-region.
    let components =
        split_connected_components(mesh, &mesh.segment_labels, Some((&normals, threshold_deg)));

    // Group region faces by component id.
    let mut comp_to_faces: HashMap<u32, Vec<u32>> = HashMap::new();
    for &f in &region_faces {
        comp_to_faces
            .entry(components[f as usize])
            .or_default()
            .push(f);
    }
    if comp_to_faces.len() <= 1 {
        return Err(format!(
            "no crease above {:.0}° splits this region",
            threshold_deg
        ));
    }

    // Keep the largest component on `label`; its faces keep their label unless the
    // input was an auto region, in which case we promote the kept half to a fresh
    // manual label too (both halves must survive a later auto-segmentation).
    let mut kept_comp = *comp_to_faces.keys().next().expect("non-empty map");
    let mut kept_size = 0usize;
    for (&c, faces) in &comp_to_faces {
        if faces.len() > kept_size || (faces.len() == kept_size && c < kept_comp) {
            kept_size = faces.len();
            kept_comp = c;
        }
    }

    let kept_is_auto = label < MANUAL_SEGMENT_OFFSET;
    let kept_label = if kept_is_auto {
        let nl = mesh.alloc_manual_label();
        // Migrate any custom name from the old (auto) label to the new manual one,
        // so a renamed auto region keeps its name after being split.
        if let Some(name) = mesh.segment_names.remove(&label) {
            mesh.segment_names.insert(nl, name);
        }
        nl
    } else {
        label
    };

    // Capture pre-operation labels (all region faces currently carry `label`),
    // then write the new labels after recording — mirror merge_segments' order.
    let mut prev_labels: Vec<(u32, u32)> = Vec::new();
    let mut writes: Vec<(u32, u32)> = Vec::new();
    let mut new_label_start: Option<u32> = None;

    for (&comp, faces) in &comp_to_faces {
        let target = if comp == kept_comp {
            kept_label
        } else {
            let nl = mesh.alloc_manual_label();
            if new_label_start.is_none() {
                new_label_start = Some(nl);
            }
            nl
        };
        for &f in faces {
            if mesh.segment_labels[f as usize] != target {
                prev_labels.push((f, label));
                writes.push((f, target));
            }
        }
    }

    mesh.history.record(OpKind::Split, None, &[], &prev_labels);
    for (f, t) in writes {
        mesh.segment_labels[f as usize] = t;
    }
    mesh.rebuild_segments();

    Ok(SplitResult {
        segments: mesh.sorted_segments(),
        segment_labels: mesh.segment_labels.clone(),
        moved_faces: prev_labels.len(),
        kept_label,
        new_label: new_label_start.unwrap_or(kept_label),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mesh::model::MeshModel;

    /// Two triangles meeting at a 90° inner corner — an L-bracket. A concave
    /// crease scores its full angle (no convex halving), so it is a crease the
    /// splitter will actually cut along.
    fn tent_mesh(label: u32) -> MeshModel {
        let mut m = MeshModel::new();
        m.vertices = vec![
            [0.0, 0.0, 0.0], // 0
            [1.0, 0.0, 0.0], // 1
            [0.0, 1.0, 0.0], // 2
            [0.0, 0.0, 1.0], // 3
        ];
        // Face 0: 0-1-2 (XY plane, normal +Z). Face 1: 0-1-3 (XZ plane, normal -Y).
        // They share edge 0-1 at a 90° concave corner — a real feature edge.
        m.faces = vec![[0, 1, 2], [0, 1, 3]];
        m.segment_labels = vec![label, label];
        m.build_adjacency();
        m.rebuild_segments();
        m
    }

    /// Two coplanar triangles (a flat square) — no internal crease at all.
    fn flat_mesh(label: u32) -> MeshModel {
        let mut m = MeshModel::new();
        m.vertices = vec![
            [0.0, 0.0, 0.0],
            [1.0, 0.0, 0.0],
            [0.0, 1.0, 0.0],
            [1.0, 1.0, 0.0],
        ];
        m.faces = vec![[0, 1, 2], [1, 3, 2]];
        m.segment_labels = vec![label, label];
        m.build_adjacency();
        m.rebuild_segments();
        m
    }

    #[test]
    fn a_crease_separates_a_region_into_two_pieces() {
        let mut m = tent_mesh(MANUAL_SEGMENT_OFFSET);
        let r = split_segment(&mut m, MANUAL_SEGMENT_OFFSET, &SplitMethod::Crease { threshold_deg: 30.0 })
            .expect("split should succeed");
        assert_eq!(r.moved_faces, 1, "exactly one of the two faces moves");
        // The two faces must now carry different labels.
        assert_ne!(m.segment_labels[0], m.segment_labels[1]);
        // The kept piece stays on the original manual label.
        assert!(m.segment_labels[0] == MANUAL_SEGMENT_OFFSET || m.segment_labels[1] == MANUAL_SEGMENT_OFFSET);
        // Exactly two regions remain.
        assert_eq!(m.segments.len(), 2);
    }

    #[test]
    fn splitting_an_auto_region_promotes_both_halves_to_manual() {
        // Label 5 is an auto segment (well below MANUAL_SEGMENT_OFFSET).
        let mut m = tent_mesh(5);
        let r = split_segment(&mut m, 5, &SplitMethod::Crease { threshold_deg: 30.0 })
            .expect("split should succeed");
        // The kept piece was promoted to a fresh manual label, not left as 5.
        assert!(r.kept_label >= MANUAL_SEGMENT_OFFSET, "kept label must be promoted to manual namespace");
        assert_ne!(r.kept_label, 5);
        // Neither face keeps the auto label.
        assert!(!m.segment_labels.contains(&5));
        // Both resulting labels are in the manual namespace.
        assert!(m.segment_labels.iter().all(|&l| l >= MANUAL_SEGMENT_OFFSET));
    }

    #[test]
    fn a_region_with_no_internal_crease_cannot_be_split() {
        let mut m = flat_mesh(MANUAL_SEGMENT_OFFSET);
        let err = split_segment(&mut m, MANUAL_SEGMENT_OFFSET, &SplitMethod::Crease { threshold_deg: 30.0 })
            .expect_err("flat region has no crease to split along");
        assert!(err.contains("no crease"), "error should explain the missing crease: {err}");
        // Region is untouched.
        assert_eq!(m.segment_labels, vec![MANUAL_SEGMENT_OFFSET, MANUAL_SEGMENT_OFFSET]);
    }

    #[test]
    fn a_non_positive_threshold_is_rejected() {
        let mut m = tent_mesh(MANUAL_SEGMENT_OFFSET);
        let err = split_segment(&mut m, MANUAL_SEGMENT_OFFSET, &SplitMethod::Crease { threshold_deg: 0.0 })
            .expect_err("threshold must be > 0");
        assert!(err.contains("greater than 0"), "error: {err}");
    }

    #[test]
    fn splitting_a_missing_region_is_rejected() {
        let mut m = tent_mesh(MANUAL_SEGMENT_OFFSET);
        let err = split_segment(&mut m, 999_999, &SplitMethod::Crease { threshold_deg: 30.0 })
            .expect_err("region must exist");
        assert!(err.contains("does not exist"), "error: {err}");
    }

    #[test]
    fn the_plane_method_is_not_yet_wired() {
        let mut m = tent_mesh(MANUAL_SEGMENT_OFFSET);
        let err = split_segment(
            &mut m,
            MANUAL_SEGMENT_OFFSET,
            &SplitMethod::Plane { point: [0.0, 0.0, 0.0], normal: [0.0, 1.0, 0.0] },
        )
        .expect_err("plane split is backlog");
        assert!(err.contains("not implemented"), "error: {err}");
    }

    #[test]
    fn a_split_can_be_undone() {
        let mut m = tent_mesh(MANUAL_SEGMENT_OFFSET);
        split_segment(&mut m, MANUAL_SEGMENT_OFFSET, &SplitMethod::Crease { threshold_deg: 30.0 })
            .expect("split should succeed");
        assert_ne!(m.segment_labels[0], m.segment_labels[1], "precondition: split happened");

        let mut colors: Vec<[u8; 4]> = vec![[0; 4]; m.segment_labels.len()];
        let mut labels = m.segment_labels.clone();
        m.history
            .undo(&mut colors, &mut labels, &mut m.vertices)
            .expect("undo should return an outcome");
        m.segment_labels = labels;
        m.rebuild_segments();

        // Both faces back on the original label, single region again.
        assert_eq!(m.segment_labels, vec![MANUAL_SEGMENT_OFFSET, MANUAL_SEGMENT_OFFSET]);
        assert_eq!(m.segments.len(), 1);
    }
}
