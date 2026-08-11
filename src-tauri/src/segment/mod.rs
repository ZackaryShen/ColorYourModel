pub mod concavity;
pub mod convex_decomp;
pub mod curvature;
pub mod dihedral;
pub mod flood_fill;
pub mod graphcut;
pub mod manual;
pub mod metrics;
pub mod postprocess;
pub mod sdf;
pub mod split;

use serde::{Deserialize, Serialize};

use crate::mesh::loader::ProgressFn;
use crate::mesh::model::{MeshModel, Segment, MANUAL_SEGMENT_OFFSET};
use crate::segment::convex_decomp::{
    segment_by_convex_decomposition, segment_by_curve_skeleton,
};
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
    /// SDF + GMM soft clustering + alpha-expansion graph cut (Shapira 2008 /
    /// CGAL Surface_mesh_segmentation standard second stage). The smoothness
    /// term uses the signed minima-rule prior: concave folds cost 1× their
    /// dihedral to cut, convex edges 0.1×. `k = 0` auto-estimates from the SDF
    /// histogram peaks. Experimental (iter 45), compared head-to-head against
    /// the legacy paths.
    #[serde(rename_all = "camelCase")]
    SdfGraphCut { k: u32 },
    /// Concavity-Aware Fields (Au et al. 2012 TVCG, simplified): a
    /// concavity-sensitive Laplacian (concave vertices weaken the edge weight
    /// so the scalar field barely resists crossing a concave seam) is solved
    /// once, then the field is thresholded into regions. `k = 0` auto-estimates
    /// from concave-seam density (articulated models get more regions).
    /// Experimental (iter 45).
    #[serde(rename_all = "camelCase")]
    Concavity { k: u32 },
    /// Approximate convex decomposition (V-HACD): voxelized hierarchical ACD that
    /// cuts the model at its narrow joints (neck / waist / wrist / ankle). Pure
    /// Rust via parry3d; robust where the concavity field went silent on armoured
    /// characters (convex ridges carry no concavity signal). Each region is one
    /// approximately-convex block. `max_hulls = 0` auto-picks 32.
    #[serde(rename_all = "camelCase")]
    ConvexDecomposition { max_hulls: u32, concavity: f32 },
    /// Curve-skeleton segmentation derived from the convex-decomposition adjacency
    /// graph: each chain between joints is merged into one limb-level region, so
    /// head / torso / limbs come back as whole parts instead of dozens of blocks.
    /// Coarser and more semantic than the raw block partition. Same V-HACD pass
    /// under the hood (`max_hulls = 0` auto-picks 32; `concavity` 0..1).
    #[serde(rename_all = "camelCase")]
    CurveSkeleton { max_hulls: u32, concavity: f32 },
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
        SegmentationAlgorithm::SdfGraphCut { k } => {
            graphcut::segment_by_sdf_graphcut(mesh, *k, on_progress)
        }
        SegmentationAlgorithm::Concavity { k } => {
            concavity::segment_by_concavity(mesh, *k, on_progress)
        }
        SegmentationAlgorithm::ConvexDecomposition {
            max_hulls,
            concavity,
        } => segment_by_convex_decomposition(mesh, *max_hulls, *concavity, on_progress),
        SegmentationAlgorithm::CurveSkeleton {
            max_hulls,
            concavity,
        } => segment_by_curve_skeleton(mesh, *max_hulls, *concavity, on_progress),
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
    use crate::segment::split::SplitMethod;

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
            (
                SegmentationAlgorithm::SdfGraphCut { k: 0 },
                r#"{"type":"sdfGraphCut","k":0}"#,
            ),
            (
                SegmentationAlgorithm::Concavity { k: 6 },
                r#"{"type":"concavity","k":6}"#,
            ),
            (
                SegmentationAlgorithm::ConvexDecomposition {
                    max_hulls: 0,
                    concavity: 0.05,
                },
                r#"{"type":"convexDecomposition","maxHulls":0,"concavity":0.05}"#,
            ),
            (
                SegmentationAlgorithm::CurveSkeleton {
                    max_hulls: 32,
                    concavity: 0.02,
                },
                r#"{"type":"curveSkeleton","maxHulls":32,"concavity":0.02}"#,
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

    /// The split method travels over the same untyped IPC boundary, so its exact
    /// JSON must be pinned too. The frontend sends `thresholdDeg` (camelCase);
    /// a missing per-variant `rename_all` would silently ship `threshold_deg`
    /// and fail at runtime with "missing field".
    #[test]
    fn split_method_wire_format_is_stable() {
        let cases: Vec<(SplitMethod, &str)> = vec![
            (
                SplitMethod::Crease { threshold_deg: 30.0 },
                r#"{"type":"crease","thresholdDeg":30.0}"#,
            ),
            (
                SplitMethod::Plane {
                    point: [0.0, 0.0, 0.0],
                    normal: [0.0, 1.0, 0.0],
                },
                r#"{"type":"plane","point":[0.0,0.0,0.0],"normal":[0.0,1.0,0.0]}"#,
            ),
        ];
        for (method, expected) in cases {
            let json = serde_json::to_string(&method).expect("serialize");
            assert_eq!(json, expected, "split wire format drifted for {:?}", method);
            let back: SplitMethod =
                serde_json::from_str(expected).expect("deserialize frontend payload");
            assert_eq!(format!("{:?}", back), format!("{:?}", method));
        }
    }
}

/// Real-STL head-to-head harness for the iter-45 segmentation algorithms.
/// Reads STL files from disk and reports region counts + wall-clock time for
/// the curvatureKMeans baseline vs sdfGraphCut vs concavity.
///
/// Run with:
///   cargo test --lib -- --ignored --nocapture stl_comparison_harness
#[cfg(test)]
mod stl_comparison_harness {
    use std::time::Instant;

    use super::{run_segmentation, SegmentationAlgorithm};
    use crate::mesh::loader::load_stl;

    struct Fixture {
        name: &'static str,
        path: &'static str,
    }

    const FIXTURES: &[Fixture] = &[
        Fixture {
            name: "Sphere",
            path: r"C:\Users\Administrator\Desktop\Sphere.stl",
        },
        Fixture {
            name: "Fire_Bambuslicer",
            path: r"C:\Users\Administrator\Desktop\Fire_Bambuslicer.stl",
        },
        Fixture {
            name: "dargon",
            path: r"C:\Users\Administrator\Desktop\dargon.stl",
        },
        Fixture {
            name: "KamenRider",
            path: r"C:\selfDIr\Blender3D\假面骑士ZZZ灾厄2.stl",
        },
        Fixture {
            name: "Sanji",
            path: r"C:\selfDIr\Blender3D\Sanji+Diorama+Detailed_U1.stl",
        },
    ];

    fn noop_progress() -> Box<crate::mesh::loader::ProgressFn> {
        Box::new(|_, _| {})
    }

    fn run_algo(
        mesh: &mut crate::mesh::model::MeshModel,
        algo: &SegmentationAlgorithm,
    ) -> (usize, f64) {
        let cb = noop_progress();
        let t = Instant::now();
        let segs = run_segmentation(mesh, algo, false, &*cb);
        let dt = t.elapsed().as_secs_f64();
        (segs.len(), dt)
    }

    fn load(path: &std::path::Path) -> Option<crate::mesh::model::MeshModel> {
        let cb = noop_progress();
        load_stl(path, &*cb).ok()
    }

    #[test]
    #[ignore = "reads large STL files from disk; run explicitly"]
    fn stl_comparison_harness() {
        for fx in FIXTURES {
            let path = std::path::Path::new(fx.path);
            if !path.exists() {
                eprintln!("[{}] MISSING {} — skipped", fx.name, fx.path);
                continue;
            }
            let mesh = match load(path) {
                Some(m) => m,
                None => {
                    eprintln!("[{}] LOAD FAILED — skipped", fx.name);
                    continue;
                }
            };
            let n = mesh.faces.len();
            eprintln!("=== {} ({} faces) ===", fx.name, n);

            let mut m = mesh;
            let (c_regions, c_secs) = run_algo(
                &mut m,
                &SegmentationAlgorithm::CurvatureKMeans {
                    k: 6,
                    smoothing_iters: 2,
                    use_sdf: true,
                    crease_threshold_deg: 45.0,
                },
            );
            eprintln!("  curvatureKMeans : {:4} regions in {:6.2}s", c_regions, c_secs);

            let mut m = match load(path) {
                Some(x) => x,
                None => continue,
            };
            let (g_regions, g_secs) =
                run_algo(&mut m, &SegmentationAlgorithm::SdfGraphCut { k: 0 });
            eprintln!("  sdfGraphCut      : {:4} regions in {:6.2}s", g_regions, g_secs);

            let mut m = match load(path) {
                Some(x) => x,
                None => continue,
            };
            let (a_regions, a_secs) = run_algo(&mut m, &SegmentationAlgorithm::Concavity { k: 0 });
            eprintln!("  concavity        : {:4} regions in {:6.2}s", a_regions, a_secs);

            let mut m = match load(path) {
                Some(x) => x,
                None => continue,
            };
            let (v_regions, v_secs) = run_algo(
                &mut m,
                &SegmentationAlgorithm::ConvexDecomposition {
                    max_hulls: 0,
                    concavity: 0.05,
                },
            );
            eprintln!("  convexDecomp     : {:4} regions in {:6.2}s", v_regions, v_secs);

            let mut m = match load(path) {
                Some(x) => x,
                None => continue,
            };
            let (s_regions, s_secs) = run_algo(
                &mut m,
                &SegmentationAlgorithm::CurveSkeleton {
                    max_hulls: 0,
                    concavity: 0.05,
                },
            );
            eprintln!("  curveSkeleton    : {:4} regions in {:6.2}s", s_regions, s_secs);
            eprintln!();
        }
    }
}
