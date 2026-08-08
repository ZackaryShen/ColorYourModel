pub mod curvature;
pub mod dihedral;
pub mod flood_fill;
pub mod manual;
pub mod metrics;
pub mod postprocess;
pub mod sdf;

use serde::{Deserialize, Serialize};

use crate::mesh::loader::ProgressFn;
use crate::mesh::model::{MeshModel, Segment};
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
pub fn run_segmentation(
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
