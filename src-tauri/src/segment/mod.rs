pub mod concavity;
pub mod convex_decomp;
pub mod curvature;
pub mod dihedral;
pub mod fh;
pub mod flood_fill;
pub mod graphcut;
pub mod manual;
pub mod metrics;
pub mod postprocess;
pub mod sdf;
pub mod seeded;
pub mod split;
pub mod resegment;
pub mod recommend;
pub mod planar;
pub mod eye;
pub mod multiview;
pub mod cross_section;
pub mod face_frame;
pub mod fuse;
pub mod template;

use serde::{Deserialize, Serialize};

use crate::mesh::loader::ProgressFn;
use crate::mesh::model::{MeshModel, Segment, MANUAL_SEGMENT_OFFSET};
use crate::segment::convex_decomp::{
    segment_by_convex_decomposition, segment_by_curve_skeleton,
};
use crate::segment::dihedral::segment_by_dihedral_angle;
use crate::segment::fh::segment_by_fh;
use crate::segment::recommend::RecommendWeights;
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
    /// Felzenszwalb-Huttenlocher graph segmentation (iter 56), ported from the
    /// final stage of SAM3D (Pointcept, arxiv 2306.03908). Unlike every other
    /// variant here it takes a *scale* (granularity) instead of a preset region
    /// count k: edges merge while their weight is below the adaptive MST
    /// threshold `Int(C) + scale/|C|`, so the part count emerges from the
    /// geometry. Edge weight = |sig_u − sig_v|, the significance field from
    /// `recommend` (curvature ⊕ concavity), so the same UI weights feed the seed
    /// suggestion and the partition. Pure-geometric: no ML, no preset k.
    #[serde(rename_all = "camelCase")]
    FhGraph {
        scale: f32,
        curvature: f32,
        concavity: f32,
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
        SegmentationAlgorithm::FhGraph {
            scale,
            curvature,
            concavity,
        } => segment_by_fh(
            mesh,
            *scale,
            RecommendWeights {
                curvature: *curvature,
                concavity: *concavity,
            },
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
            (
                SegmentationAlgorithm::FhGraph {
                    scale: 0.3,
                    curvature: 1.0,
                    concavity: 1.0,
                },
                r#"{"type":"fhGraph","scale":0.3,"curvature":1.0,"concavity":1.0}"#,
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

            let mut m = match load(path) {
                Some(x) => x,
                None => continue,
            };
            let (h_regions, h_secs) = run_algo(
                &mut m,
                &SegmentationAlgorithm::FhGraph {
                    scale: 0.3,
                    curvature: 1.0,
                    concavity: 1.0,
                },
            );
            eprintln!("  fhGraph          : {:4} regions in {:6.2}s", h_regions, h_secs);
            eprintln!();
        }
    }
}

/// Godzilla fuse-path diagnosis. The fold-angle backbone under-segments smooth
/// sculpts (the whole body floods into one region at 5°), so this probe measures
/// WHY: the raw edge-angle histogram, the dihedral(5°) partition size
/// distribution, and a reproduction of the exact fuse vote the SeedPanel sends
/// (`cut_threshold=2, min_region_faces=0`). Orthographic label renders (PPM,
/// front + side) are written next to the log so the partitions can be judged
/// visually, not just by counts.
///
/// Run:
///   cargo test --release --lib -- --ignored --nocapture godzilla_fuse_diagnosis
#[cfg(test)]
mod godzilla_diagnosis {
    use std::collections::HashMap;
    use std::time::Instant;

    use petgraph::visit::EdgeRef;

    use crate::mesh::loader::{load_stl, ProgressFn};
    use crate::mesh::model::MeshModel;
    use crate::segment::dihedral::segment_by_dihedral_angle;
    use crate::segment::fuse::fuse_region_sets;
    use crate::segment::planar::{detect_planar_regions, PlanarParams};
    use crate::segment::multiview::{detect_multiview_regions, MultiViewParams};

    pub(crate) const GODZILLA: &str = r"C:\selfDIr\3D_3mf\stls\哥斯拉_U1.stl";
    const OUT_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), r"\target\diag_godzilla");

    pub(crate) fn noop_progress() -> Box<ProgressFn> {
        Box::new(|_, _| {})
    }

    fn load() -> Option<MeshModel> {
        let cb = noop_progress();
        let t = Instant::now();
        let m = load_stl(std::path::Path::new(GODZILLA), &*cb).ok();
        eprintln!("load: {:?} in {:.2}s", GODZILLA, t.elapsed().as_secs_f64());
        m
    }

    /// Per-region face counts, descending.
    pub(crate) fn region_sizes(mesh: &MeshModel) -> Vec<usize> {
        let mut c: HashMap<u32, usize> = HashMap::new();
        for &l in &mesh.segment_labels {
            *c.entry(l).or_insert(0) += 1;
        }
        let mut v: Vec<usize> = c.into_values().collect();
        v.sort_unstable_by(|a, b| b.cmp(a));
        v
    }

    fn report_sizes(name: &str, sizes: &[usize], n_faces: usize) {
        let n_reg = sizes.len();
        let top: Vec<usize> = sizes.iter().take(12).copied().collect();
        let pct = |p: f64| -> usize {
            let i = ((n_reg as f64 - 1.0) * p).round() as usize;
            sizes.get(i).copied().unwrap_or(0)
        };
        eprintln!(
            "[{}] regions={} top={} p50={} p90={} largest_share={:.1}%",
            name,
            n_reg,
            format!("{:?}", top),
            pct(0.5),
            pct(0.9),
            sizes.first().map(|s| *s as f64 / n_faces as f64 * 100.0).unwrap_or(0.0),
        );
    }

    /// Flat-shade an orthographic label render. view: 0 = front (XY), 1 = side
    /// (ZY). Faces are sub-pixel at this resolution, so a plain bbox scanline
    /// rasteriser with an atomic depth buffer is enough (and honest — no
    /// smoothing that could hide a real boundary). `palette` maps a 12-bit
    /// label to RGB.
    fn render(mesh: &MeshModel, labels: &[u32], view: u8, path: &str, w: usize, h: usize, palette: fn(u32) -> [u8; 3]) {
        use rayon::prelude::*;
        let (ux, uy, ud) = match view {
            0 => (0usize, 1usize, 2usize),
            _ => (2usize, 1usize, 0usize),
        };
        let mut lo = [f32::MAX; 2];
        let mut hi = [f32::MIN; 2];
        let mut dlo = f32::MAX;
        let mut dhi = f32::MIN;
        for v in &mesh.vertices {
            let p = [v[ux], v[uy]];
            for k in 0..2 {
                lo[k] = lo[k].min(p[k]);
                hi[k] = hi[k].max(p[k]);
            }
            dlo = dlo.min(v[ud]);
            dhi = dhi.max(v[ud]);
        }
        let span = [(hi[0] - lo[0]).max(1e-6), (hi[1] - lo[1]).max(1e-6)];
        let scale = (w as f32 / span[0]).min(h as f32 / span[1]) * 0.96;
        let ox = w as f32 / 2.0 - (lo[0] + hi[0]) / 2.0 * scale;
        let oy = h as f32 / 2.0 - (lo[1] + hi[1]) / 2.0 * scale;
        let dspan = (dhi - dlo).max(1e-6);

        let pix: Vec<std::sync::atomic::AtomicU32> =
            (0..w * h).map(|_| std::sync::atomic::AtomicU32::new(0)).collect();
        mesh.faces.par_iter().enumerate().for_each(|(fi, f)| {
            let label = labels[fi];
            let pts: [[f32; 3]; 3] = [
                mesh.vertices[f[0] as usize],
                mesh.vertices[f[1] as usize],
                mesh.vertices[f[2] as usize],
            ];
            let px: Vec<[f32; 2]> = pts
                .iter()
                .map(|p| [p[ux] * scale + ox, (h as f32) - (p[uy] * scale + oy)])
                .collect();
            let dz: Vec<f32> = pts.iter().map(|p| (p[ud] - dlo) / dspan).collect();
            let minx = px.iter().fold(f32::MAX, |m, p| m.min(p[0])).max(0.0) as isize;
            let maxx = px.iter().fold(f32::MIN, |m, p| m.max(p[0])).min(w as f32 - 1.0) as isize;
            let miny = px.iter().fold(f32::MAX, |m, p| m.min(p[1])).max(0.0) as isize;
            let maxy = px.iter().fold(f32::MIN, |m, p| m.max(p[1])).min(h as f32 - 1.0) as isize;
            if minx > maxx || miny > maxy {
                return;
            }
            let (ax, ay) = (px[0][0], px[0][1]);
            let (bx, by) = (px[1][0], px[1][1]);
            let (cx, cy) = (px[2][0], px[2][1]);
            let det = (bx - ax) * (cy - ay) - (cx - ax) * (by - ay);
            if det.abs() < 1e-12 {
                return;
            }
            for y in miny..=maxy {
                for x in minx..=maxx {
                    let (xf, yf) = (x as f32 + 0.5, y as f32 + 0.5);
                    let l0 = ((bx - ax) * (yf - ay) - (xf - ax) * (by - ay)) / det;
                    let l1 = ((xf - ax) * (cy - ay) - (cx - ax) * (yf - ay)) / det;
                    let l2 = 1.0 - l0 - l1;
                    if l0 < 0.0 || l1 < 0.0 || l2 < 0.0 {
                        continue;
                    }
                    let d = l0 * dz[0] + l1 * dz[1] + l2 * dz[2];
                    let dkey = (d * 1.0e6) as u32;
                    let idx = y as usize * w + x as usize;
                    let cell = &pix[idx];
                    let mut cur = cell.load(std::sync::atomic::Ordering::Relaxed);
                    while dkey > cur {
                        match cell.compare_exchange_weak(
                            cur,
                            dkey | ((label as u32 & 0xFFF) << 20),
                            std::sync::atomic::Ordering::Relaxed,
                            std::sync::atomic::Ordering::Relaxed,
                        ) {
                            Ok(_) => break,
                            Err(c) => cur = c,
                        }
                    }
                }
            }
        });

        // Golden-ratio hue per label → stable, distinguishable colours.
        let mut img = vec![0u8; w * h * 3];
        for (i, cell) in pix.iter().enumerate() {
            let raw = cell.load(std::sync::atomic::Ordering::Relaxed);
            let c = if raw == 0 {
                [28, 28, 32]
            } else {
                let label = (raw >> 20) & 0xFFF;
                palette(label)
            };
            img[i * 3..i * 3 + 3].copy_from_slice(&c);
        }
        let header = format!("P6\n{} {}\n255\n", w, h);
        let _ = std::fs::write(path, header.into_bytes().iter().copied().chain(img).collect::<Vec<u8>>());
        eprintln!("wrote {}", path);
    }

    /// Golden-ratio HSV palette for region ids (diagnosis renders only).
    fn palette_region(label: u32) -> [u8; 3] {
        let hue = (label as f32 * 0.6180339887) % 1.0;
        let (s, v) = (0.75f32, 0.95f32);
        let i = (hue * 6.0).floor() as i32 % 6;
        let f = hue * 6.0 - i as f32;
        let (r, g, b) = match i.rem_euclid(6) {
            0 => (v, v * (1.0 - s * f), v * (1.0 - s)),
            1 => (v * (1.0 - s * f), v, v * (1.0 - s)),
            2 => (v * (1.0 - s), v, v * (1.0 - s * f)),
            3 => (v * (1.0 - s), v * (1.0 - s * f), v),
            4 => (v * (1.0 - s * f), v * (1.0 - s), v),
            _ => (v, v * (1.0 - s), v * (1.0 - s * f)),
        };
        [(r * 255.0) as u8, (g * 255.0) as u8, (b * 255.0) as u8]
    }

    /// Grayscale heat palette: label value 0..240 → brightness 0..255.
    fn palette_gray(v: u32) -> [u8; 3] {
        let b = (v as f32 / 240.0 * 255.0).min(255.0) as u8;
        [b, b, b]
    }

    /// Partition → PPM renders (front + side) in OUT_DIR.
    pub(crate) fn render_partition(mesh: &MeshModel, tag: &str) {
        let _ = std::fs::create_dir_all(OUT_DIR);
        let labels = mesh.segment_labels.clone();
        render(mesh, &labels, 0, &format!("{}\\{}_front.ppm", OUT_DIR, tag), 900, 900, palette_region);
        render(mesh, &labels, 1, &format!("{}\\{}_side.ppm", OUT_DIR, tag), 900, 900, palette_region);
    }

    /// REFUTE blockers B1/B2 decisive probe.
    ///
    /// B1: does a sub-threshold (<cut°) leakage path keep head+torso in ONE
    /// union-find component at the aggressive cut thresholds the soft-fold plan
    /// needs? Measured directly: union-find at cut ∈ {1.5, 2, 3} + component-
    /// colour renders (head/chest same colour = leakage = plan dead).
    ///
    /// B2: is the boundary-strip normal angle θ separable between real part
    /// valleys (neck) and skin texture? For every cut edge, θ = angle between
    /// the mean normals of the ≤s-hop balls on each side (degenerate resultant →
    /// θ=0, merge-friendly). Output: θ histogram over cut edges + a grayscale
    /// heatmap render (bright = high θ) to be judged visually.
    #[test]
    #[ignore = "reads the 75MB Godzilla STL from disk; run explicitly"]
    fn godzilla_soft_fold_feasibility() {
        let mut mesh = match load() {
            Some(m) => m,
            None => {
                eprintln!("GODZILLA STL MISSING — skipped");
                return;
            }
        };
        let n = mesh.faces.len();
        let _ = std::fs::create_dir_all(OUT_DIR);

        // Raw fold angle per adjacency edge (face ids via the weight map, the
        // only invariant-based form per the refuter's special-check (c)).
        let edges: Vec<(u32, u32, f32)> = mesh
            .face_adjacency
            .edge_references()
            .map(|e| {
                let a = mesh.face_adjacency[e.source()];
                let b = mesh.face_adjacency[e.target()];
                let na = &mesh.normals[a as usize];
                let nb = &mesh.normals[b as usize];
                let d = (na[0] * nb[0] + na[1] * nb[1] + na[2] * nb[2]).clamp(-1.0, 1.0);
                (a, b, d.acos())
            })
            .collect();
        eprintln!("unique adjacency edges: {}", edges.len());

        // ── B1: union-find components at three cut thresholds ───────────
        for cut_deg in [1.5f32, 2.0, 3.0] {
            let cut_rad = cut_deg.to_radians();
            let mut parent: Vec<u32> = (0..n as u32).collect();
            fn find(p: &mut Vec<u32>, x: u32) -> u32 {
                if p[x as usize] != x {
                    p[x as usize] = find(p, p[x as usize]);
                }
                p[x as usize]
            }
            for &(a, b, ang) in &edges {
                if ang < cut_rad {
                    let ra = find(&mut parent, a);
                    let rb = find(&mut parent, b);
                    if ra != rb {
                        parent[ra as usize] = rb;
                    }
                }
            }
            let mut counts: HashMap<u32, usize> = HashMap::new();
            let mut labels = vec![0u32; n];
            let mut comp_map: HashMap<u32, u32> = HashMap::new();
            let mut next_cid = 0u32;
            for i in 0..n as u32 {
                let root = find(&mut parent, i);
                let cid = *comp_map.entry(root).or_insert_with(|| {
                    next_cid += 1;
                    next_cid - 1
                });
                labels[i as usize] = cid;
                *counts.entry(cid).or_insert(0) += 1;
            }
            let mut sizes: Vec<usize> = counts.values().copied().collect();
            sizes.sort_unstable_by(|a, b| b.cmp(a));
            report_sizes(&format!("cut{:.1}", cut_deg), &sizes, n);
            render(
                &mesh,
                &labels,
                0,
                &format!("{}\\connect_{:.1}_front.ppm", OUT_DIR, cut_deg),
                900,
                900,
                palette_region,
            );
        }

        // ── B2: strip-θ statistic over cut edges at cut=2°, s=3 ─────────
        let cut_rad = 2.0f32.to_radians();
        let s_hops = 3usize;
        // Component membership at this cut (keep-edges only).
        let mut parent: Vec<u32> = (0..n as u32).collect();
        fn find2(p: &mut Vec<u32>, x: u32) -> u32 {
            if p[x as usize] != x {
                p[x as usize] = find2(p, p[x as usize]);
            }
            p[x as usize]
        }
        for &(a, b, ang) in &edges {
            if ang < cut_rad {
                let ra = find2(&mut parent, a);
                let rb = find2(&mut parent, b);
                if ra != rb {
                    parent[ra as usize] = rb;
                }
            }
        }
        let mut comp = vec![0u32; n];
        let mut cmap: HashMap<u32, u32> = HashMap::new();
        let mut next_cid = 0u32;
        for i in 0..n as u32 {
            comp[i as usize] = *cmap.entry(find2(&mut parent, i)).or_insert_with(|| {
                next_cid += 1;
                next_cid - 1
            });
        }
        // Boundary faces per component + cut edges list.
        let mut is_boundary = vec![false; n];
        let mut cut_edges: Vec<(u32, u32)> = Vec::new();
        for &(a, b, ang) in &edges {
            if ang >= cut_rad {
                cut_edges.push((a, b));
                is_boundary[a as usize] = true;
                is_boundary[b as usize] = true;
            }
        }
        // BFS depth from boundary faces, ≤ s_hops, within component.
        let mut depth = vec![u8::MAX; n];
        let mut frontier: Vec<u32> = Vec::new();
        for f in 0..n as u32 {
            if is_boundary[f as usize] {
                depth[f as usize] = 0;
                frontier.push(f);
            }
        }
        let mut ring = Vec::new();
        for _ in 0..s_hops {
            ring.clear();
            for &f in &frontier {
                for e in mesh.face_adjacency.edges(petgraph::graph::NodeIndex::new(f as usize)) {
                    let g = mesh.face_adjacency[e.target()] as usize;
                    if depth[g] == u8::MAX && comp[g] == comp[f as usize] {
                        depth[g] = depth[f as usize] + 1;
                        ring.push(g as u32);
                    }
                }
            }
            std::mem::swap(&mut frontier, &mut ring);
        }
        // Per-component mean normal of its ≤s shell (single global shell is
        // wrong — the statistic must be per (component, boundary-neighbourhood);
        // approximated per cut edge by the two endpoint components' shell means.
        // For a fairer LOCAL reading, average only faces whose depth ≤ s AND
        // that lie within the same component as the endpoint (already implied).
        let ncomp = cmap.len();
        let mut shell_sum = vec![[0.0f32; 4]; ncomp]; // xyz + count in w
        for f in 0..n {
            let d = depth[f];
            if d <= s_hops as u8 {
                let c = comp[f] as usize;
                let nn = &mesh.normals[f];
                shell_sum[c][0] += nn[0];
                shell_sum[c][1] += nn[1];
                shell_sum[c][2] += nn[2];
                shell_sum[c][3] += 1.0;
            }
        }
        // θ per cut edge from the two endpoint shells.
        let mut theta_hist = vec![0u64; 19]; // 0,2.5,...,45+ in 2.5° buckets up to 45
        let mut face_theta = vec![0.0f32; n];
        for &(a, b) in &cut_edges {
            let sa = &shell_sum[comp[a as usize] as usize];
            let sb = &shell_sum[comp[b as usize] as usize];
            if sa[3] < 1.0 || sb[3] < 1.0 {
                continue;
            }
            let norm = |v: &[f32; 4]| -> [f32; 3] {
                let l = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
                if l < 1e-3 {
                    [0.0; 3]
                } else {
                    [v[0] / l, v[1] / l, v[2] / l]
                }
            };
            let va = norm(sa);
            let vb = norm(sb);
            let th = if va == [0.0; 3] || vb == [0.0; 3] {
                0.0f32 // degenerate (curved-shell) resultant → merge-friendly
            } else {
                let d = (va[0] * vb[0] + va[1] * vb[1] + va[2] * vb[2]).clamp(-1.0, 1.0);
                d.acos() * 180.0 / std::f32::consts::PI
            };
            let bi = ((th / 2.5) as usize).min(18);
            theta_hist[bi] += 1;
            if th > face_theta[a as usize] {
                face_theta[a as usize] = th;
            }
            if th > face_theta[b as usize] {
                face_theta[b as usize] = th;
            }
        }
        let total_th: u64 = theta_hist.iter().sum();
        for (i, &c) in theta_hist.iter().enumerate() {
            eprintln!(
                "  theta [{:4.1},{:4.1}) : {:9} ({:5.2}%)",
                i as f32 * 2.5,
                (i + 1) as f32 * 2.5,
                c,
                c as f64 / total_th as f64 * 100.0
            );
        }
        // Heatmap: grayscale = θ (bright = high), non-shell faces dark.
        let mut heat = vec![0u32; n];
        for f in 0..n {
            heat[f] = (face_theta[f] / 45.0 * 240.0).min(240.0) as u32;
        }
        render(
            &mesh,
            &heat,
            0,
            &format!("{}\\theta_heat_front.ppm", OUT_DIR),
            900,
            900,
            palette_gray,
        );

        // ── Pivot candidate (refuter M4): the EXISTING dihedral pipeline at
        // sub-5° thresholds. Phase1 cuts (B1 says the neck separates at 2°),
        // Phase3 avg-normal merge heals texture rubble, Phase4 absorbs crumbs.
        // Zero new algorithm — only the UI slider floor (5°) blocks it today.
        for deg in [2.0f32, 2.5, 3.0] {
            let t = Instant::now();
            let mut m = load().unwrap();
            let cb = noop_progress();
            segment_by_dihedral_angle(&mut m, deg, &*cb);
            let sizes = region_sizes(&m);
            eprintln!("--- dihedral({:.1}°) full pipeline in {:.2}s", deg, t.elapsed().as_secs_f64());
            report_sizes(&format!("dihedral{:.1}", deg), &sizes, n);
            render_partition(&m, &format!("dihedral_full{:.1}", deg));
        }
    }

    /// E2E: the REAL fuse command path (`planar + multiview + dihedral(deg) →
    /// fuse_region_sets(2, 0, …)` — the exact payload SeedPanel sends) at the
    /// proposed 2° slider floor vs the current 5° floor, across one sculpt
    /// (Godzilla), two organic sculpts (pug, dragon-whisker) and one
    /// hard-surface regression (kfc_station). Renders for Godzilla only.
    #[test]
    #[ignore = "reads large STL files from disk; run explicitly"]
    fn fuse_floor_e2e() {
        let fixtures: &[(&str, &str, bool)] = &[
            ("godzilla", r"C:\selfDIr\3D_3mf\stls\哥斯拉_U1.stl", true),
            ("pug", r"C:\selfDIr\Projects\ColorYourModel\samples\_src\pug.stl", false),
            ("dragon_whisker", r"C:\selfDIr\Projects\ColorYourModel\samples\_src\dragon-whisker.stl", false),
            ("kfc_station", r"C:\selfDIr\Projects\ColorYourModel\samples\_src\kfc_station.stl", false),
        ];
        for (name, path, do_render) in fixtures {
            let p = std::path::Path::new(path);
            if !p.exists() {
                eprintln!("[{}] MISSING {} — skipped", name, path);
                continue;
            }
            for deg in [2.0f32, 5.0] {
                let cb = noop_progress();
                let t = Instant::now();
                let mut mesh = match load_stl(p, &*cb) {
                    Ok(m) => m,
                    Err(_) => {
                        eprintln!("[{}] LOAD FAILED", name);
                        continue;
                    }
                };
                let n_faces = mesh.faces.len();
                let detect_min = (n_faces / 500).max(2);
                let planar = detect_planar_regions(
                    &mesh,
                    &PlanarParams { angle_thr_deg: 15.0, dist_thr_factor: 1.0 / 30.0, min_region_faces: detect_min },
                )
                .into_iter()
                .map(|r| r.face_indices)
                .collect::<Vec<_>>();
                let multiview = detect_multiview_regions(
                    &mesh,
                    &MultiViewParams { view_count: 12, angle_thr_deg: 20.0, min_region_faces: detect_min, match_threshold: 1 },
                )
                .into_iter()
                .map(|r| r.face_indices)
                .collect::<Vec<_>>();
                let dihedral_sets = {
                    let _ = segment_by_dihedral_angle(&mut mesh, deg, &*cb);
                    let mut map: HashMap<u32, Vec<u32>> = HashMap::new();
                    for (i, &l) in mesh.segment_labels.iter().enumerate() {
                        map.entry(l).or_default().push(i as u32);
                    }
                    mesh.segment_labels = vec![0u32; n_faces];
                    mesh.segments.clear();
                    map.into_values().collect::<Vec<_>>()
                };
                let r = fuse_region_sets(&mut mesh, &planar, &multiview, &dihedral_sets, &[], 2, 0)
                    .expect("fuse failed");
                let sizes = region_sizes(&mesh);
                eprintln!(
                    "[{} dihedral{:.0}°] fuse: regions={} max_share={:.1}% median={} time={:.1}s (planar={} mv={} dih={})",
                    name,
                    deg,
                    r.region_count,
                    sizes.first().map(|s| *s as f64 / n_faces as f64 * 100.0).unwrap_or(0.0),
                    r.region_size_median,
                    t.elapsed().as_secs_f64(),
                    planar.len(),
                    multiview.len(),
                    dihedral_sets.len(),
                );
                if *do_render {
                    render_partition(&mesh, &format!("fuse_{:.0}deg", deg));
                }
            }
        }
    }


    #[test]
    #[ignore = "reads the 75MB Godzilla STL from disk; run explicitly"]
    fn godzilla_fuse_diagnosis() {
        let mut mesh = match load() {
            Some(m) => m,
            None => {
                eprintln!("GODZILLA STL MISSING — skipped");
                return;
            }
        };
        let n = mesh.faces.len();
        eprintln!("faces={} adjacency_edges={}", n, mesh.face_adjacency.edge_count());

        // ── 1. Raw edge dihedral-angle histogram ────────────────────────
        let buckets: [f32; 13] = [0.5, 1.0, 2.0, 3.0, 5.0, 8.0, 12.0, 20.0, 30.0, 45.0, 60.0, 90.0, 180.0];
        let mut hist = vec![0u64; buckets.len()];
        let mut concave_sharp = 0u64;
        let centers = mesh.face_centers();
        for edge in mesh.face_adjacency.edge_references() {
            let fi = mesh.face_adjacency[edge.source()];
            let fj = mesh.face_adjacency[edge.target()];
            let na = &mesh.normals[fi as usize];
            let nb = &mesh.normals[fj as usize];
            let d = (na[0] * nb[0] + na[1] * nb[1] + na[2] * nb[2]).clamp(-1.0, 1.0);
            let ang = d.acos() * 180.0 / std::f32::consts::PI;
            let bi = match buckets.iter().position(|&b| ang <= b) {
                Some(i) => i,
                None => buckets.len() - 1,
            };
            hist[bi] += 1;
            // Signed test: face centroid vs neighbour plane (concave = centre
            // of face i is on the negative side of j's outward plane).
            let cj = centers[fj as usize];
            let ci = centers[fi as usize];
            let side = (ci[0] - cj[0]) * nb[0] + (ci[1] - cj[1]) * nb[1] + (ci[2] - cj[2]) * nb[2];
            if ang > 5.0 && side < 0.0 {
                concave_sharp += 1;
            }
        }
        let total: u64 = hist.iter().sum();
        let mut lo = 0.0f32;
        for (i, &c) in hist.iter().enumerate() {
            eprintln!("  angle ({:5.1}, {:5.1}] : {:10} ({:5.2}%)", lo, buckets[i], c, c as f64 / total as f64 * 100.0);
            lo = buckets[i];
        }
        eprintln!("  sharp(>5°) concave edges: {} / {}", concave_sharp, total);

        // ── 2. The fuse backbone as shipped: dihedral at 5° ─────────────
        let cb = noop_progress();
        let t = Instant::now();
        segment_by_dihedral_angle(&mut mesh, 5.0, &*cb);
        eprintln!("dihedral(5°) done in {:.2}s", t.elapsed().as_secs_f64());
        report_sizes("dihedral5", &region_sizes(&mesh), n);
        render_partition(&mesh, "dihedral5");

        // ── 3. Reproduce the exact SeedPanel fuse (cut=2, min=0, 5°) ────
        let mut m2 = load().unwrap();
        let detect_min = (n / 500).max(2);
        let planar = detect_planar_regions(
            &m2,
            &PlanarParams { angle_thr_deg: 15.0, dist_thr_factor: 1.0 / 30.0, min_region_faces: detect_min },
        )
        .into_iter()
        .map(|r| r.face_indices)
        .collect::<Vec<_>>();
        let multiview = detect_multiview_regions(
            &m2,
            &MultiViewParams { view_count: 12, angle_thr_deg: 20.0, min_region_faces: detect_min, match_threshold: 1 },
        )
        .into_iter()
        .map(|r| r.face_indices)
        .collect::<Vec<_>>();
        let dihedral_sets = {
            let _ = segment_by_dihedral_angle(&mut m2, 5.0, &*noop_progress());
            let mut map: HashMap<u32, Vec<u32>> = HashMap::new();
            for (i, &l) in m2.segment_labels.iter().enumerate() {
                map.entry(l).or_default().push(i as u32);
            }
            m2.segment_labels = vec![0u32; n];
            m2.segments.clear();
            map.into_values().collect::<Vec<_>>()
        };
        eprintln!(
            "channels: planar={} multiview={} dihedral={}",
            planar.len(),
            multiview.len(),
            dihedral_sets.len()
        );
        let r = fuse_region_sets(&mut m2, &planar, &multiview, &dihedral_sets, &[], 2, 0).unwrap();
        eprintln!(
            "fuse as shipped: regions={} cut={}.raw={} before_merge={} median={} max={} passes={} tiny_before={}",
            r.region_count,
            r.edge_cut.unwrap_or(0),
            r.edge_total.unwrap_or(0),
            r.regions_before_merge,
            r.region_size_median,
            r.region_size_max,
            r.merge_passes,
            r.tiny_regions_before_merge,
        );
        render_partition(&m2, "fuse_as_shipped");

        // Section 4 (FH / concavity / SdfGraphCut alternative backbones) was
        // removed: on this mesh those probes hang in fh.rs's O(crumbs×E)
        // merge_small_components, and the refuted soft-fold plan made them
        // moot — the adopted fix reuses the existing dihedral pipeline at a
        // lower threshold (see godzilla_soft_fold_feasibility).
    }
}

/// Hard-surface regression for the 2° fuse floor: two clean CAD-ish meshes,
/// full fuse path at 2° vs 5°. Guards the "lower floor over-splits bevelled
/// hard-surface models" concern (refuter M5).
#[cfg(test)]
mod fuse_floor_regression {
    use super::godzilla_diagnosis::noop_progress;
    use crate::segment::dihedral::segment_by_dihedral_angle;
    use crate::segment::fuse::fuse_region_sets;
    use crate::segment::planar::{detect_planar_regions, PlanarParams};
    use crate::segment::multiview::{detect_multiview_regions, MultiViewParams};
    use crate::mesh::loader::load_stl;
    use std::collections::HashMap;

    const FIXTURES: &[(&str, &str)] = &[
        ("ring_stand", r"C:\selfDIr\Projects\ColorYourModel\samples\_src\ring-stand.stl"),
        ("cyberpunk_mask", r"C:\selfDIr\Projects\ColorYourModel\samples\_src\cyberpunk-mask.stl"),
    ];

    #[test]
    #[ignore = "reads STL files from disk; run explicitly"]
    fn hard_surface_floor_regression() {
        for (name, path) in FIXTURES {
            let p = std::path::Path::new(path);
            if !p.exists() {
                eprintln!("[{}] MISSING — skipped", name);
                continue;
            }
            for deg in [2.0f32, 5.0] {
                let cb = noop_progress();
                let mut mesh = load_stl(p, &*cb).expect("load");
                let n = mesh.faces.len();
                let detect_min = (n / 500).max(2);
                let planar = detect_planar_regions(
                    &mesh,
                    &PlanarParams { angle_thr_deg: 15.0, dist_thr_factor: 1.0 / 30.0, min_region_faces: detect_min },
                ).into_iter().map(|r| r.face_indices).collect::<Vec<_>>();
                let multiview = detect_multiview_regions(
                    &mesh,
                    &MultiViewParams { view_count: 12, angle_thr_deg: 20.0, min_region_faces: detect_min, match_threshold: 1 },
                ).into_iter().map(|r| r.face_indices).collect::<Vec<_>>();
                let dihedral_sets = {
                    let _ = segment_by_dihedral_angle(&mut mesh, deg, &*cb);
                    let mut map: HashMap<u32, Vec<u32>> = HashMap::new();
                    for (i, &l) in mesh.segment_labels.iter().enumerate() {
                        map.entry(l).or_default().push(i as u32);
                    }
                    mesh.segment_labels = vec![0u32; n];
                    mesh.segments.clear();
                    map.into_values().collect::<Vec<_>>()
                };
                let r = fuse_region_sets(&mut mesh, &planar, &multiview, &dihedral_sets, &[], 2, 0).expect("fuse");
                let mut sizes: Vec<usize> = mesh.segment_labels.iter().fold(HashMap::<u32, usize>::new(), |mut m, l| { *m.entry(*l).or_insert(0) += 1; m }).into_values().collect();
                sizes.sort_unstable_by(|a, b| b.cmp(a));
                eprintln!(
                    "[{} {:.0}°] faces={} fuse_regions={} max_share={:.1}% median={}",
                    name, deg, n, r.region_count,
                    sizes.first().map(|s| *s as f64 / n as f64 * 100.0).unwrap_or(0.0),
                    r.region_size_median,
                );
            }
        }
    }
}

/// Sub-2 deg floor probe: does opening the slider to 0 still terminate, and
/// what does it buy? fuse E2E at 0 / 0.5 / 1 deg on Godzilla (region counts,
/// wall clock, renders). 0 deg disconnects EVERY face before phase-3, so this
/// also stress-tests the normal-consistency merge at 1.5M singleton regions.
#[cfg(test)]
mod fuse_floor_zero {
    use super::godzilla_diagnosis::{noop_progress, region_sizes, render_partition, GODZILLA};
    use crate::mesh::loader::load_stl;
    use crate::segment::dihedral::segment_by_dihedral_angle;
    use crate::segment::fuse::fuse_region_sets;
    use crate::segment::planar::{detect_planar_regions, PlanarParams};
    use crate::segment::multiview::{detect_multiview_regions, MultiViewParams};
    use std::collections::HashMap;
    use std::time::Instant;

    #[test]
    #[ignore = "reads the 75MB Godzilla STL from disk; run explicitly"]
    fn fuse_at_sub2_deg_floors() {
        let p = std::path::Path::new(GODZILLA);
        for deg in [0.0f32, 0.5, 1.0] {
            let cb = noop_progress();
            let t = Instant::now();
            let mut mesh = load_stl(p, &*cb).expect("load");
            let n = mesh.faces.len();
            let detect_min = (n / 500).max(2);
            let planar = detect_planar_regions(
                &mesh,
                &PlanarParams { angle_thr_deg: 15.0, dist_thr_factor: 1.0 / 30.0, min_region_faces: detect_min },
            ).into_iter().map(|r| r.face_indices).collect::<Vec<_>>();
            let multiview = detect_multiview_regions(
                &mesh,
                &MultiViewParams { view_count: 12, angle_thr_deg: 20.0, min_region_faces: detect_min, match_threshold: 1 },
            ).into_iter().map(|r| r.face_indices).collect::<Vec<_>>();
            let dihedral_sets = {
                let _ = segment_by_dihedral_angle(&mut mesh, deg, &*cb);
                let mut map: HashMap<u32, Vec<u32>> = HashMap::new();
                for (i, &l) in mesh.segment_labels.iter().enumerate() {
                    map.entry(l).or_default().push(i as u32);
                }
                mesh.segment_labels = vec![0u32; n];
                mesh.segments.clear();
                map.into_values().collect::<Vec<_>>()
            };
            let r = fuse_region_sets(&mut mesh, &planar, &multiview, &dihedral_sets, &[], 2, 0).expect("fuse");
            let sizes = region_sizes(&mesh);
            eprintln!(
                "[godzilla {:.1} deg] fuse: regions={} max_share={:.1}% median={} total={:.1}s",
                deg, r.region_count,
                sizes.first().map(|s| *s as f64 / n as f64 * 100.0).unwrap_or(0.0),
                r.region_size_median,
                t.elapsed().as_secs_f64(),
            );
            render_partition(&mesh, &format!("fuse_{:.1}deg", deg));
        }
    }
}

/// Lasso interaction timing on the real Godzilla mesh — the user-reported
/// "one lasso region took over a minute" reproduction. Times the exact
/// per-click path (`snap_point_to_vertex_on_face`, called once per lasso
/// click) and the close path (`region_from_loop`, called once on closure).
/// Run in DEBUG to see what dev-mode users feel:
///   cargo test --lib lasso_debug_timing -- --ignored --nocapture
/// (and with --release for the shipped-build reference).
#[cfg(test)]
mod lasso_timing {
    use super::godzilla_diagnosis::{GODZILLA, noop_progress};
    use crate::mesh::loader::load_stl;
    use crate::segment::manual::{region_from_loop, snap_point_to_vertex_on_face};
    use std::time::Instant;

    #[test]
    #[ignore = "reads the 75MB Godzilla STL from disk; run explicitly"]
    fn lasso_debug_timing() {
        let mesh = load_stl(std::path::Path::new(GODZILLA), &*noop_progress()).expect("load");
        let fi: u32 = 510313; // the face under the user's cursor in the field report
        let tri = mesh.faces[fi as usize];
        let c = [
            (mesh.vertices[tri[0] as usize][0]
                + mesh.vertices[tri[1] as usize][0]
                + mesh.vertices[tri[2] as usize][0]) / 3.0,
            (mesh.vertices[tri[0] as usize][1]
                + mesh.vertices[tri[1] as usize][1]
                + mesh.vertices[tri[2] as usize][1]) / 3.0,
            (mesh.vertices[tri[0] as usize][2]
                + mesh.vertices[tri[1] as usize][2]
                + mesh.vertices[tri[2] as usize][2]) / 3.0,
        ];
        let t = Instant::now();
        let mut snaps = 0;
        for _ in 0..8 {
            if snap_point_to_vertex_on_face(&mesh, &c, fi).is_some() {
                snaps += 1;
            }
        }
        eprintln!("8 lasso clicks (snap_point_to_vertex_on_face): {:.2}s ({} ok)", t.elapsed().as_secs_f64(), snaps);

        let pts = vec![
            mesh.vertices[tri[0] as usize],
            mesh.vertices[tri[1] as usize],
            mesh.vertices[tri[2] as usize],
        ];
        let faces = vec![fi, fi, fi];
        let t = Instant::now();
        let region = region_from_loop(&mesh, &pts, &faces);
        eprintln!(
            "1 tiny lasso close (region_from_loop): {:.2}s -> {} faces",
            t.elapsed().as_secs_f64(),
            region.len()
        );

        // REALISTIC loop: six clicks spread ~8-15 units apart around the same
        // area — the shape of a real lasso stroke, where consecutive clicked
        // points are far apart and every gap costs a full-graph Dijkstra.
        let mut realistic_pts: Vec<[f32; 3]> = Vec::new();
        for &(dx, dy) in &[
            (0.0f32, 0.0f32),
            (8.0, 0.0),
            (12.0, 6.0),
            (6.0, 12.0),
            (-4.0, 10.0),
            (-6.0, 3.0),
        ] {
            let q = [c[0] + dx, c[1] + dy, c[2]];
            if let Some((vi, _)) = mesh.nearest_vertex(&q) {
                realistic_pts.push(mesh.vertices[vi as usize]);
            }
        }
        let t = Instant::now();
        let region = region_from_loop(&mesh, &realistic_pts, &[]);
        eprintln!(
            "1 realistic 6-point lasso close: {:.2}s -> {} faces",
            t.elapsed().as_secs_f64(),
            region.len()
        );
    }
}

/// Measures the debug-build cost of serializing the `SegmentResult` IPC
/// payload for a 1.5M-face mesh (the lasso finalize ships full label + colour
/// buffers). Suspected residual lag after the compute fixes.
#[cfg(test)]
mod ipc_payload_timing {
    #[test]
    #[ignore = "allocation-heavy timing probe; run explicitly"]
    fn segment_result_json_timing() {
        let n = 1_499_964usize;
        let labels: Vec<u32> = (0..n as u32).map(|i| 100_000 + i % 90).collect();
        let colors: Vec<u8> = (0..n * 4).map(|i| (i % 251) as u8).collect();
        let t = std::time::Instant::now();
        let s = serde_json::to_string(&serde_json::json!({
            "segmentLabels": labels,
            "faceColors": colors,
        }))
        .unwrap();
        eprintln!(
            "serialize 1.5M labels + 6M colours: {:.2}s ({} MB)",
            t.elapsed().as_secs_f64(),
            s.len() / 1_048_576
        );
    }
}
