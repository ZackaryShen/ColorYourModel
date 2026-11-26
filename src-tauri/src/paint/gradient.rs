use std::collections::{HashMap, HashSet};

use nalgebra::Vector3;
use parry3d::{
    math::{Pose, Vector},
    query::details::distance_support_map_support_map,
    shape::{Segment, Triangle},
};
use petgraph::graph::NodeIndex;
use serde::{Deserialize, Serialize};

pub use crate::mesh::model::GradientSample;
use crate::mesh::model::MeshModel;

pub(crate) const MAX_STOPS: usize = 8;
pub(crate) const MAX_POINTS: usize = 4096;
pub(crate) const MAX_RADIUS: f32 = 200.0;
const MAX_QUERY_COUNT: usize = 8192;
const MAX_CANDIDATE_CHECKS: usize = 16_000_000;
const MIN_DIRECTION: f64 = 1.0e-5;

#[derive(Debug, Clone, Copy, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GradientHit {
    pub face_id: u32,
    pub point: [f32; 3],
}

type Point = Vector3<f64>;

pub fn validate(mesh: &MeshModel, stops: &[[u8; 4]], radius: f32) -> Result<(), String> {
    if mesh.texture_painting {
        return Err("Gradient paint is unavailable on texture projects".into());
    }
    if !(2..=MAX_STOPS).contains(&stops.len()) {
        return Err(format!("Gradient paint needs 2 to {MAX_STOPS} color stops"));
    }
    if !radius.is_finite() || radius <= 0.0 || radius > MAX_RADIUS {
        return Err("Invalid gradient paint radius".into());
    }
    Ok(())
}

fn point(coordinates: [f32; 3]) -> Point {
    Point::from(coordinates.map(f64::from))
}

pub fn hit_point(mesh: &MeshModel, hit: GradientHit) -> Result<Point, String> {
    let face = mesh
        .faces
        .get(hit.face_id as usize)
        .ok_or("Invalid gradient face")?;
    let hit = point(hit.point);
    let [a, b, c] = face.map(|i| point(mesh.vertices[i as usize]));
    let normal = (b - a).cross(&(c - a));
    let area = normal.norm();
    let tolerance = 1.0e-5 + a.amax().max(b.amax()).max(c.amax()) * f64::from(f32::EPSILON) * 4.0;
    if !hit.iter().all(|v| v.is_finite())
        || area == 0.0
        || (hit - a).dot(&normal).abs() > area * tolerance
    {
        return Err("Invalid gradient hit point".into());
    }
    let normal = normal / area;
    for (u, v) in [(a, b), (b, c), (c, a)] {
        let edge = v - u;
        if edge.cross(&(hit - u)).dot(&normal) < -tolerance * edge.norm() {
            return Err("Gradient hit lies outside its face".into());
        }
    }
    Ok(hit)
}

fn sample(stops: &[[u8; 4]], progress: f64) -> GradientSample {
    let position = progress.clamp(0.0, 1.0) * (stops.len() - 1) as f64;
    let index = (position as usize).min(stops.len() - 2);
    GradientSample {
        from: stops[index],
        to: stops[index + 1],
        mix: ((position - index as f64) * f64::from(u16::MAX)).round() as u16,
        ramp_id: 0,
    }
}

pub fn fill_samples(
    mesh: &MeshModel,
    faces: impl IntoIterator<Item = u32>,
    start: Point,
    end: Point,
    stops: &[[u8; 4]],
) -> Result<Vec<(u32, GradientSample)>, String> {
    let direction = end - start;
    let length_squared = direction.norm_squared();
    if length_squared < MIN_DIRECTION * MIN_DIRECTION {
        return Err("Drag across the model to set the gradient direction".into());
    }
    Ok(faces
        .into_iter()
        .map(|face| {
            let progress = (point(mesh.face_center(face)) - start).dot(&direction) / length_squared;
            (face, sample(stops, progress))
        })
        .collect())
}

#[derive(Clone, Copy)]
struct PaintCandidate {
    distance_squared: f64,
    path_distance: f64,
}

struct BrushSpan {
    start: Point,
    direction: Point,
    length: f64,
    path_distance: f64,
}

impl BrushSpan {
    fn project(&self, face_center: Point) -> PaintCandidate {
        let offset = face_center - self.start;
        let t = if self.length > 0.0 {
            (offset.dot(&self.direction) / (self.length * self.length)).clamp(0.0, 1.0)
        } else {
            0.0
        };
        PaintCandidate {
            distance_squared: (offset - self.direction * t).norm_squared(),
            path_distance: self.path_distance + self.length * t,
        }
    }
}

fn add_candidate(candidates: &mut HashMap<u32, PaintCandidate>, face: u32, hit: PaintCandidate) {
    let best = candidates.entry(face).or_insert(hit);
    if hit.distance_squared < best.distance_squared {
        *best = hit;
    }
}

fn check_budget(checks: &mut usize, cost: usize) -> Result<(), String> {
    *checks += cost;
    if *checks > MAX_CANDIDATE_CHECKS {
        return Err("Gradient brush stroke covers too many faces".into());
    }
    Ok(())
}

fn visible(mesh: &MeshModel, face: u32) -> bool {
    !mesh
        .segment_labels
        .get(face as usize)
        .is_some_and(|label| mesh.hidden_labels.contains(label))
}

fn parry_vector(p: Point) -> Vector {
    Vector::new(p.x as f32, p.y as f32, p.z as f32)
}

/// A thin stroke can cross a large triangle without passing near its centre.
/// Follow only triangles the centreline actually crosses on the hit surface.
fn follow_centerline(
    mesh: &MeshModel,
    anchors: [u32; 2],
    span: &BrushSpan,
    radius: f64,
    candidates: &mut HashMap<u32, PaintCandidate>,
    candidate_checks: &mut usize,
) -> Result<(), String> {
    let tolerance = radius.min(1.0e-5 + span.length * f64::from(f32::EPSILON) * 4.0);
    let line = Segment::new(Vector::ZERO, parry_vector(span.direction));
    let mut visited = HashSet::from(anchors);
    let mut reached: Vec<_> = anchors
        .into_iter()
        .filter(|&face| visible(mesh, face))
        .collect();
    let mut index = 0;
    while let Some(&current) = reached.get(index) {
        for node in mesh
            .face_adjacency
            .neighbors(NodeIndex::new(current as usize))
        {
            let face = node.index() as u32;
            if !visited.insert(face) || !visible(mesh, face) {
                continue;
            }
            check_budget(candidate_checks, 1)?;
            let vertices =
                mesh.faces[face as usize].map(|v| point(mesh.vertices[v as usize]) - span.start);
            // Reject separated bounding boxes before the exact convex distance.
            let misses = (0..3).any(|axis| {
                let min = span.direction[axis].min(0.0) - tolerance;
                let max = span.direction[axis].max(0.0) + tolerance;
                vertices.iter().all(|v| v[axis] < min) || vertices.iter().all(|v| v[axis] > max)
            });
            if misses {
                continue;
            }
            let [a, b, c] = vertices.map(parry_vector);
            if distance_support_map_support_map(&Pose::IDENTITY, &line, &Triangle::new(a, b, c))
                <= tolerance as f32
            {
                add_candidate(
                    candidates,
                    face,
                    span.project(point(mesh.face_center(face))),
                );
                reached.push(face);
            }
        }
        index += 1;
    }
    Ok(())
}

pub fn brush_samples(
    mesh: &MeshModel,
    hits: &[GradientHit],
    stops: &[[u8; 4]],
    radius: f32,
) -> Result<Vec<(u32, GradientSample)>, String> {
    if hits.is_empty() || hits.len() > MAX_POINTS {
        return Err("Invalid number of gradient brush points".into());
    }
    let hits: Vec<_> = hits
        .iter()
        .map(|&hit| hit_point(mesh, hit).map(|point| (hit.face_id, point)))
        .collect::<Result<_, _>>()?;
    let mut candidates = HashMap::new();
    let mut path_distance = 0.0;
    let mut query_count = 0;
    let mut candidate_checks = 0;
    let radius = f64::from(radius);

    let first = hits[0];
    for (&(face, end), (start_face, start)) in hits
        .iter()
        .zip(std::iter::once(first).chain(hits.iter().copied()))
    {
        let direction = end - start;
        let segment_length = direction.norm();
        let span = BrushSpan {
            start,
            direction,
            length: segment_length,
            path_distance,
        };
        if segment_length > 0.0 || query_count == 0 {
            // Query an enclosing sphere for each short capsule, then check
            // the exact point-to-segment distance for a gap-free stroke.
            let subdivisions = (segment_length / (2.0 * radius)).ceil().max(1.0) as usize;
            if subdivisions > MAX_QUERY_COUNT - query_count {
                return Err("Gradient brush stroke is too long for this radius".into());
            }
            query_count += subdivisions;
            let query_radius = (radius + segment_length / (2.0 * subdivisions as f64)) as f32;
            for division in 0..subdivisions {
                check_budget(&mut candidate_checks, mesh.face_radius_scan_cost())?;
                let center = start + direction * ((division as f64 + 0.5) / subdivisions as f64);
                let faces = mesh.faces_within_radius(
                    &[center.x as f32, center.y as f32, center.z as f32],
                    query_radius,
                );
                check_budget(&mut candidate_checks, faces.len())?;
                for face in faces {
                    let hit = span.project(point(mesh.face_center(face)));
                    if hit.distance_squared < radius * radius {
                        add_candidate(&mut candidates, face, hit);
                    }
                }
            }
        }
        if start_face != face && segment_length > 0.0 {
            follow_centerline(
                mesh,
                [start_face, face],
                &span,
                radius,
                &mut candidates,
                &mut candidate_checks,
            )?;
        }
        path_distance += segment_length;
        // On a coarse triangle the picked surface point may lie beyond the
        // brush radius from its centroid. It is still directly in the stroke.
        candidates.entry(face).or_insert(PaintCandidate {
            distance_squared: f64::INFINITY,
            path_distance,
        });
    }

    // Only keep patches connected to a known ray hit by edges within the
    // stroke. A nearby wall or separate mesh shell can share the radius.
    let mut reached = Vec::new();
    let mut take_candidate = |face| {
        if !visible(mesh, face) {
            return None;
        }
        candidates.remove(&face).map(|hit| {
            let progress = if path_distance > 0.0 {
                hit.path_distance / path_distance
            } else {
                0.0
            };
            (face, sample(stops, progress))
        })
    };
    reached.extend(hits.iter().filter_map(|&(face, _)| take_candidate(face)));
    let mut index = 0;
    while let Some(&(face, _)) = reached.get(index) {
        reached.extend(
            mesh.face_adjacency
                .neighbors(NodeIndex::new(face as usize))
                .filter_map(|node| take_candidate(node.index() as u32)),
        );
        index += 1;
    }
    Ok(reached)
}

#[cfg(test)]
mod footprint_tests {
    use super::*;

    const STOPS: [[u8; 4]; 2] = [[186, 12, 47, 255], [0, 255, 255, 255]];

    fn ray_hit(mesh: &MeshModel, x: f64, z: f64, back: bool) -> GradientHit {
        let mut nearest = None;
        for (index, face) in mesh.faces.iter().enumerate() {
            let [a, b, c] = face.map(|v| point(mesh.vertices[v as usize]));
            let ab = b - a;
            let ac = c - a;
            let det = ab.x * ac.z - ab.z * ac.x;
            if det.abs() < 1e-12 {
                continue;
            }
            let t = ((x - a.x) * ac.z - (z - a.z) * ac.x) / det;
            let u = (ab.x * (z - a.z) - ab.z * (x - a.x)) / det;
            if t >= 0.0 && u >= 0.0 && t + u <= 1.0 {
                let y = a.y + t * ab.y + u * ac.y;
                if nearest.is_none_or(|(closest, _)| if back { y > closest } else { y < closest }) {
                    nearest = Some((y, index as u32));
                }
            }
        }
        let (y, face_id) = nearest.expect("the test stroke must stay on the model");
        GradientHit {
            face_id,
            point: [x as f32, y as f32, z as f32],
        }
    }

    fn assert_local_edits(mesh: &MeshModel, hits: &[GradientHit], tolerance: f64) -> HashSet<u32> {
        let updated: HashSet<u32> = brush_samples(mesh, hits, &STOPS, 3.5)
            .unwrap()
            .into_iter()
            .map(|(face, _)| face)
            .collect();
        let stroke: Vec<_> = hits
            .windows(2)
            .map(|pair| {
                let start = point(pair[0].point);
                let direction = point(pair[1].point) - start;
                BrushSpan {
                    start,
                    direction,
                    length: direction.norm(),
                    path_distance: 0.0,
                }
            })
            .collect();
        let distance = |vertex: u32| {
            let p = point(mesh.vertices[vertex as usize]);
            stroke
                .iter()
                .map(|span| span.project(p).distance_squared)
                .fold(f64::INFINITY, f64::min)
                .sqrt()
        };
        let max_distance = updated
            .iter()
            .flat_map(|&face| mesh.faces[face as usize])
            .map(distance)
            .fold(0.0, f64::max);
        eprintln!("native stroke: mesh_faces={}, painted_faces={}, maximum distance from path={max_distance:.3} mm",
            mesh.faces.len(), updated.len());
        assert!(
            hits.iter().all(|hit| updated.contains(&hit.face_id)),
            "some positions along the stroke were left unpainted"
        );
        assert!(
            max_distance < 3.5 + tolerance,
            "stroke leaks to {max_distance:.3} mm from the centreline at a 3.5 mm brush radius"
        );
        updated
    }

    #[test]
    fn brush_stays_local_across_a_native_paint_tree_boundary() {
        use crate::threemf::test_support::{mesh_object, model_xml, Fixture};

        let object = mesh_object(
            1,
            "Wall",
            &[
                [-4.0, 0.0, 0.0],
                [4.0, 0.0, 0.0],
                [-4.0, 0.0, 26.0],
                [4.0, 0.0, 26.0],
            ],
            &[[0, 1, 2], [1, 3, 2]],
            &[Some("0C0C1"), Some("8")],
        );
        let config = serde_json::json!({
            "filament_colour": ["#000000", "#040404"],
            "mixed_filament_definitions": "1,2,1,1,50,0,g,w,m2,z0,d0",
        });
        let fixture = Fixture::new(&[
            (
                "3D/3dmodel.model",
                model_xml("millimeter", &object, r#"<item objectid="1"/>"#),
            ),
            (
                "Metadata/project_settings.config",
                serde_json::to_vec(&config).unwrap(),
            ),
        ]);
        let mesh = crate::threemf::load_plate(fixture.path(), 0, &|_, _| {})
            .unwrap()
            .mesh;
        let hits = [
            ray_hit(&mesh, -3.0, 3.0, false),
            ray_hit(&mesh, 3.0, 23.0, false),
        ];
        let painted = assert_local_edits(&mesh, &hits, 1.2);
        // Only the endpoints were sampled. The centreline and both sides of
        // the brush must be painted through the shared edge under the stroke.
        for altitude in 6..=20 {
            let altitude = f64::from(altitude);
            let x = (altitude - 13.0) * 0.3;
            for offset in [-1.8, 0.0, 1.8] {
                assert!(painted.contains(&ray_hit(&mesh, x + offset, altitude, false).face_id));
            }
        }
        let mut edge_counts = HashMap::new();
        let mut area_by_material = HashMap::new();
        for (id, &[a, b, c]) in mesh.faces.iter().enumerate() {
            for (u, v) in [(a, b), (b, c), (c, a)] {
                *edge_counts.entry((u.min(v), u.max(v))).or_insert(0) += 1;
            }
            let [a, b, c] = [a, b, c].map(|v| point(mesh.vertices[v as usize]));
            assert!(
                (b - a).cross(&(c - a)).y < 0.0,
                "split faces keep the source winding"
            );
            let material = (mesh.source_material_slots[id], mesh.face_colors[id]);
            *area_by_material.entry(material).or_insert(0.0) +=
                (b - a).cross(&(c - a)).norm() * 0.5;
        }
        for ((a, b), count) in edge_counts {
            if count != 2 {
                let [a, b] = [a, b].map(|v| mesh.vertices[v as usize]);
                assert_eq!(count, 1);
                assert!(
                    (a[0] == b[0] && a[0].abs() == 4.0)
                        || (a[2] == b[2] && [0.0, 26.0].contains(&a[2])),
                    "interior edge is split on one side only: {a:?}–{b:?}"
                );
            }
        }
        assert_eq!(
            area_by_material,
            HashMap::from([((2, [4, 4, 4, 255]), 104.0), ((3, [2, 2, 2, 255]), 104.0)])
        );
    }

    #[test]
    #[ignore = "requires CYM_REAL_GRADIENT_BRUSH_3MF pointing at the user's cylinder"]
    fn brush_footprint_stays_local_on_the_real_native_cylinder() {
        let source =
            std::env::var("CYM_REAL_GRADIENT_BRUSH_3MF").expect("CYM_REAL_GRADIENT_BRUSH_3MF");
        let source = std::path::Path::new(&source);
        let mut plate = crate::threemf::load_plate(source, 0, &|_, _| {}).unwrap();
        let mesh = &mut plate.mesh;
        let output = std::env::var_os("CYM_GRADIENT_BRUSH_SAVE_AS");
        for back in [false, true] {
            let hits: Vec<_> = (0..=60)
                .map(|n| {
                    let t = f64::from(n) / 60.0;
                    ray_hit(mesh, 126.0 + 19.0 * t, 5.0 + 17.0 * t, back)
                })
                .collect();
            let painted = assert_local_edits(mesh, &hits, 1.2);
            for n in 1..20 {
                let t = f64::from(n) / 20.0;
                for offset in [-1.5, 1.5] {
                    let hit = ray_hit(mesh, 126.0 + 19.0 * t + offset, 5.0 + 17.0 * t, back);
                    assert!(
                        painted.contains(&hit.face_id),
                        "gap on the cylinder at {hit:?}"
                    );
                }
            }
            if let Some(output) = output.as_ref().filter(|_| back) {
                save_real_stroke(source, output, &plate.name, mesh, hits);
            }
        }
    }

    fn save_real_stroke(
        source: &std::path::Path,
        output: &std::ffi::OsStr,
        plate_name: &str,
        mesh: &mut MeshModel,
        hits: Vec<GradientHit>,
    ) {
        use crate::export::source_presets::SourcePresets;
        use crate::export::threemf::{write_project_3mf, ProjectPlate, ProjectWritePurpose};
        use crate::mesh::history::{GradientContext, GradientTool};

        let output = std::path::Path::new(output);
        assert_ne!(source.canonicalize().ok(), output.canonicalize().ok());
        let mut samples = brush_samples(mesh, &hits, &STOPS, 3.5).unwrap();
        samples.sort_unstable_by_key(|&(face, _)| face);
        eprintln!(
            "saved stroke endpoints: {:?} to {:?}",
            hits.first(),
            hits.last()
        );
        mesh.apply_gradient(
            &samples,
            GradientContext {
                tool: GradientTool::Brush,
                stops: STOPS.into(),
                points: hits.into(),
                radius: 3.5,
                whole_region: false,
            },
        );
        let source = SourcePresets::read(source).unwrap().unwrap();
        write_project_3mf(
            &[ProjectPlate {
                name: plate_name.into(),
                mesh,
            }],
            0,
            output,
            None,
            Some(&source),
            ProjectWritePurpose::EngineeringSave,
            &|_, _| {},
        )
        .unwrap();
        let mut loaded = crate::threemf::load_plate(output, 0, &|_, _| {})
            .unwrap()
            .mesh;
        assert_eq!(loaded.faces, mesh.faces);
        assert_eq!(loaded.gradient_faces, mesh.gradient_faces);
        assert_eq!(loaded.face_colors, mesh.face_colors);
        assert!(loaded
            .history
            .undo_with_gradients(
                &mut loaded.face_colors,
                &mut loaded.segment_labels,
                &mut loaded.gradient_faces
            )
            .is_some());
        assert!(loaded.gradient_faces.is_empty());
        assert!(loaded
            .history
            .redo_with_gradients(
                &mut loaded.face_colors,
                &mut loaded.segment_labels,
                &mut loaded.gradient_faces
            )
            .is_some());
        assert_eq!(loaded.gradient_faces, mesh.gradient_faces);
    }
}
