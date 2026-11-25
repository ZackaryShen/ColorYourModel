//! Assemble one plate's build items into a single mesh.
//!
//! A plate is a set of build items; each item names an object whose geometry is
//! either an inline mesh or a tree of components that may cross model parts.
//! Assembly bakes `build × component` transforms (the build item is the outer
//! transform, so a component's translation is also scaled by a scaling build
//! item), flattens the component tree, and merges every object of the plate
//! into one vertex/face/paint triple.
//!
//! The merge deduplicates vertices by coordinate bit pattern, the same rule the
//! STL loader uses, and appends each mesh's face indices with an offset. That
//! reordering is deliberately **not** applied to a plate holding exactly one
//! untouched object: the fingerprinter downstream hashes this geometry, so a
//! single object with an identity placement is cloned verbatim (order, duplicate
//! coordinates and paint values untouched) and the project still reopens.
//!
//! Missing geometry is never degraded to an empty mesh: an object that resolves
//! to no mesh, a component naming a missing part or object, out-of-range face
//! indices and deeper-than-[`MAX_DEPTH`] nesting are all `Err`.

use std::collections::{HashMap, HashSet};

use super::model::{RawMesh, Scene, Transform};
use super::plates::{ModelSettings, PlateGroup};

/// One plate's geometry, with `paint[i]` and `base_slots[i]` belonging to
/// `faces[i]`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct AssembledMesh {
    pub vertices: Vec<[f32; 3]>,
    pub faces: Vec<[u32; 3]>,
    /// Raw `paint_color` values, parallel to `faces` in traversal order.
    pub paint: Vec<Option<String>>,
    /// The object's `model_settings` `extruder` (1-based filament slot) for
    /// each face, parallel to `faces`. It is the face's base colour when its
    /// `paint_color` is absent (or state 0). `None` means the object declares
    /// no extruder and the reader falls back to the slicer's slot 1.
    pub base_slots: Vec<Option<u8>>,
}

/// Bounds traversal borrows source geometry; no inactive editable mesh is allocated.
pub fn item_bounds(
    scene: &Scene,
    item_index: usize,
) -> Result<Option<([f32; 3], [f32; 3])>, String> {
    let item = scene
        .build
        .get(item_index)
        .ok_or("Build item is outside the scene")?;
    let part = build_part_key(scene, item.object_id)?;
    let mut pieces = Vec::new();
    collect_pieces(scene, part, item.object_id, item.transform, 0, &mut pieces)?;
    let mut low = [f32::INFINITY; 3];
    let mut high = [f32::NEG_INFINITY; 3];
    let mut any = false;
    for piece in pieces {
        for &vertex in &piece.mesh.vertices {
            let world = piece.transform.apply(vertex);
            for axis in 0..3 {
                low[axis] = low[axis].min(world[axis]);
                high[axis] = high[axis].max(world[axis]);
            }
            any = true;
        }
    }
    Ok(any.then_some((low, high)))
}

pub fn item_face_count(scene: &Scene, item_index: usize) -> Result<u32, String> {
    let item = scene
        .build
        .get(item_index)
        .ok_or("Build item is outside the scene")?;
    let part = build_part_key(scene, item.object_id)?;
    let mut pieces = Vec::new();
    collect_pieces(scene, part, item.object_id, item.transform, 0, &mut pieces)?;
    let count: usize = pieces
        .iter()
        .map(|piece| {
            piece
                .mesh
                .paint
                .iter()
                .map(|raw| {
                    raw.as_deref()
                        .and_then(|raw| super::paint::decode(raw).ok())
                        .map(|tree| super::paint::leaf_count(&tree))
                        .unwrap_or(1)
                })
                .sum::<usize>()
        })
        .sum();
    u32::try_from(count).map_err(|_| "Plate exceeds supported face count".into())
}

/// Component nesting followed before assembly gives up. Real Orca/Bambu
/// projects nest one or two levels; the cap turns a cyclic `p:path` graph into
/// an error instead of unbounded recursion.
const MAX_DEPTH: usize = 32;

/// One mesh plus the transform that places it in plate space.
struct Piece<'a> {
    mesh: &'a RawMesh,
    transform: Transform,
    // The mesh's source part and object, used to name the culprit in errors.
    part: &'a str,
    object_id: u32,
}

/// Assemble the build items of one plate into a single mesh.
///
/// `settings` supplies each build object's base extruder: a plate may mix
/// objects on different filaments, so the base travels with every face rather
/// than being resolved once for the plate.
///
/// A plate with no build items yields an empty mesh. An error carries the
/// offending object or build index, so a failed import can name what is broken.
pub fn assemble(
    scene: &Scene,
    group: &PlateGroup,
    settings: &ModelSettings,
) -> Result<AssembledMesh, String> {
    if group.item_indices.is_empty() {
        return Ok(AssembledMesh::default());
    }
    if let Some((mesh, object_id)) = single_verbatim_mesh(scene, group) {
        let base = object_base(settings, object_id);
        return Ok(AssembledMesh {
            vertices: mesh.vertices.clone(),
            faces: mesh.faces.clone(),
            paint: mesh.paint.clone(),
            // The fast path must not reorder or dedupe geometry (the
            // fingerprint depends on it); the per-face bases are built
            // alongside in face order.
            base_slots: vec![base; mesh.faces.len()],
        });
    }
    bake_and_merge(scene, group, settings)
}

/// The `model_settings` `extruder` of a build object, 1-based (zero means default).
///
/// The lookup is by the **build item's** object id — the id the plate lists
/// and `model_settings.config` keys — because `plates.rs` does not parse
/// per-`<part>` metadata and a component's object id can collide with an
/// unrelated top-level entry.
fn object_base(settings: &ModelSettings, object_id: u32) -> Option<u8> {
    settings.objects.get(&object_id)?.extruder
}

/// The untouched mesh of a plate that holds exactly one object, if this plate
/// qualifies for the verbatim path. The object id is returned so the caller can
/// resolve that object's base extruder.
///
/// Qualifying means a single build item with the identity transform whose
/// object is an inline mesh with no components, or a component-free leaf
/// reached through exactly one identity component. No dedupe and no reordering
/// happens on this path; the geometry fingerprint downstream depends on it.
fn single_verbatim_mesh<'a>(scene: &'a Scene, group: &PlateGroup) -> Option<(&'a RawMesh, u32)> {
    let [index] = group.item_indices.as_slice() else {
        return None;
    };
    let item = scene.build.get(*index)?;
    if item.transform != Transform::identity() {
        return None;
    }
    let part = build_part_key(scene, item.object_id).ok()?;
    let object = scene.parts.get(part)?.get(&item.object_id)?;
    let mesh = if object.components.is_empty() {
        object.mesh.as_ref()?
    } else if object.mesh.is_none() && object.components.len() == 1 {
        let component = &object.components[0];
        if component.transform != Transform::identity() {
            return None;
        }
        let child_part = component.path.as_deref().unwrap_or(part);
        let child = scene.parts.get(child_part)?.get(&component.object_id)?;
        if !child.components.is_empty() {
            return None;
        }
        child.mesh.as_ref()?
    } else {
        return None;
    };
    // Validation does not touch the order. A malformed mesh falls through to
    // the baking path only so the error is produced in one place.
    validate_mesh(mesh, item.object_id, part).ok()?;
    Some((mesh, item.object_id))
}

/// Bake every build item's geometry into one deduplicated mesh.
fn bake_and_merge(
    scene: &Scene,
    group: &PlateGroup,
    settings: &ModelSettings,
) -> Result<AssembledMesh, String> {
    let mut out = AssembledMesh::default();
    let mut vertex_map: HashMap<[u32; 3], u32> = HashMap::new();

    for &index in &group.item_indices {
        let item = scene.build.get(index).ok_or_else(|| {
            format!(
                "plate lists build item {index}, but the build has {} items",
                scene.build.len()
            )
        })?;
        let part = build_part_key(scene, item.object_id)?;
        let base = object_base(settings, item.object_id);
        let mut pieces = Vec::new();
        collect_pieces(scene, part, item.object_id, item.transform, 0, &mut pieces)?;
        if pieces.is_empty() {
            return Err(format!(
                "object {} resolves to no geometry in any component",
                item.object_id
            ));
        }
        for piece in pieces {
            validate_mesh(piece.mesh, piece.object_id, piece.part)?;
            append_piece(piece, base, &mut out, &mut vertex_map)?;
        }
    }
    Ok(out)
}

/// Flatten an object into the meshes it contributes, innermost component first.
///
/// `transform` is the accumulated outer transform. A component's transform is
/// applied to its own geometry before that accumulated transform, which is why
/// the child receives `transform.compose(component.transform)`.
fn collect_pieces<'a>(
    scene: &'a Scene,
    part: &'a str,
    object_id: u32,
    transform: Transform,
    depth: usize,
    pieces: &mut Vec<Piece<'a>>,
) -> Result<(), String> {
    if depth > MAX_DEPTH {
        return Err(format!(
            "object {object_id} exceeds the {MAX_DEPTH}-level component nesting limit"
        ));
    }
    let objects = scene.parts.get(part).ok_or_else(|| {
        format!("object {object_id} names model part {part:?}, which the package does not contain")
    })?;
    let object = objects
        .get(&object_id)
        .ok_or_else(|| format!("model part {part:?} has no object {object_id}"))?;

    if let Some(mesh) = &object.mesh {
        pieces.push(Piece {
            mesh,
            transform,
            part,
            object_id,
        });
    }
    for component in &object.components {
        let child_part = component.path.as_deref().unwrap_or(part);
        collect_pieces(
            scene,
            child_part,
            component.object_id,
            transform.compose(component.transform),
            depth + 1,
            pieces,
        )?;
    }
    Ok(())
}

/// Append one baked mesh, reusing vertices already placed at the same
/// coordinate bit pattern and offsetting face indices into the merged mesh.
fn append_piece(
    piece: Piece,
    base: Option<u8>,
    out: &mut AssembledMesh,
    vertex_map: &mut HashMap<[u32; 3], u32>,
) -> Result<(), String> {
    let mut indices = Vec::with_capacity(piece.mesh.vertices.len());
    for vertex in &piece.mesh.vertices {
        let baked = piece.transform.apply(*vertex);
        let key = [baked[0].to_bits(), baked[1].to_bits(), baked[2].to_bits()];
        let index = match vertex_map.get(&key) {
            Some(&index) => index,
            None => {
                let index = u32::try_from(out.vertices.len())
                    .map_err(|_| "assembled mesh exceeds u32::MAX vertices".to_string())?;
                vertex_map.insert(key, index);
                out.vertices.push(baked);
                index
            }
        };
        indices.push(index);
    }

    for (face_index, face) in piece.mesh.faces.iter().enumerate() {
        let mut mapped = [0u32; 3];
        for (slot, &corner) in face.iter().enumerate() {
            mapped[slot] = *indices.get(corner as usize).ok_or_else(|| {
                format!(
                    "object {} in {:?}: face {face_index} references vertex {corner}, \
                     but the mesh has {} vertices",
                    piece.object_id,
                    piece.part,
                    piece.mesh.vertices.len()
                )
            })?;
        }
        let paint = piece.mesh.paint.get(face_index).cloned().ok_or_else(|| {
            format!(
                "object {} in {:?}: face {face_index} has no paint value",
                piece.object_id, piece.part
            )
        })?;
        out.faces.push(mapped);
        out.paint.push(paint);
        out.base_slots.push(base);
    }
    Ok(())
}

/// Reject a mesh the assembler cannot index safely or keep paint parallel to.
fn validate_mesh(mesh: &RawMesh, object_id: u32, part: &str) -> Result<(), String> {
    if mesh.paint.len() != mesh.faces.len() {
        return Err(format!(
            "object {object_id} in {part:?} has {} faces but {} paint values",
            mesh.faces.len(),
            mesh.paint.len()
        ));
    }
    let vertex_count = mesh.vertices.len();
    for (face_index, face) in mesh.faces.iter().enumerate() {
        for &corner in face {
            if corner as usize >= vertex_count {
                return Err(format!(
                    "object {object_id} in {part:?}: face {face_index} references vertex \
                     {corner}, but the mesh has {vertex_count} vertices"
                ));
            }
        }
    }
    Ok(())
}

/// The model part that holds the object a build item names.
///
/// `Scene` keeps no main-part name, but the part is recoverable from the
/// structure: every other part entered `Scene.parts` through a component
/// `p:path`, so the build part is the one no component names. When that is not
/// unique (a hand-built scene, or a component cycle that points back at the
/// main part), the part holding every build object wins; if neither rule picks
/// a single part the id is genuinely ambiguous and assembly reports it instead
/// of guessing geometry.
fn build_part_key<'a>(scene: &'a Scene, object_id: u32) -> Result<&'a str, String> {
    let mut candidates: Vec<&str> = scene
        .parts
        .iter()
        .filter(|(_, objects)| objects.contains_key(&object_id))
        .map(|(part, _)| part.as_str())
        .collect();
    candidates.sort_unstable();
    match candidates.as_slice() {
        [] => Err(format!("no model part holds object {object_id}")),
        [only] => Ok(*only),
        _ => {
            let referenced: HashSet<&str> = scene
                .parts
                .values()
                .flat_map(|objects| objects.values())
                .flat_map(|object| object.components.iter())
                .filter_map(|component| component.path.as_deref())
                .collect();
            if let Some(part) = unique(
                candidates
                    .iter()
                    .copied()
                    .filter(|part| !referenced.contains(part)),
            ) {
                return Ok(part);
            }
            let build_objects: HashSet<u32> =
                scene.build.iter().map(|item| item.object_id).collect();
            if let Some(part) = unique(candidates.iter().copied().filter(|part| {
                scene
                    .parts
                    .get(*part)
                    .is_some_and(|objects| build_objects.iter().all(|id| objects.contains_key(id)))
            })) {
                return Ok(part);
            }
            Err(format!(
                "object {object_id} is ambiguous: model parts {candidates:?} all hold it"
            ))
        }
    }
}

/// The only element of an iterator, or `None` when it yields zero or several.
fn unique<'a>(mut items: impl Iterator<Item = &'a str>) -> Option<&'a str> {
    let first = items.next()?;
    if items.next().is_some() {
        None
    } else {
        Some(first)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::threemf::model::{BuildItem, Component, SceneObject};
    use crate::threemf::plates::ObjectMeta;

    const MAIN: &str = "3D/3dmodel.model";
    const PART: &str = "3D/Objects/part_1.model";

    /// The pre-existing tests exercise geometry only; a default config has no
    /// extruder metadata, so every face's base slot is `None`.
    fn assemble(scene: &Scene, group: &PlateGroup) -> Result<AssembledMesh, String> {
        super::assemble(scene, group, &ModelSettings::default())
    }

    fn settings_with(extruders: &[(u32, u8)]) -> ModelSettings {
        ModelSettings {
            objects: extruders
                .iter()
                .map(|&(id, extruder)| {
                    (
                        id,
                        ObjectMeta {
                            name: None,
                            extruder: Some(extruder),
                        },
                    )
                })
                .collect(),
            plates: vec![],
            present: false,
        }
    }

    fn object(id: u32, mesh: Option<RawMesh>, components: Vec<Component>) -> SceneObject {
        SceneObject {
            id,
            name: None,
            extruder: None,
            mesh,
            components,
        }
    }

    fn component(object_id: u32, path: Option<&str>, transform: Transform) -> Component {
        Component {
            object_id,
            path: path.map(str::to_string),
            transform,
        }
    }

    fn item(object_id: u32, transform: Transform) -> BuildItem {
        BuildItem {
            object_id,
            transform,
        }
    }

    fn group(item_indices: &[usize]) -> PlateGroup {
        PlateGroup {
            plater_id: Some(1),
            name: "plate 1".to_string(),
            item_indices: item_indices.to_vec(),
        }
    }

    fn make_scene(parts: &[(&str, Vec<SceneObject>)], build: Vec<BuildItem>) -> Scene {
        Scene {
            unit: "millimeter".to_string(),
            parts: parts
                .iter()
                .map(|(name, objects)| {
                    let objects = objects.iter().cloned().map(|o| (o.id, o)).collect();
                    (name.to_string(), objects)
                })
                .collect(),
            build,
        }
    }

    fn translated(x: f32, y: f32, z: f32) -> Transform {
        Transform([1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, x, y, z])
    }

    /// A one-face triangle used by the merge tests.
    fn triangle() -> RawMesh {
        RawMesh {
            vertices: vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
            faces: vec![[0, 1, 2]],
            paint: vec![Some("1C".to_string())],
        }
    }

    #[test]
    fn a_single_object_keeps_vertex_and_face_order_untouched() {
        // The second and fourth vertices share a coordinate on purpose: any
        // dedupe would collapse them and reorder the faces, and the geometry
        // fingerprint downstream would no longer match the saved project.
        let mesh = RawMesh {
            vertices: vec![
                [0.0, 0.0, 0.0],
                [1.0, 0.0, 0.0],
                [0.0, 1.0, 0.0],
                [1.0, 0.0, 0.0],
            ],
            faces: vec![[3, 1, 2], [0, 2, 1]],
            paint: vec![Some("1C".to_string()), None],
        };
        let scene = make_scene(
            &[(MAIN, vec![object(7, Some(mesh.clone()), vec![])])],
            vec![item(7, Transform::identity())],
        );

        let assembled = assemble(&scene, &group(&[0])).expect("one object assembles");

        // Vector equality, not just lengths: order and duplicates are the value.
        assert_eq!(assembled.vertices, mesh.vertices);
        assert_eq!(assembled.faces, mesh.faces);
        assert_eq!(assembled.paint, mesh.paint);
    }

    #[test]
    fn a_single_identity_component_still_uses_the_verbatim_fast_path() {
        let leaf = RawMesh {
            vertices: vec![
                [0.0, 0.0, 0.0],
                [1.0, 0.0, 0.0],
                [0.0, 1.0, 0.0],
                [1.0, 0.0, 0.0],
            ],
            faces: vec![[3, 1, 2], [0, 2, 1]],
            paint: vec![None, Some("0C".to_string())],
        };
        let scene = make_scene(
            &[(
                MAIN,
                vec![
                    object(2, None, vec![component(1, None, Transform::identity())]),
                    object(1, Some(leaf.clone()), vec![]),
                ],
            )],
            vec![item(2, Transform::identity())],
        );

        let assembled = assemble(&scene, &group(&[0])).expect("identity component assembles");

        assert_eq!(assembled.vertices, leaf.vertices);
        assert_eq!(assembled.faces, leaf.faces);
        assert_eq!(assembled.paint, leaf.paint);
    }

    #[test]
    fn two_objects_are_baked_then_merged_with_offset_indices() {
        let scene = make_scene(
            &[(
                MAIN,
                vec![
                    object(1, Some(triangle()), vec![]),
                    object(2, Some(triangle()), vec![]),
                ],
            )],
            vec![
                item(1, Transform::identity()),
                item(2, translated(10.0, 0.0, 0.0)),
            ],
        );

        let assembled = assemble(&scene, &group(&[0, 1])).expect("two objects merge");

        assert_eq!(
            assembled.vertices,
            vec![
                [0.0, 0.0, 0.0],
                [1.0, 0.0, 0.0],
                [0.0, 1.0, 0.0],
                [10.0, 0.0, 0.0],
                [11.0, 0.0, 0.0],
                [10.0, 1.0, 0.0],
            ]
        );
        // The second object's face keeps its winding and is offset by three.
        assert_eq!(assembled.faces, vec![[0, 1, 2], [3, 4, 5]]);
        assert_eq!(
            assembled.paint,
            vec![Some("1C".to_string()), Some("1C".to_string())]
        );
    }

    #[test]
    fn duplicate_vertices_across_objects_are_merged() {
        let scene = make_scene(
            &[(
                MAIN,
                vec![
                    object(1, Some(triangle()), vec![]),
                    object(2, Some(triangle()), vec![]),
                ],
            )],
            vec![
                item(1, translated(5.0, 0.0, 0.0)),
                item(2, translated(5.0, 0.0, 0.0)),
            ],
        );

        let assembled = assemble(&scene, &group(&[0, 1])).expect("coincident objects merge");

        assert_eq!(assembled.vertices.len(), 3, "identical coordinates merge");
        assert_eq!(
            assembled.vertices,
            vec![[5.0, 0.0, 0.0], [6.0, 0.0, 0.0], [5.0, 1.0, 0.0]]
        );
        // Both faces index the same three merged vertices.
        assert_eq!(assembled.faces, vec![[0, 1, 2], [0, 1, 2]]);
        assert_eq!(
            assembled.paint,
            vec![Some("1C".to_string()), Some("1C".to_string())]
        );
    }

    #[test]
    fn a_component_inside_an_object_is_flattened_before_the_build_transform() {
        // The component lifts the part by 5; the build item then doubles
        // everything and shifts it by 10. Composition must therefore be
        // vertex · component · build: a vertex at the origin ends at
        // (10, 10, 0), not (10, 5, 0).
        let main = object(
            2,
            None,
            vec![component(1, Some(PART), translated(0.0, 5.0, 0.0))],
        );
        let scene = make_scene(
            &[
                (MAIN, vec![main]),
                (PART, vec![object(1, Some(triangle()), vec![])]),
            ],
            vec![item(
                2,
                Transform([2.0, 0.0, 0.0, 0.0, 2.0, 0.0, 0.0, 0.0, 2.0, 10.0, 0.0, 0.0]),
            )],
        );

        let assembled = assemble(&scene, &group(&[0])).expect("component flattens");

        assert_eq!(
            assembled.vertices,
            vec![[10.0, 10.0, 0.0], [12.0, 10.0, 0.0], [10.0, 12.0, 0.0]]
        );
        assert_eq!(assembled.faces, vec![[0, 1, 2]]);
    }

    #[test]
    fn an_object_with_no_resolvable_geometry_is_an_error() {
        // The build item names an object no model part contains.
        let scene = make_scene(&[(MAIN, vec![])], vec![item(42, Transform::identity())]);
        let err = assemble(&scene, &group(&[0])).expect_err("missing object is not an empty mesh");
        assert!(err.contains("42"), "error must name the object: {err}");

        // The object exists but has neither a mesh nor components.
        let scene = make_scene(
            &[(MAIN, vec![object(3, None, vec![])])],
            vec![item(3, Transform::identity())],
        );
        let err = assemble(&scene, &group(&[0])).expect_err("an empty object is an error");
        assert!(err.contains('3'), "error must name the object: {err}");
    }

    #[test]
    fn a_component_that_names_missing_geometry_is_an_error() {
        // A component path that is not a `Scene.parts` key.
        let scene = make_scene(
            &[(
                MAIN,
                vec![object(
                    2,
                    None,
                    vec![component(
                        1,
                        Some("3D/Objects/gone.model"),
                        Transform::identity(),
                    )],
                )],
            )],
            vec![item(2, Transform::identity())],
        );
        let err = assemble(&scene, &group(&[0])).expect_err("missing part is not degradable");
        assert!(
            err.contains("gone.model"),
            "error must name the part: {err}"
        );

        // A component id that the named part does not hold.
        let scene = make_scene(
            &[(
                MAIN,
                vec![object(
                    2,
                    None,
                    vec![component(9, None, Transform::identity())],
                )],
            )],
            vec![item(2, Transform::identity())],
        );
        let err = assemble(&scene, &group(&[0])).expect_err("missing component object is an error");
        assert!(
            err.contains('9'),
            "error must name the component object: {err}"
        );
    }

    #[test]
    fn nesting_deeper_than_the_limit_is_an_error() {
        // A 41-object chain; assembly may only follow 32 component levels.
        let objects: Vec<SceneObject> = (0..41)
            .map(|id| {
                let components = if id < 40 {
                    vec![component(id + 1, None, Transform::identity())]
                } else {
                    vec![]
                };
                let mesh = if id == 40 { Some(triangle()) } else { None };
                object(id, mesh, components)
            })
            .collect();
        let scene = make_scene(&[(MAIN, objects)], vec![item(0, Transform::identity())]);

        let err = assemble(&scene, &group(&[0])).expect_err("excessive nesting is an error");

        assert!(err.contains("32"), "error must name the depth limit: {err}");
    }

    #[test]
    fn a_face_index_outside_the_mesh_is_an_error() {
        let bad = RawMesh {
            vertices: vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
            faces: vec![[0, 1, 3]],
            paint: vec![None],
        };

        // The verbatim fast path validates too.
        let scene = make_scene(
            &[(MAIN, vec![object(7, Some(bad.clone()), vec![])])],
            vec![item(7, Transform::identity())],
        );
        let err = assemble(&scene, &group(&[0])).expect_err("bad index must not be indexed");
        assert!(err.contains("vertex 3"), "error must name the index: {err}");

        // And so does the baked path.
        let scene = make_scene(
            &[(
                MAIN,
                vec![
                    object(7, Some(bad.clone()), vec![]),
                    object(8, Some(triangle()), vec![]),
                ],
            )],
            vec![
                item(7, translated(1.0, 0.0, 0.0)),
                item(8, Transform::identity()),
            ],
        );
        let err = assemble(&scene, &group(&[0, 1])).expect_err("bad index must not be indexed");
        assert!(err.contains("vertex 3"), "error must name the index: {err}");
    }

    #[test]
    fn a_build_item_resolves_in_the_main_part_not_a_same_id_leaf() {
        // The wrapper in the main part and the leaf in the external part are
        // both object 1. The build item must pick the wrapper: only then does
        // the component's +10 in y apply.
        let wrapper = object(
            1,
            None,
            vec![component(1, Some(PART), translated(0.0, 10.0, 0.0))],
        );
        let scene = make_scene(
            &[
                (MAIN, vec![wrapper]),
                (PART, vec![object(1, Some(triangle()), vec![])]),
            ],
            vec![item(1, translated(100.0, 0.0, 0.0))],
        );

        let assembled = assemble(&scene, &group(&[0])).expect("wrapper resolves");

        assert_eq!(
            assembled.vertices,
            vec![[100.0, 10.0, 0.0], [101.0, 10.0, 0.0], [100.0, 11.0, 0.0]]
        );
    }

    #[test]
    fn an_object_id_that_two_unclaimed_parts_both_hold_is_ambiguous() {
        // The parser cannot build this scene: a second part is always reached
        // through a component path. A hand-built scene must still not have its
        // geometry guessed.
        let scene = make_scene(
            &[
                (MAIN, vec![object(1, Some(triangle()), vec![])]),
                (PART, vec![object(1, Some(triangle()), vec![])]),
            ],
            vec![item(1, Transform::identity())],
        );

        let err = assemble(&scene, &group(&[0])).expect_err("ambiguous object id is an error");

        assert!(err.contains("ambiguous"), "error must say why: {err}");
    }

    #[test]
    fn each_faces_base_slot_comes_from_its_build_object() {
        // Two objects on one plate, deliberately on different filaments. The
        // merged fast path must not reorder or dedupe, but each face has to
        // carry the extruder of the object it came from.
        let scene = make_scene(
            &[(
                MAIN,
                vec![
                    object(1, Some(triangle()), vec![]),
                    object(2, Some(triangle()), vec![]),
                ],
            )],
            vec![
                item(1, Transform::identity()),
                item(2, translated(10.0, 0.0, 0.0)),
            ],
        );
        let settings = settings_with(&[(1, 0), (2, 2), (9, 1)]);

        let assembled =
            super::assemble(&scene, &group(&[0, 1]), &settings).expect("two objects merge");

        assert_eq!(assembled.faces, vec![[0, 1, 2], [3, 4, 5]]);
        assert_eq!(
            assembled.base_slots,
            vec![Some(0), Some(2)],
            "each face inherits its own object's 0-based extruder"
        );

        // The verbatim single-object path carries the base too.
        let single = make_scene(
            &[(MAIN, vec![object(1, Some(triangle()), vec![])])],
            vec![item(1, Transform::identity())],
        );
        let assembled =
            super::assemble(&single, &group(&[0]), &settings).expect("one object is verbatim");
        assert_eq!(assembled.base_slots, vec![Some(0)]);

        // An object with no settings entry has no base; the reader falls back
        // to slot 1 rather than inheriting another object's extruder.
        let missing = make_scene(
            &[(MAIN, vec![object(7, Some(triangle()), vec![])])],
            vec![item(7, Transform::identity())],
        );
        let assembled =
            super::assemble(&missing, &group(&[0]), &settings).expect("object without settings");
        assert_eq!(assembled.base_slots, vec![None]);
    }

    #[test]
    fn a_group_with_no_build_items_is_an_empty_mesh() {
        let scene = make_scene(&[(MAIN, vec![])], vec![]);

        let assembled = assemble(&scene, &group(&[])).expect("an empty plate assembles");

        assert_eq!(assembled, AssembledMesh::default());
    }

    #[test]
    fn a_build_index_outside_the_build_is_an_error() {
        let scene = make_scene(&[(MAIN, vec![])], vec![]);

        let err = assemble(&scene, &group(&[3])).expect_err("out-of-range build index is an error");

        assert!(err.contains('3'), "error must name the build index: {err}");
    }
}
