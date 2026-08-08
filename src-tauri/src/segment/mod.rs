pub mod curvature;
pub mod dihedral;
pub mod flood_fill;
pub mod manual;
pub mod metrics;
pub mod postprocess;
pub mod sdf;

use serde::{Deserialize, Serialize};

use crate::mesh::loader::ProgressFn;
use crate::mesh::model::{MeshModel, Segment, MANUAL_SEGMENT_OFFSET};
use crate::segment::dihedral::segment_by_dihedral_angle;
use crate::segment::sdf::segment_by_sdf;

/// Which intelligent-segmentation algorithm to run, with its tunable parameters.
///
/// One dispatch surface so the UI only needs a single algorithm selector plus
/// dynamic parameter sliders (REFUTE: avoid N near-identical commands and the
/// resulting configuration drift). Every variant writes per-face labels into
/// `mesh.segment_labels` and segment metadata into `mesh.segments`.
///
/// Wire format is **internally tagged** (`{"type":"dihedral","angleThreshold":30}`)
/// so the TypeScript side is a plain discriminated union. Note that a container
/// -level `rename_all` only renames *variants*; the per-variant attribute is what
/// camelCases the fields, and dropping it would silently ship `angle_threshold`
/// to a frontend that sends `angleThreshold`. `serde_wire_format_is_stable`
/// below pins the exact JSON so that mistake fails a test instead of a user.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum SegmentationAlgorithm {
    /// Flat/curved region growing by dihedral angle (existing baseline).
    #[serde(rename_all = "camelCase")]
    Dihedral { angle_threshold: f32 },
    /// Shape Diameter Function: thickness-based semantic parts (existing, fixed).
    /// `k = 0` auto-estimates the cluster count from SDF histogram peaks.
    #[serde(rename_all = "camelCase")]
    ShapeDiameter { k: u32 },
    /// Intrinsic feature clustering (new, REFUTE-revised): local curvature +
    /// thickness (|SDF|), deterministic k-means, then connectivity split,
    /// feature-distance merge, and finally a contour merge that dissolves any
    /// region border not sitting on a real crease. `crease_threshold_deg = 0`
    /// uses the built-in default.
    #[serde(rename_all = "camelCase")]
    CurvatureKMeans {
        k: u32,
        smoothing_iters: u32,
        use_sdf: bool,
        crease_threshold_deg: f32,
    },
}

/// Run the selected algorithm and return segment metadata. Per-face labels are
/// written into `mesh.segment_labels`; `mesh.segments` is rebuilt by the backend.
///
/// # `preserve_manual`
///
/// Every algorithm assigns a label to *every* face, so re-running one wipes the
/// lasso regions and segment-brush strokes the user drew by hand — and the
/// commands clear the undo history immediately afterwards, because the recorded
/// diffs were taken against labels that no longer exist. The manual work is
/// therefore unrecoverable. (Face colours are untouched: nothing in `export`
/// reads `segment_labels`, so what is lost is the partition, not the paint.)
///
/// The obvious fix — warn before running — was rejected: the segmentation panel
/// is not the only caller. Importing a model auto-segments it immediately, so a
/// dialog wired into the panel guards one path, misses the other, and leaves
/// behind the impression that the case is handled. Restoring the manual labels
/// after the algorithm has run protects both callers without asking anything of
/// either.
///
/// Auto labels are compacted to 0..K, and overwriting some of them can leave a
/// hole in that range or empty a region entirely. That is safe: the frontend
/// tests segment membership against the returned metadata (`segmentIds` in
/// Viewport) and colours by `label % palette_len`, neither of which assumes the
/// range is contiguous.
pub fn run_segmentation(
    mesh: &mut MeshModel,
    algo: &SegmentationAlgorithm,
    preserve_manual: bool,
    on_progress: &ProgressFn,
) -> Vec<Segment> {
    let manual: Vec<(usize, u32)> = if preserve_manual {
        mesh.segment_labels
            .iter()
            .enumerate()
            .filter(|(_, &l)| l >= MANUAL_SEGMENT_OFFSET)
            .map(|(i, &l)| (i, l))
            .collect()
    } else {
        Vec::new()
    };

    // Region names are keyed by label, and a re-run renumbers the auto range
    // from zero — the new region 3 has nothing to do with the old one, so a
    // name left behind would reattach itself to an unrelated part of the model.
    // Manual labels are never recycled and (when preserved) come back attached
    // to the same faces, so their names survive. Purging *before* dispatch lets
    // the rebuild inside the algorithm see the cleaned table; doing it after
    // would leave the metadata it just produced carrying the stale names.
    if preserve_manual {
        mesh.segment_names
            .retain(|&label, _| label >= MANUAL_SEGMENT_OFFSET);
    } else {
        mesh.segment_names.clear();
    }

    let segments = dispatch(mesh, algo, on_progress);

    if manual.is_empty() {
        return segments;
    }
    for (i, label) in manual {
        mesh.segment_labels[i] = label;
    }
    // Face counts moved and regions may have disappeared, so the metadata the
    // algorithm just produced is stale.
    mesh.rebuild_segments();
    mesh.sorted_segments()
}

fn dispatch(
    mesh: &mut MeshModel,
    algo: &SegmentationAlgorithm,
    on_progress: &ProgressFn,
) -> Vec<Segment> {
    match algo {
        SegmentationAlgorithm::Dihedral { angle_threshold } => {
            segment_by_dihedral_angle(mesh, *angle_threshold, on_progress)
        }
        SegmentationAlgorithm::ShapeDiameter { k } => segment_by_sdf(mesh, *k, on_progress),
        SegmentationAlgorithm::CurvatureKMeans {
            k,
            smoothing_iters,
            use_sdf,
            crease_threshold_deg,
        } => curvature::segment_by_curvature_kmeans(
            mesh,
            *k,
            *smoothing_iters,
            *use_sdf,
            *crease_threshold_deg,
            on_progress,
        ),
    }
}

#[cfg(test)]
mod preserve_manual_tests {
    use super::*;
    use crate::segment::metrics::unit_cube;

    fn noop_progress() -> Box<ProgressFn> {
        Box::new(|_, _| {})
    }

    /// A cube with two faces claimed by a hand-drawn region.
    fn cube_with_manual_region() -> (MeshModel, u32) {
        let mut mesh = unit_cube();
        let label = mesh.alloc_manual_label();
        mesh.segment_labels[0] = label;
        mesh.segment_labels[1] = label;
        (mesh, label)
    }

    #[test]
    fn rerunning_keeps_hand_drawn_regions() {
        let (mut mesh, label) = cube_with_manual_region();
        let cb = noop_progress();

        let segments = run_segmentation(
            &mut mesh,
            &SegmentationAlgorithm::Dihedral {
                angle_threshold: 30.0,
            },
            true,
            &*cb,
        );

        assert_eq!(mesh.segment_labels[0], label);
        assert_eq!(mesh.segment_labels[1], label);
        // Metadata must describe the restored labels, not the ones the
        // algorithm produced before they were overwritten.
        let manual = segments
            .iter()
            .find(|s| s.id == label)
            .expect("manual region missing from returned metadata");
        assert_eq!(manual.face_count, 2);
        assert_eq!(manual.color, Some(MeshModel::manual_label_color(label)));
        let total: u32 = segments.iter().map(|s| s.face_count).sum();
        assert_eq!(total, mesh.faces.len() as u32, "face counts must partition");
    }

    #[test]
    fn opting_out_lets_the_algorithm_claim_every_face() {
        let (mut mesh, label) = cube_with_manual_region();
        let cb = noop_progress();

        let segments = run_segmentation(
            &mut mesh,
            &SegmentationAlgorithm::Dihedral {
                angle_threshold: 30.0,
            },
            false,
            &*cb,
        );

        assert!(
            !mesh.segment_labels.contains(&label),
            "preserve_manual=false must not leave manual labels behind"
        );
        assert!(segments.iter().all(|s| s.id < MANUAL_SEGMENT_OFFSET));
    }

    /// The restore path must be inert when there is nothing to restore — the
    /// common case, and the one where an accidental extra `rebuild_segments`
    /// would be pure overhead on every import.
    #[test]
    fn a_mesh_without_manual_regions_is_unaffected_by_the_flag() {
        let cb = noop_progress();
        let algo = SegmentationAlgorithm::Dihedral {
            angle_threshold: 30.0,
        };

        let mut kept = unit_cube();
        let with = run_segmentation(&mut kept, &algo, true, &*cb);
        let mut wiped = unit_cube();
        let without = run_segmentation(&mut wiped, &algo, false, &*cb);

        assert_eq!(kept.segment_labels, wiped.segment_labels);
        assert_eq!(with.len(), without.len());
    }

    /// Names are keyed by label. A re-run renumbers the auto range from zero,
    /// so "Left arm" on label 3 would reappear on whatever the algorithm calls
    /// 3 next time — a different part of the model entirely. Manual labels are
    /// never recycled and come back attached to the same faces, so their names
    /// have to survive the same call that drops the auto ones.
    #[test]
    fn a_rerun_forgets_auto_names_and_keeps_manual_ones() {
        let (mut mesh, label) = cube_with_manual_region();
        mesh.rebuild_segments();
        mesh.rename_segment(label, "Handle").unwrap();
        let auto_label = *mesh
            .segment_labels
            .iter()
            .find(|&&l| l < MANUAL_SEGMENT_OFFSET)
            .expect("cube has auto faces");
        mesh.rename_segment(auto_label, "Left arm").unwrap();

        let cb = noop_progress();
        let segments = run_segmentation(
            &mut mesh,
            &SegmentationAlgorithm::Dihedral {
                angle_threshold: 30.0,
            },
            true,
            &*cb,
        );

        assert_eq!(mesh.segment_names.get(&label).map(String::as_str), Some("Handle"));
        assert!(
            !mesh.segment_names.contains_key(&auto_label),
            "an auto name outlived the renumbering that invalidated it"
        );
        assert!(
            segments
                .iter()
                .all(|s| s.id >= MANUAL_SEGMENT_OFFSET || s.name.starts_with("Region ")),
            "auto regions must come back with default names"
        );
    }

    /// Wiping the manual regions has to wipe their names too, or the next
    /// hand-drawn region inherits a name it never had.
    #[test]
    fn opting_out_also_drops_manual_names() {
        let (mut mesh, label) = cube_with_manual_region();
        mesh.rebuild_segments();
        mesh.rename_segment(label, "Handle").unwrap();

        let cb = noop_progress();
        run_segmentation(
            &mut mesh,
            &SegmentationAlgorithm::Dihedral {
                angle_threshold: 30.0,
            },
            false,
            &*cb,
        );

        assert!(mesh.segment_names.is_empty());
    }
}

#[cfg(test)]
mod wire_format_tests {
    use super::*;

    /// The IPC boundary is untyped: Tauri hands serde whatever JSON the webview
    /// produced. A rename on either side is invisible at compile time and shows
    /// up as a runtime "missing field" that looks like a UI bug, so the exact
    /// wire shape is pinned here.
    #[test]
    fn serde_wire_format_is_stable() {
        let cases: Vec<(SegmentationAlgorithm, &str)> = vec![
            (
                SegmentationAlgorithm::Dihedral {
                    angle_threshold: 30.0,
                },
                r#"{"type":"dihedral","angleThreshold":30.0}"#,
            ),
            (
                SegmentationAlgorithm::ShapeDiameter { k: 0 },
                r#"{"type":"shapeDiameter","k":0}"#,
            ),
            (
                SegmentationAlgorithm::CurvatureKMeans {
                    k: 6,
                    smoothing_iters: 2,
                    use_sdf: true,
                    crease_threshold_deg: 20.0,
                },
                r#"{"type":"curvatureKMeans","k":6,"smoothingIters":2,"useSdf":true,"creaseThresholdDeg":20.0}"#,
            ),
        ];
        for (algo, expected) in cases {
            let json = serde_json::to_string(&algo).expect("serialize");
            assert_eq!(json, expected, "wire format drifted for {:?}", algo);
            // And the frontend's exact payload must deserialize back.
            let back: SegmentationAlgorithm =
                serde_json::from_str(expected).expect("deserialize frontend payload");
            assert_eq!(format!("{:?}", back), format!("{:?}", algo));
        }
    }
}
