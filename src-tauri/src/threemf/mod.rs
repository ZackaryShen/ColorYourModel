//! Read side of the 3MF format: OPC package access, model XML, plate grouping,
//! palette resolution, `paint_color` decoding, triangle subdivision rebuild and
//! the import orchestration that turns all of it into one [`MeshModel`] per
//! plate.
//!
//! The orchestration is deliberately forgiving about everything optional:
//! engineering state (`Metadata/cym/*`), `model_settings.config` and
//! `project_settings.config` each degrade independently to "not there" with a
//! warning, because a geometrically valid 3MF must still open. Only failures
//! that leave no usable geometry — a bad package, unparsable model XML, a
//! missing referenced part — are errors.

pub mod assemble;
pub mod legacy_paint;
mod materials;
pub mod model;
pub mod opc;
pub mod paint;
pub mod plates;
mod refine;
mod source_materials;
pub mod subdivide;

#[cfg(test)]
pub mod test_support;

use std::collections::HashMap;
use std::path::Path;

use self::materials::ProjectMaterials;
use self::model::Scene;
use self::opc::Package;
use self::paint::PaintNode;
use self::plates::{ModelSettings, PlateGroup};
use crate::export::project_state;
use crate::mesh::model::{MeshModel, DEFAULT_FACE_COLOR, MANUAL_SEGMENT_OFFSET};

pub(crate) use source_materials::source_materials_entry;

/// Physical slot count and count of active native mixed recipes, in file order.
pub(crate) fn native_material_slot_count(
    config: &serde_json::Value,
) -> Result<Option<(usize, usize)>, String> {
    if config["mixed_filament_definitions"]
        .as_str()
        .is_none_or(str::is_empty)
    {
        return Ok(None);
    }
    let parsed = materials::project_materials_from_config(config)?;
    Ok(Some((parsed.physical.0.len(), parsed.mixed.len())))
}

/// Package part holding `model_settings.config`.
const MODEL_SETTINGS_PART: &str = "Metadata/model_settings.config";
/// Package part holding the slicer's project settings (palette, bed size).
const PROJECT_SETTINGS_PART: &str = "Metadata/project_settings.config";
/// Directory the writer owns for engineering state.
const ENGINEERING_DIR: &str = "Metadata/cym";
/// Engineering header: format version, unit, and one fingerprint per plate.
const ENGINEERING_HEADER_PART: &str = "Metadata/cym/project.json";

// Bound one editable plate's reconstructed geometry, including paint splits.
const MAX_EDITOR_FACES: usize = 1_500_000;

#[derive(Clone, Copy)]
struct GeometryBudget {
    vertices: usize,
    faces: usize,
}

impl GeometryBudget {
    const fn new(faces: usize) -> Self {
        Self {
            vertices: faces * 3,
            faces,
        }
    }

    fn check(&self, vertices: usize, faces: usize) -> Result<(), String> {
        if vertices > self.vertices || faces > self.faces {
            return Err("3MF exceeds the cumulative vertex or triangle budget".into());
        }
        Ok(())
    }

    fn claim(&mut self, vertices: usize, faces: usize) -> Result<(), String> {
        self.check(vertices, faces)?;
        self.vertices -= vertices;
        self.faces -= faces;
        Ok(())
    }
}

/// What happened to the optional engineering state (`Metadata/cym/*`) of a
/// plate.
///
/// A `bool` cannot separate "the file never carried engineering state" from
/// "it carried some and this build discarded it": the first is a normal plain
/// import with no notice, the second must tell the user the saved state was
/// dropped. Keeping the three outcomes explicit is what lets the command layer
/// and the status bar distinguish them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EngineeringState {
    /// The saved engineering state matched the geometry and was applied.
    Restored,
    /// The file carried no engineering parts at all.
    Absent,
    /// Engineering parts existed but could not be applied (version, parse,
    /// fingerprint, decode, or a missing plate part).
    Dropped,
}

impl EngineeringState {
    /// Stable wire name for the frontend DTO.
    pub fn as_str(self) -> &'static str {
        match self {
            EngineeringState::Restored => "restored",
            EngineeringState::Absent => "absent",
            EngineeringState::Dropped => "dropped",
        }
    }
}

/// One plate ready to become a tab.
pub struct ImportedPlate {
    pub plater_id: u32,
    pub name: String,
    /// Geometry is in millimetres and **plate-local** coordinates: the plate's
    /// world-grid origin has already been subtracted, so plate 1 and plate 7
    /// both start near the origin. `engineering_state` says whether the exact
    /// saved state (labels, RGBA, names, history, high-water mark) survived.
    pub mesh: MeshModel,
    pub engineering_state: EngineeringState,
    /// True when a `history_<i>.bin` part existed but could not be restored
    /// (unknown op kind, corrupt payload, or an unreadable part). A missing
    /// history part is not a loss: the file never carried one.
    pub history_dropped: bool,
}

/// Everything one 3MF contributes: the declared unit and one plate per
/// `model_settings.config` plate (or a single plate when the file has none).
pub struct ImportedProject {
    /// The unit the file declared. Coordinates are always millimetres; this is
    /// kept because the project header records it at write time.
    pub unit: String,
    pub plates: Vec<ImportedPlate>,
}

/// Import every plate of a 3MF.
///
/// An error means no usable geometry could be produced (bad package, malformed
/// model XML, a referenced part/object that does not resolve). Engineering
/// state, `model_settings.config` and `project_settings.config` are optional:
/// each failure is logged and the plate falls back to plain geometry + colours.
#[derive(Clone)]
pub struct PlateRef {
    pub face_count: u32,
    pub plater_id: u32,
    pub name: String,
}

pub struct ProjectIndex {
    pub plates: Vec<PlateRef>,
    pub active_index: usize,
}

/// Read scene metadata without constructing any plate's editable mesh.
pub fn load_plates(path: &Path) -> Result<ProjectIndex, String> {
    ProjectReader::open(path, &|_, _| {})?.index()
}

pub fn load_plate(
    path: &Path,
    index: usize,
    progress: &dyn Fn(f32, &str),
) -> Result<ImportedPlate, String> {
    let mut reader = ProjectReader::open(path, progress)?;
    let plate = reader.load(index, progress)?;
    progress(1.0, "3mf:done");
    Ok(plate)
}

pub fn open_active_plate(
    path: &Path,
    progress: &dyn Fn(f32, &str),
) -> Result<(ProjectIndex, ImportedPlate, Option<ProjectLayout>), String> {
    let stamp = ProjectLayout::stamp(path);
    let mut reader = ProjectReader::open(path, progress)?;
    let index = reader.index()?;
    let active = reader.load(index.active_index, progress)?;
    let layout = stamp.map(|stamp| ProjectLayout {
        path: path.to_path_buf(),
        stamp,
        groups: reader.groups,
    });
    progress(1.0, "3mf:done");
    Ok((index, active, layout))
}

/// Retain plate membership, but no mesh data, for on-demand source loads.
pub struct ProjectLayout {
    path: std::path::PathBuf,
    stamp: (std::time::SystemTime, u64),
    groups: Vec<PlateGroup>,
}

impl ProjectLayout {
    fn stamp(path: &Path) -> Option<(std::time::SystemTime, u64)> {
        let metadata = path.metadata().ok()?;
        Some((metadata.modified().ok()?, metadata.len()))
    }

    pub fn applies_to(&self, path: &Path) -> bool {
        self.path == path && Self::stamp(path) == Some(self.stamp)
    }

    pub fn load(
        &self,
        path: &Path,
        index: usize,
        progress: &dyn Fn(f32, &str),
    ) -> Result<ImportedPlate, String> {
        let mut group = self
            .groups
            .get(index)
            .ok_or("Requested plate is outside the project")?
            .clone();
        progress(0.02, "3mf:open");
        let mut package = Package::open(path)?;
        progress(0.10, "3mf:model");
        let scene = model::read_selected_scene(&mut package, &group.item_indices)?;
        group.item_indices = (0..scene.build.len()).collect();
        let settings = read_settings(&mut package);
        let config = read_optional(&mut package, PROJECT_SETTINGS_PART);
        let palette = config.as_deref().and_then(read_palette);
        let (bed_width, bed_depth) = config.as_deref().map(bed_size).unwrap_or_default();
        let context = PlateContext {
            scene: &scene,
            settings: &settings,
            palette: palette.as_ref(),
            bed_width,
            bed_depth,
            plate_count: self.groups.len(),
        };
        let header_present = part_exists(&package, ENGINEERING_HEADER_PART);
        let header = read_engineering_header(&mut package);
        progress(
            0.30,
            &format!("3mf:plate {}/{}", index + 1, self.groups.len()),
        );
        let plate = import_plate(
            &mut package,
            &context,
            header.as_ref(),
            header_present,
            index,
            &group,
        )?;
        progress(1.0, "3mf:done");
        Ok(plate)
    }
}

pub fn read_project(path: &Path, progress: &dyn Fn(f32, &str)) -> Result<ImportedProject, String> {
    let mut reader = ProjectReader::open(path, progress)?;
    let plates = (0..reader.groups.len())
        .map(|index| reader.load(index, progress))
        .collect::<Result<Vec<_>, _>>()?;
    progress(1.0, "3mf:done");
    Ok(ImportedProject {
        unit: reader.scene.unit,
        plates,
    })
}

struct ProjectReader {
    package: Package,
    scene: Scene,
    settings: ModelSettings,
    groups: Vec<PlateGroup>,
    palette: Option<ProjectMaterials>,
    bed_width: f32,
    bed_depth: f32,
    header: Option<project_state::ProjectState>,
    header_present: bool,
}

impl ProjectReader {
    fn open(path: &Path, progress: &dyn Fn(f32, &str)) -> Result<Self, String> {
        progress(0.02, "3mf:open");
        let mut package = Package::open(path)?;
        progress(0.10, "3mf:model");
        let scene = model::read_scene(&mut package)?;
        progress(0.25, "3mf:plates");
        let settings = read_settings(&mut package);
        let config = read_optional(&mut package, PROJECT_SETTINGS_PART);
        let (bed_width, bed_depth) = config.as_deref().map(bed_size).unwrap_or_default();
        let palette = config.as_deref().and_then(read_palette);
        let groups = group_scene(&scene, &settings, bed_width, bed_depth)?;
        let header_present = part_exists(&package, ENGINEERING_HEADER_PART);
        let header = read_engineering_header(&mut package);
        Ok(Self {
            package,
            scene,
            settings,
            groups,
            palette,
            bed_width,
            bed_depth,
            header,
            header_present,
        })
    }

    fn index(&self) -> Result<ProjectIndex, String> {
        let mut assembly = assemble::AssemblyBudget::new(MAX_EDITOR_FACES * 2);
        let plates: Vec<_> = self
            .groups
            .iter()
            .enumerate()
            .map(|(index, group)| {
                let plater_id = group.plater_id.unwrap_or(index as u32 + 1);
                let name = self
                    .header
                    .as_ref()
                    .and_then(|h| h.plates.iter().find(|p| p.plater_id == plater_id))
                    .map(|p| p.name.clone())
                    .filter(|n| !n.trim().is_empty())
                    .unwrap_or_else(|| plate_name(&self.settings, &self.scene, group, plater_id));
                let face_count = group.item_indices.iter().try_fold(0u32, |total, &item| {
                    total
                        .checked_add(assemble::item_face_count(&self.scene, item, &mut assembly)?)
                        .filter(|&count| count as usize <= MAX_EDITOR_FACES)
                        .ok_or_else(|| "Plate exceeds supported face count".to_string())
                })?;
                Ok(PlateRef {
                    plater_id,
                    name,
                    face_count,
                })
            })
            .collect::<Result<_, String>>()?;
        let active_index = self
            .header
            .as_ref()
            .and_then(|h| plates.iter().position(|p| p.plater_id == h.active_plate))
            .unwrap_or(0);
        Ok(ProjectIndex {
            plates,
            active_index,
        })
    }

    fn load(
        &mut self,
        index: usize,
        progress: &dyn Fn(f32, &str),
    ) -> Result<ImportedPlate, String> {
        let total = self.groups.len();
        let group = self
            .groups
            .get(index)
            .ok_or("Requested plate is outside the project")?;
        progress(0.30, &format!("3mf:plate {}/{}", index + 1, total));
        let context = PlateContext {
            scene: &self.scene,
            settings: &self.settings,
            palette: self.palette.as_ref(),
            bed_width: self.bed_width,
            bed_depth: self.bed_depth,
            plate_count: total,
        };
        import_plate(
            &mut self.package,
            &context,
            self.header.as_ref(),
            self.header_present,
            index,
            group,
        )
    }
}

/// Printed world positions win over stale slicer instance metadata.
fn group_scene(
    scene: &Scene,
    settings: &ModelSettings,
    width: f32,
    depth: f32,
) -> Result<Vec<PlateGroup>, String> {
    let mut groups = plates::group(&scene.build, settings);
    if groups.len() < 2 || width <= 0.0 || depth <= 0.0 {
        return Ok(groups);
    }
    let hints = groups.clone();
    for group in &mut groups {
        group.item_indices.clear();
    }
    let mut assembly = assemble::AssemblyBudget::new(MAX_EDITOR_FACES * 2);
    for item in 0..scene.build.len() {
        let hint = hints
            .iter()
            .position(|g| g.item_indices.contains(&item))
            .unwrap_or(0);
        let target = if let Some((lo, hi)) = assemble::item_bounds(scene, item, &mut assembly)? {
            let center = [(lo[0] + hi[0]) * 0.5, (lo[1] + hi[1]) * 0.5];
            groups
                .iter()
                .enumerate()
                .position(|(i, g)| {
                    let origin = plate_origin(
                        g.plater_id.unwrap_or(i as u32 + 1),
                        groups.len(),
                        width,
                        depth,
                    );
                    center[0] >= origin[0]
                        && center[0] <= origin[0] + width
                        && center[1] >= origin[1]
                        && center[1] <= origin[1] + depth
                })
                .unwrap_or(hint)
        } else {
            hint
        };
        if target != hint {
            log::warn!("3mf: build item {item} plate membership contradicts printed position; using plate {}",target+1);
        }
        groups[target].item_indices.push(item);
    }
    Ok(groups)
}

/// Everything about the file that is shared by all of its plates.
struct PlateContext<'a> {
    scene: &'a Scene,
    settings: &'a ModelSettings,
    palette: Option<&'a ProjectMaterials>,
    bed_width: f32,
    bed_depth: f32,
    plate_count: usize,
}

/// Assemble, subdivide, normalise, colour and (when it matches) restore one
/// plate.
fn import_plate(
    package: &mut Package,
    context: &PlateContext,
    engineering_header: Option<&project_state::ProjectState>,
    engineering_header_present: bool,
    index: usize,
    group: &PlateGroup,
) -> Result<ImportedPlate, String> {
    let plater_id = group.plater_id.unwrap_or(index as u32 + 1);

    let assembled = assemble::assemble(context.scene, group, context.settings)
        .map_err(|error| format!("plate {plater_id}: {error}"))?;
    let budget = GeometryBudget::new(MAX_EDITOR_FACES);
    let (trees, face_count) = decode_paint(&assembled.paint, budget.faces)?;
    // Each split adds at most one vertex per additional triangle.
    let vertex_bound = assembled.vertices.len() + (face_count - assembled.faces.len());
    budget.check(vertex_bound, face_count)?;
    let mut expanded = subdivide::expand(&assembled.vertices, &assembled.faces, &trees);
    expanded.slots = resolve_face_slots(&expanded, &trees, &assembled.base_slots, context.palette);
    // Mixed paint trees can leave full-height unpainted neighbors between
    // finely subdivided patches. Refine before assigning any editor face IDs.
    if !engineering_header_present
        && trees.iter().flatten().any(|tree| tree.split > 0)
        && context.palette.is_some_and(|palette| {
            expanded.slots.iter().any(|&slot| usize::from(slot) > palette.physical.0.len())
        })
    {
        expanded = refine::for_paint(expanded, budget);
    }

    // The fingerprint domain is the world coordinates **as written into the
    // file**, i.e. before the plate-origin shift below. The writer hashes the
    // vertices it emits (also world space); hashing the plate-local form here
    // would break on a non-zero origin because `fl(fl(world - origin) + origin)`
    // is not `world` at 264 mm (f32 ulp there dwarfs the `{:.6}` grid).
    let fingerprint = engineering_header
        .is_some()
        .then(|| project_state::fingerprint_of(&expanded.vertices, &expanded.faces));

    // Plate-local coordinates for the tab: the writer re-adds this origin.
    let origin = plate_origin(
        plater_id,
        context.plate_count,
        context.bed_width,
        context.bed_depth,
    );
    let vertices = expanded
        .vertices
        .iter()
        .map(|vertex| {
            [
                vertex[0] - origin[0],
                vertex[1] - origin[1],
                vertex[2] - origin[2],
            ]
        })
        .collect();

    let face_colors = expanded
        .slots
        .iter()
        .map(|&slot| {
            context
                .palette
                .and_then(|palette| palette.colour(slot))
                .unwrap_or(DEFAULT_FACE_COLOR)
        })
        .collect();

    let mut mesh = MeshModel::new();
    mesh.vertices = vertices;
    mesh.faces = expanded.faces;
    mesh.face_colors = face_colors;
    if let Some(palette) = context.palette.filter(|p| !p.mixed.is_empty()) {
        mesh.source_material_colors = palette.source_colours();
        mesh.source_material_slots = expanded.slots;
    }
    let face_count = mesh.faces.len();
    mesh.segment_labels = vec![0; face_count];
    mesh.unit = "millimeter".to_string();

    let restore = try_restore(
        package,
        engineering_header,
        engineering_header_present,
        index,
        plater_id,
        fingerprint,
        &mut mesh,
    );

    let texture_ordinal = engineering_header
        .and_then(|header| {
            header
                .plates
                .iter()
                .position(|plate| plate.plater_id == plater_id)
        })
        .unwrap_or(index);
    let texture_fingerprint = fingerprint.filter(|_| restore.state == EngineeringState::Restored);
    let rebuild_segments = match crate::engineering::restore_texture_state(
        package,
        texture_ordinal,
        texture_fingerprint,
        &mut mesh,
    ) {
        Ok(rebuild) => rebuild,
        Err(error) => {
            mesh.texture_state_dropped = true;
            log::warn!("3mf: texture settings for plate {plater_id} could not restore ({error})");
            true
        }
    };

    if restore.state == EngineeringState::Restored {
        if let Some(saved) = engineering_header.and_then(|header| {
            header
                .plates
                .iter()
                .find(|plate| plate.plater_id == plater_id)
        }) {
            for vertex in &mut mesh.vertices {
                for axis in 0..3 {
                    vertex[axis] -= saved.placement_offset[axis];
                }
            }
        }
    }

    // Import each native mixed slot as its own region, even when different
    // recipes produce the same preview swatch. Ordinary palettes still group
    // by colour and restored engineering projects retain their edited labels.
    if restore.state != EngineeringState::Restored {
        mesh.segment_labels = if mesh.source_material_slots.is_empty() {
            labels_from_values(&mesh.face_colors)
        } else {
            labels_from_values(&mesh.source_material_slots)
        };
    }

    // Derived state, in the same order every other loader uses.
    mesh.compute_normals();
    mesh.compute_bbox();
    if rebuild_segments {
        mesh.rebuild_segments();
    }
    mesh.build_kdtree();
    mesh.build_vertex_kdtree();
    mesh.build_adjacency();

    Ok(ImportedPlate {
        plater_id,
        name: engineering_header
            .and_then(|header| header.plates.iter().find(|p| p.plater_id == plater_id))
            .map(|p| p.name.clone())
            .filter(|n| !n.trim().is_empty())
            .unwrap_or_else(|| plate_name(context.settings, context.scene, group, plater_id)),
        mesh,
        engineering_state: restore.state,
        history_dropped: restore.history_dropped,
    })
}

/// Region labels start with the largest region. Equal counts sort by slot or
/// swatch for a stable order across imports.
fn labels_from_values<T: Copy + Ord + std::hash::Hash>(values: &[T]) -> Vec<u32> {
    let mut counts: HashMap<T, u32> = HashMap::new();
    for &value in values {
        *counts.entry(value).or_insert(0) += 1;
    }
    let mut ordered: Vec<_> = counts.into_iter().collect();
    ordered.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    let label_of: HashMap<_, _> = ordered
        .iter()
        .enumerate()
        .map(|(index, (value, _))| (*value, index as u32))
        .collect();
    values.iter().map(|value| label_of[value]).collect()
}

/// Decode every `paint_color` tree, degrading one malformed value to "no
/// paint" instead of failing the import.
fn decode_paint(
    paint: &[Option<String>],
    max_faces: usize,
) -> Result<(Vec<Option<PaintNode>>, usize), String> {
    let mut trees = Vec::with_capacity(paint.len());
    let mut faces = 0;
    for (index, value) in paint.iter().enumerate() {
        let tree = if let Some(hex) = value.as_deref() {
            match paint::decode(hex) {
                Ok(node) => Some(node),
                Err(error) => {
                    log::warn!("3mf: face {index} has an undecodable paint_color; rendering it unpainted: {error}");
                    None
                }
            }
        } else {
            None
        };
        faces += tree.as_ref().map_or(1, paint::leaf_count);
        if faces > max_faces {
            return Err("3MF paint exceeds the expanded triangle budget".into());
        }
        trees.push(tree);
    }
    Ok((trees, faces))
}

/// Assign expanded faces to native slots. Unpainted leaves take their own
/// object's base slot (or 1) and paint leaves address the full slot namespace.
fn resolve_face_slots(
    expanded: &subdivide::Expanded,
    trees: &[Option<PaintNode>],
    base_slots: &[Option<u8>],
    palette: Option<&ProjectMaterials>,
) -> Vec<u8> {
    let Some(palette) = palette else {
        return vec![0; expanded.faces.len()];
    };
    let last_physical_slot = u8::try_from(palette.physical.0.len()).unwrap_or(u8::MAX);
    let effective = |slot, base: Option<u8>| match slot {
        0 => base.filter(|&base| palette.colour(base).is_some()).unwrap_or(1),
        slot if palette.colour(slot).is_some() => slot,
        // Keep the legacy fallback for incomplete ordinary palettes. With a
        // mixed palette a missing recipe must not borrow a real slot ID.
        _ if palette.mixed.is_empty() => last_physical_slot,
        _ => 0,
    };

    // Each base contributes one contiguous run of leaves in the expanded mesh.
    let bases = trees.iter().zip(base_slots).flat_map(|(tree, base)| {
        std::iter::repeat_n(*base, tree.as_ref().map_or(1, paint::leaf_count))
    });
    let mut slots: Vec<_> = expanded
        .slots
        .iter()
        .zip(bases)
        .map(|(&slot, base)| effective(slot, base))
        .collect();
    if slots.len() != expanded.faces.len() {
        // A malformed hand-built mesh must not borrow a neighbour's base.
        slots = (0..expanded.faces.len())
            .map(|index| effective(expanded.slots.get(index).copied().unwrap_or(0), None))
            .collect();
    }
    slots
}

/// The plate's display name: the config's plate name, then the object name,
/// then the 1-based ordinal.
fn plate_name(
    settings: &ModelSettings,
    scene: &Scene,
    group: &PlateGroup,
    plater_id: u32,
) -> String {
    if !group.name.trim().is_empty() {
        return group.name.clone();
    }
    let object_name = group
        .item_indices
        .first()
        .and_then(|&item| scene.build.get(item))
        .and_then(|item| settings.objects.get(&item.object_id))
        .and_then(|meta| meta.name.as_deref())
        .filter(|name| !name.trim().is_empty());
    match object_name {
        Some(name) => name.to_string(),
        None => format!("plate {plater_id}"),
    }
}

/// The world origin of a plate's cell in the slicer's grid.
///
/// Cells are laid out row-major on a grid wide enough for `plate_count`, with
/// 20% spacing between beds. A single-plate file — and any file without a
/// usable `printable_area` — has the identity origin.
pub(crate) fn plate_origin(
    plater_id: u32,
    plate_count: usize,
    bed_width: f32,
    bed_depth: f32,
) -> [f32; 3] {
    if plate_count <= 1 || (bed_width <= 0.0 && bed_depth <= 0.0) {
        return [0.0; 3];
    }
    let count = plate_count.max(1);
    // Smallest column count whose square reaches the plate count.
    let mut columns = 1usize;
    while columns
        .checked_mul(columns)
        .is_some_and(|square| square < count)
    {
        columns += 1;
    }
    let index = plater_id.saturating_sub(1) as usize;
    let row = index / columns;
    let column = index % columns;
    [
        column as f32 * bed_width * 1.2,
        -(row as f32) * bed_depth * 1.2,
        0.0,
    ]
}

/// Parse `Metadata/cym/project.json` once for the whole file.
///
/// `None` covers both "file has no engineering state" and every parse failure
/// (unparsable, incompatible version, no plates). The caller tracks whether the
/// part *existed* separately, so a rejected part is reported as
/// [`EngineeringState::Dropped`] rather than as a plain file.
fn read_engineering_header(package: &mut Package) -> Option<project_state::ProjectState> {
    let bytes = read_optional(package, ENGINEERING_HEADER_PART)?;
    let Ok(text) = std::str::from_utf8(&bytes) else {
        log::warn!("3mf: engineering state ignored (project.json is not UTF-8)");
        return None;
    };
    let project = match project_state::decode_project_json(text) {
        Ok(project) => project,
        Err(error) => {
            log::warn!("3mf: engineering state ignored ({error})");
            return None;
        }
    };
    if project.plates.is_empty() {
        log::warn!("3mf: engineering state ignored (project.json lists no plates)");
        return None;
    }
    Some(project)
}

/// What restoring one plate's engineering state produced.
struct RestoreOutcome {
    state: EngineeringState,
    history_dropped: bool,
}

impl RestoreOutcome {
    fn absent() -> Self {
        Self {
            state: EngineeringState::Absent,
            history_dropped: false,
        }
    }

    fn dropped(history_dropped: bool) -> Self {
        Self {
            state: EngineeringState::Dropped,
            history_dropped,
        }
    }
}

/// Restore exact engineering state when it belongs to this geometry.
///
/// Never an error: every way the state can fail to apply degrades to plain
/// import. The outcome distinguishes "the file carried no engineering parts"
/// (`Absent`, no notice) from "parts existed but were rejected" (`Dropped`,
/// which the UI must surface), and reports a present-but-unreadable history
/// separately.
fn try_restore(
    package: &mut Package,
    project: Option<&project_state::ProjectState>,
    header_part_present: bool,
    plate_index: usize,
    plater_id: u32,
    fingerprint: Option<project_state::Fingerprint>,
    mesh: &mut MeshModel,
) -> RestoreOutcome {
    let Some(project) = project else {
        // The header was absent or unreadable. Only "something existed" is a
        // drop worth telling the user about; a file with no `Metadata/cym/*`
        // part at all is a normal plain import.
        let plate_present = part_exists(package, &plate_part_name(plate_index));
        let history_present = part_exists(package, &history_part_name(plate_index));
        if header_part_present || plate_present || history_present {
            log::warn!("3mf: engineering state exists but could not be read; using plain import");
            return RestoreOutcome::dropped(history_present);
        }
        return RestoreOutcome::absent();
    };
    let Some((header_index, header)) = project
        .plates
        .iter()
        .enumerate()
        .find(|(_, header)| header.plater_id == plater_id)
    else {
        // `read_engineering_header` rejects an empty plate list, so this is
        // only reachable for a hand-built `ProjectState`.
        return RestoreOutcome::dropped(part_exists(package, &history_part_name(plate_index)));
    };
    let plate_part = plate_part_name(header_index);
    let history_part = history_part_name(header_index);

    // The caller hashes the subdivided, millimetre geometry **before** the
    // plate-origin shift: that is the world-coordinate domain the writer
    // hashes, and the only domain that survives a non-zero origin.
    if fingerprint != Some(header.fingerprint) {
        log::warn!("3mf: engineering state belongs to different geometry; using plain import",);
        return RestoreOutcome::dropped(part_exists(package, &history_part));
    }

    // Parts are numbered by the plate's 1-based ordinal (the header is located
    // by index, so an out-of-range plate falls back to the first part too).
    let Some(state_bytes) = read_optional(package, &plate_part) else {
        log::warn!("3mf: engineering header matched, but {plate_part} is missing");
        return RestoreOutcome::dropped(part_exists(package, &history_part));
    };
    let state = match project_state::decode_plate_state(&state_bytes, mesh.faces.len()) {
        Ok(state) => state,
        Err(error) => {
            log::warn!("3mf: {plate_part} ignored ({error})");
            return RestoreOutcome::dropped(part_exists(package, &history_part));
        }
    };
    mesh.face_colors = state.face_colors;
    mesh.segment_labels = state.segment_labels;
    mesh.segment_names = state.segment_names;
    mesh.next_manual_label = state.next_manual_label;

    // In a CYM engineering file, public paint slots encode the current
    // print appearance. The original assignments belong to this sidecar.
    mesh.source_material_slots.clear();
    mesh.source_material_colors.clear();
    let source_part = source_materials::part_name(header_index);
    if let Some(bytes) = read_optional(package, &source_part) {
        if let Err(error) = source_materials::restore(&bytes, mesh, header.fingerprint) {
            log::warn!("3mf: {source_part} ignored ({error})");
        }
    }

    let gradient_part = format!("{ENGINEERING_DIR}/gradient_faces_{}.bin", header_index + 1);
    if let Some(bytes) = read_optional(package, &gradient_part) {
        match project_state::decode_gradient_faces(&bytes, &mesh.face_colors) {
            Ok((gradients, ramps)) => {
                mesh.gradient_faces = gradients;
                mesh.gradient_ramps = ramps;
            }
            Err(error) => log::warn!("3mf: {gradient_part} ignored ({error})"),
        }
    }

    let paint_sources_part = format!("{ENGINEERING_DIR}/paint_sources_{}.bin", header_index + 1);
    if let Some(bytes) = read_optional(package, &paint_sources_part) {
        match project_state::decode_paint_sources(&bytes, mesh.faces.len()) {
            Ok(sources) => mesh.paint_sources = sources,
            Err(error) => log::warn!("3mf: {paint_sources_part} ignored ({error})"),
        }
    }

    // History is optional on its own: a missing part means "open with an empty
    // stack", never "reject the file". A part that exists but cannot decode is
    // a real loss, so the DTO reports it instead of silently emptying the
    // stack.
    let mut history_dropped = header.history_truncated;
    if let Some(history_bytes) = read_optional(package, &history_part) {
        match project_state::decode_history_with_ramps(&history_bytes, mesh.faces.len(), &mesh.gradient_ramps) {
            Ok(Some(mut history)) => {
                history.identify_legacy_gradients(&mesh.gradient_faces);
                mesh.history = history;
            }
            Ok(None) => {
                log::warn!("3mf: persisted history is unreadable; keeping an empty stack");
                history_dropped = true;
            }
            Err(error) => {
                log::warn!("3mf: persisted history ignored ({error})");
                history_dropped = true;
            }
        }
    } else if part_exists(package, &history_part) {
        history_dropped = true;
    }
    mesh.history.set_persisted_truncated(history_dropped);
    mesh.recover_gradient_ramps();
    if !project.paint_sources_complete {
        legacy_paint::recover_missing(mesh);
    }

    // The high-water mark must never let the next allocation reuse a live
    // manual label, even if the stored value is stale or corrupt.
    let floor = mesh
        .segment_labels
        .iter()
        .copied()
        .filter(|&label| label >= MANUAL_SEGMENT_OFFSET)
        .max()
        .map_or(MANUAL_SEGMENT_OFFSET, |label| {
            label.saturating_add(1).max(MANUAL_SEGMENT_OFFSET)
        });
    mesh.next_manual_label = mesh.next_manual_label.max(floor);
    RestoreOutcome {
        state: EngineeringState::Restored,
        history_dropped,
    }
}

/// Engineering part names for a plate's zero-based ordinal.
fn plate_part_name(plate_index: usize) -> String {
    format!("{ENGINEERING_DIR}/plate_{}.bin", plate_index + 1)
}

fn history_part_name(plate_index: usize) -> String {
    format!("{ENGINEERING_DIR}/history_{}.bin", plate_index + 1)
}

/// Whether the package lists a part with exactly this name.
fn part_exists(package: &Package, name: &str) -> bool {
    package.part_names().iter().any(|part| part == name)
}

/// `model_settings.config`, or the "no plate information" default on any
/// failure.
fn read_settings(package: &mut Package) -> ModelSettings {
    let Some(bytes) = read_optional(package, MODEL_SETTINGS_PART) else {
        return ModelSettings::default();
    };
    match plates::parse(&bytes) {
        Ok(settings) => settings,
        Err(error) => {
            log::warn!("3mf: {error}; importing the file as a single plate");
            ModelSettings::default()
        }
    }
}

/// Read an optional part, warning only when it exists but cannot be read.
fn read_optional(package: &mut Package, name: &str) -> Option<Vec<u8>> {
    match package.read(name) {
        Ok(bytes) => Some(bytes),
        Err(error) => {
            if package.part_names().iter().any(|part| part == name) {
                log::warn!("3mf: optional part {name:?} could not be read: {error}");
            }
            None
        }
    }
}

/// The filament palette from `project_settings.config`, or `None` when the
/// file has no usable one.
fn read_palette(bytes: &[u8]) -> Option<ProjectMaterials> {
    let Ok(text) = std::str::from_utf8(bytes) else {
        log::warn!("3mf: project_settings.config is not UTF-8; importing without a palette");
        return None;
    };
    match materials::project_materials(text) {
        Ok(palette) => Some(palette),
        Err(error) => {
            log::warn!("3mf: {error}; importing without a palette");
            None
        }
    }
}

/// The bed footprint from `printable_area`'s maximum x and y, or `(0, 0)` when
/// the file has no usable area. Both the array form (`["0x0", ...]`) and the
/// comma-separated string form written by machine presets are accepted.
pub(crate) fn bed_size(bytes: &[u8]) -> (f32, f32) {
    let Ok(text) = std::str::from_utf8(bytes) else {
        return (0.0, 0.0);
    };
    let Ok(root) = serde_json::from_str::<serde_json::Value>(text) else {
        return (0.0, 0.0);
    };
    let Some(area) = root.get("printable_area") else {
        return (0.0, 0.0);
    };
    let mut points: Vec<&str> = Vec::new();
    match area {
        serde_json::Value::Array(entries) => {
            points.extend(entries.iter().filter_map(|entry| entry.as_str()));
        }
        serde_json::Value::String(area) => points.extend(area.split(',')),
        _ => return (0.0, 0.0),
    }

    let mut max = [0.0f32; 2];
    for point in points {
        let mut coordinates = point.trim().split('x');
        let (Some(x), Some(y), None) = (coordinates.next(), coordinates.next(), coordinates.next())
        else {
            continue;
        };
        let (Ok(x), Ok(y)) = (x.trim().parse::<f32>(), y.trim().parse::<f32>()) else {
            continue;
        };
        if x.is_finite() && y.is_finite() {
            max[0] = max[0].max(x);
            max[1] = max[1].max(y);
        }
    }
    if max[0] > 0.0 && max[1] > 0.0 {
        (max[0], max[1])
    } else {
        (0.0, 0.0)
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;
    use crate::export::project_state::{
        encode_history, encode_plate_state, encode_project_json, fingerprint_of, Fingerprint,
        PlateStateHeader, ProjectState,
    };
    use crate::mesh::history::{History, OpKind, RawEntry};
    use crate::mesh::model::{MeshModel, DEFAULT_FACE_COLOR, MANUAL_SEGMENT_OFFSET};
    use crate::threemf::test_support::{
        mesh_object, model_settings, model_xml, plate, project_settings, Fixture,
    };

    /// A palette whose entry `i` is the grey `(i + 1)`; a face resolved to
    /// palette index `i` therefore has the easy-to-read colour `colour(i)`.
    const PALETTE: &[&str] = &["#010101", "#020202", "#030303", "#040404"];

    const TRIANGLE_VERTICES: &[[f32; 3]] = &[[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]];
    const TRIANGLE_FACE: &[[u32; 3]] = &[[0, 1, 2]];
    const QUAD_VERTICES: &[[f32; 3]] = &[
        [0.0, 0.0, 0.0],
        [1.0, 0.0, 0.0],
        [0.0, 1.0, 0.0],
        [1.0, 1.0, 0.0],
    ];
    const QUAD_FACES: &[[u32; 3]] = &[[0, 1, 2], [1, 3, 2]];

    fn colour(index: usize) -> [u8; 4] {
        let value = index as u8 + 1;
        [value, value, value, 255]
    }

    fn model_part(resources: &str, build: &str) -> Vec<u8> {
        model_xml("millimeter", resources, build)
    }

    fn read(entries: Vec<(&'static str, Vec<u8>)>) -> ImportedProject {
        let fixture = Fixture::new(&entries);
        read_project(fixture.path(), &|_, _| {}).expect("synthetic 3MF imports")
    }

    fn one_object(object: &str) -> Vec<(&'static str, Vec<u8>)> {
        vec![(
            "3D/3dmodel.model",
            model_part(object, r#"<item objectid="1"/>"#),
        )]
    }

    #[test]
    fn native_mixed_slots_keep_distinct_regions_when_preview_colours_match() {
        let object = mesh_object(
            1,
            "Cylinder paint",
            TRIANGLE_VERTICES,
            &[[0, 1, 2]; 7],
            &[
                None,
                Some("2C"),
                Some("3C"),
                Some("4C"),
                Some("5C"),
                Some("6C"),
                Some("4"),
            ],
        );
        let mut entries = one_object(&object);
        let config = serde_json::json!({
            "filament_colour": ["#212721", "#BA0C2F", "#FFFFFF", "#00FFFF"],
            "mixed_filament_definitions": concat!(
                "2,3,1,1,50,0,g,w,m0,z2,xa0,xb0,d0,o0,u1,cm3,r1/0.8000/0.2000;",
                "1,2,0,1,50,0,g,w,m2,z0,d0;",
                "1,2,1,1,62,0,g,w,m2,z0,xa0,xb0,d0,o0,u2,cm0;",
                "1,2,1,1,50,0,g,w,m2,z0,d1;",
                "1,2,1,1,25,0,g,w,m2,z0,xa0,xb0,d0,o0,u3,cm1,1234;",
                "1,2,1,1,50,0,g123,w16/43/41,m0,z0,xa0,xb0,d0,o0,u4,cm2;",
                "2,3,1,1,50,0,g,w,m2,z0,d0,cm0"
            ),
        });
        entries.push((PROJECT_SETTINGS_PART, serde_json::to_vec(&config).unwrap()));
        let project = read(entries);
        let mesh = &project.plates[0].mesh;
        assert_eq!(mesh.segments.len(), 6);
        assert_eq!(
            mesh.face_colors,
            vec![
                [33, 39, 33, 255],
                [221, 134, 151, 255],
                [128, 22, 42, 255],
                [119, 140, 148, 255],
                [190, 116, 130, 255],
                [221, 134, 151, 255],
                [33, 39, 33, 255],
            ],
        );
        assert_eq!(mesh.segment_labels, vec![0, 1, 2, 3, 4, 5, 0]);
        assert_eq!(mesh.source_material_slots, vec![1, 5, 6, 7, 8, 9, 1]);
        assert!(mesh.gradient_faces.is_empty());
    }

    #[test]
    #[ignore = "needs CYM_REAL_MIXED_3MF"]
    fn the_native_cylinder_preserves_four_painted_material_slots() {
        let path = std::env::var_os("CYM_REAL_MIXED_3MF").expect("CYM_REAL_MIXED_3MF");
        let project = read_project(Path::new(&path), &|_, _| {}).unwrap();
        let mesh = &project.plates[0].mesh;
        let mut counts = std::collections::BTreeMap::new();
        for (&slot, &color) in mesh.source_material_slots.iter().zip(&mesh.face_colors) {
            *counts.entry((slot, color)).or_insert(0u32) += 1;
        }
        eprintln!("Native cylinder {} faces: {counts:?}", mesh.faces.len());
        assert_eq!(mesh.segments.len(), 5);
        assert_eq!(mesh.source_material_slots.len(), mesh.faces.len());
        assert_eq!(
            counts.into_keys().collect::<Vec<_>>(),
            vec![
                (1, [33, 39, 33, 255]),
                (5, [221, 134, 151, 255]),
                (6, [128, 22, 42, 255]),
                (7, [119, 140, 148, 255]),
                (8, [190, 116, 130, 255]),
            ]
        );
        assert!(mesh.gradient_faces.is_empty());
    }

    #[test]
    fn engineering_restore_keeps_only_geometry_matched_original_materials() {
        let object = mesh_object(
            1,
            "Edited paint",
            TRIANGLE_VERTICES,
            TRIANGLE_FACE,
            &[Some("3C")],
        );
        let mut entries = one_object(&object);
        let config = serde_json::json!({
            "filament_colour": PALETTE,
            "mixed_filament_definitions": "1,2,1,1,50;1,3,1,1,50",
        });
        entries.push((PROJECT_SETTINGS_PART, serde_json::to_vec(&config).unwrap()));
        let fingerprint = fingerprint_of(TRIANGLE_VERTICES, TRIANGLE_FACE);
        let header = ProjectState {
            unit: "millimeter".into(),
            active_plate: 1,
            paint_sources_complete: true,
            plates: vec![PlateStateHeader {
                plater_id: 1,
                name: "Edited paint".into(),
                object_ids: vec![1],
                fingerprint,
                history_truncated: false,
                placement_offset: [0.0; 3],
            }],
        };
        let mut stored = MeshModel::new();
        stored.faces = TRIANGLE_FACE.to_vec();
        stored.face_colors = vec![[99, 100, 101, 255]];
        stored.segment_labels = vec![0];
        // Original slot 5 persists even though the model's current paint uses slot 6.
        stored.source_material_slots = vec![5];
        stored.source_material_colors = (0..5).map(|index| Some(colour(index))).collect();
        entries.push((
            ENGINEERING_HEADER_PART,
            encode_project_json(&header).into_bytes(),
        ));
        entries.push(("Metadata/cym/plate_1.bin", encode_plate_state(&stored)));
        let (name, valid) = source_materials_entry(0, &stored, fingerprint).unwrap();
        const SIDECAR: &str = "Metadata/cym/source_materials_1.bin";
        assert_eq!(name, SIDECAR);
        let wrong_fingerprint = Fingerprint {
            hash: fingerprint.hash ^ 1,
            ..fingerprint
        };
        let (_, stale) = source_materials_entry(0, &stored, wrong_fingerprint).unwrap();
        for (sidecar, original_slots) in [
            (None, &[][..]),
            (Some(stale), &[][..]),
            (Some(valid), &[5][..]),
        ] {
            let mut entries = entries.clone();
            if let Some(sidecar) = sidecar {
                entries.push((SIDECAR, sidecar));
            }
            let project = read(entries);
            let plate = &project.plates[0];
            assert_eq!(plate.engineering_state, EngineeringState::Restored);
            assert_eq!(plate.mesh.face_colors, [[99, 100, 101, 255]]);
            assert_eq!(plate.mesh.source_material_slots, original_slots);
            assert_eq!(
                plate.mesh.source_material_colors.get(4).copied().flatten(),
                (!original_slots.is_empty()).then(|| colour(4))
            );
        }
    }

    #[test]
    fn paint_expansion_is_counted_across_all_base_faces() {
        let paint = [Some("841".into()), Some("841".into())];
        assert_eq!(decode_paint(&paint, 4).unwrap().1, 4);
        let error = decode_paint(&paint, 3).unwrap_err();
        assert!(error.contains("expanded triangle budget"), "{error}");
    }

    #[test]
    fn corrupt_texture_metadata_warns_without_discarding_saved_face_colors() {
        let object = mesh_object(
            1,
            "Textured",
            TRIANGLE_VERTICES,
            TRIANGLE_FACE,
            &[Some("4")],
        );
        let mut entries = one_object(&object);
        let mut mesh = MeshModel::new();
        mesh.vertices = TRIANGLE_VERTICES.to_vec();
        mesh.faces = TRIANGLE_FACE.to_vec();
        mesh.face_colors = vec![[55, 66, 77, 255]];
        mesh.segment_labels = vec![0];
        let state = project_state::ProjectState {
            unit: "millimeter".into(),
            active_plate: 1,
            paint_sources_complete: true,
            plates: vec![project_state::PlateStateHeader {
                plater_id: 1,
                name: "Textured".into(),
                object_ids: vec![1],
                fingerprint: project_state::fingerprint_of(&mesh.vertices, &mesh.faces),
                history_truncated: false,
                placement_offset: [0.0; 3],
            }],
        };
        entries.push((
            "Metadata/cym/project.json",
            project_state::encode_project_json(&state).into_bytes(),
        ));
        entries.push((
            "Metadata/cym/plate_1.bin",
            project_state::encode_plate_state(&mesh),
        ));
        entries.push(("Metadata/cym/texture_1.json", b"bad JSON".to_vec()));
        let imported = read(entries);
        let plate = &imported.plates[0];
        assert_eq!(plate.engineering_state, EngineeringState::Restored);
        assert_eq!(plate.mesh.face_colors, mesh.face_colors);
        assert!(plate.mesh.texture_state_dropped);
        assert!(plate.mesh.texture_slot_mapping.is_none());
    }

    #[test]
    fn imports_geometry_and_paint_from_a_synthetic_3mf() {
        let object = mesh_object(1, "Part", QUAD_VERTICES, QUAD_FACES, &[Some("1C"), None]);
        let mut entries = one_object(&object);
        entries.push((
            "Metadata/project_settings.config",
            project_settings(PALETTE, None),
        ));

        let project = read(entries);
        assert_eq!(project.unit, "millimeter");
        assert_eq!(project.plates.len(), 1);
        let plate = &project.plates[0];
        assert_eq!(plate.plater_id, 1);
        // No model_settings: the grouping default names the only plate.
        assert_eq!(plate.name, "plate 1");
        assert_eq!(plate.engineering_state, EngineeringState::Absent);
        assert!(!plate.history_dropped);
        assert_eq!(plate.mesh.vertices, QUAD_VERTICES);
        assert_eq!(plate.mesh.faces, QUAD_FACES);
        // `1C` decodes to a leaf in extruder slot 4 -> palette index 3.
        assert_eq!(plate.mesh.face_colors, vec![colour(3), colour(0)]);
    }

    #[test]
    fn a_subdivided_paint_value_produces_more_faces_than_triangles() {
        let object = mesh_object(
            1,
            "Part",
            TRIANGLE_VERTICES,
            TRIANGLE_FACE,
            &[Some("000800303")],
        );
        let mut entries = one_object(&object);
        entries.push((
            "Metadata/project_settings.config",
            project_settings(PALETTE, None),
        ));

        let project = read(entries);
        let mesh = &project.plates[0].mesh;
        assert_eq!(mesh.faces.len(), 7);
        let unpainted = mesh.face_colors.iter().filter(|c| **c == colour(0)).count();
        let slot_two = mesh.face_colors.iter().filter(|c| **c == colour(1)).count();
        assert_eq!(unpainted, 6);
        assert_eq!(slot_two, 1);
    }

    #[test]
    #[ignore = "requires the real edited panda CYM 3MF"]
    fn panda_legacy_brushes_recover_as_drawing_colours() {
        let path = std::env::var("CYM_LEGACY_PAINT_3MF").expect("CYM_LEGACY_PAINT_3MF");
        let plate = load_plate(Path::new(&path), 0, &|_, _| {}).unwrap();
        assert_eq!(plate.engineering_state, EngineeringState::Restored);
        let groups = plate.mesh.paint_source_groups();
        assert_eq!(groups.len(), 4, "{groups:?}");
        for color in [
            [255, 0, 0, 255],
            [0, 176, 80, 255],
            [112, 48, 160, 255],
            [0, 176, 240, 255],
        ] {
            assert!(
                groups
                    .iter()
                    .any(|(source, count)| *source == color && *count > 500),
                "{groups:?}"
            );
        }
        let used = plate.mesh.used_colors();
        assert!(used.len() <= 10, "{used:?}");
        let mut package = Package::open(Path::new(&path)).unwrap();
        let encoded = package.read("Metadata/cym/plate_1.bin").unwrap();
        let snapshot = project_state::decode_plate_state(&encoded, plate.mesh.faces.len()).unwrap();
        assert_eq!(plate.mesh.face_colors, snapshot.face_colors);
        eprintln!(
            "Legacy file: drawing groups {groups:?}, {} used colours",
            used.len()
        );
    }

    #[test]
    fn slicer_extruder_three_is_white_in_a_four_slot_project() {
        let object = mesh_object(1, "Body", QUAD_VERTICES, QUAD_FACES, &[None, Some("1C")]);
        let mut entries = one_object(&object);
        entries.push((
            "Metadata/project_settings.config",
            br##"{"filament_colour":["#212721","#BA0C2F","#FFFFFF","#00FFFF"]}"##.to_vec(),
        ));
        entries.push((
            "Metadata/model_settings.config",
            br#"<config><object id="1"><metadata key="extruder" value="3"/></object></config>"#
                .to_vec(),
        ));
        let p = read(entries);
        assert_eq!(
            p.plates[0].mesh.face_colors,
            vec![[255, 255, 255, 255], [0, 255, 255, 255]]
        );
    }

    #[test]
    fn without_engineering_parts_each_inherited_colour_becomes_its_own_region() {
        let object = mesh_object(1, "Part", QUAD_VERTICES, QUAD_FACES, &[Some("1C"), None]);
        let mut entries = one_object(&object);
        entries.push((
            "Metadata/project_settings.config",
            project_settings(PALETTE, None),
        ));

        let project = read(entries);
        let plate = &project.plates[0];
        assert_eq!(plate.engineering_state, EngineeringState::Absent);
        assert_eq!(plate.mesh.face_colors, vec![colour(3), colour(0)]);
        // Two distinct inherited colours -> two regions, one per colour. A
        // single giant region made every fill act on the whole model.
        assert_ne!(
            plate.mesh.segment_labels[0], plate.mesh.segment_labels[1],
            "the two inherited colours must not collapse into one region"
        );
        assert_eq!(plate.mesh.segments.len(), 2);
        assert!(plate.mesh.segment_names.is_empty());
        assert_eq!(plate.mesh.next_manual_label, MANUAL_SEGMENT_OFFSET);
    }

    #[test]
    fn a_single_inherited_colour_still_yields_the_single_region() {
        let object = mesh_object(
            1,
            "Part",
            QUAD_VERTICES,
            QUAD_FACES,
            &[Some("1C"), Some("1C")],
        );
        let mut entries = one_object(&object);
        entries.push((
            "Metadata/project_settings.config",
            project_settings(PALETTE, None),
        ));

        let plate = &read(entries).plates[0];
        assert_eq!(plate.mesh.segment_labels, vec![0, 0]);
        assert_eq!(plate.mesh.segments.len(), 1);
    }

    #[test]
    fn with_matching_engineering_parts_the_exact_state_is_restored() {
        let object = mesh_object(1, "Part", QUAD_VERTICES, QUAD_FACES, &[None, None]);

        let mut stored = MeshModel::new();
        stored.vertices = QUAD_VERTICES.to_vec();
        stored.faces = QUAD_FACES.to_vec();
        stored.face_colors = vec![[1, 2, 3, 255], [250, 240, 230, 128]];
        stored.segment_labels = vec![0, 100_003];
        stored.segment_names =
            HashMap::from([(0, "背景".to_string()), (100_003, "左臂".to_string())]);
        stored.next_manual_label = 100_009;
        stored.history = History::from_raw(
            vec![RawEntry {
                gradient_context: None,
                kind: OpKind::Paint,
                stroke_id: Some(4),
                colors: vec![(0, [9, 9, 9, 255])],
                labels: vec![],
                gradients: Vec::new(),
            }],
            vec![],
        );

        let header = ProjectState {
            active_plate: 1,
            paint_sources_complete: false,
            unit: "millimeter".to_string(),
            plates: vec![PlateStateHeader {
                history_truncated: false,
                placement_offset: [0.0; 3],
                plater_id: 1,
                name: "Part".to_string(),
                object_ids: vec![1],
                fingerprint: fingerprint_of(QUAD_VERTICES, QUAD_FACES),
            }],
        };

        let entries = vec![
            (
                "3D/3dmodel.model",
                model_part(&object, r#"<item objectid="1"/>"#),
            ),
            (
                "Metadata/project_settings.config",
                project_settings(PALETTE, None),
            ),
            (
                "Metadata/cym/project.json",
                encode_project_json(&header).into_bytes(),
            ),
            ("Metadata/cym/plate_1.bin", encode_plate_state(&stored)),
            (
                "Metadata/cym/history_1.bin",
                encode_history(&stored.history),
            ),
        ];

        let mut project = read(entries);
        let mut plate = project.plates.remove(0);
        assert_eq!(plate.engineering_state, EngineeringState::Restored);
        assert!(!plate.history_dropped);
        assert_eq!(plate.mesh.face_colors, stored.face_colors);
        assert_eq!(plate.mesh.segment_labels, vec![0, 100_003]);
        // Names survive through `rebuild_segments`, i.e. they reach the DTO.
        assert_eq!(plate.mesh.segment_names[&100_003], "左臂");
        assert_eq!(plate.mesh.segments[&100_003].name, "左臂");
        assert_eq!(plate.mesh.segments[&0].name, "背景");
        assert_eq!(plate.mesh.next_manual_label, 100_009);
        assert!(plate.mesh.history.can_undo());
        // The restored stack is the saved one, not an empty placeholder.
        plate
            .mesh
            .history
            .undo(&mut plate.mesh.face_colors, &mut plate.mesh.segment_labels)
            .expect("one undo step");
        assert_eq!(plate.mesh.face_colors[0], [9, 9, 9, 255]);
    }

    #[test]
    fn a_fingerprint_mismatch_falls_back_to_plain_import_instead_of_failing() {
        let object = mesh_object(1, "Part", QUAD_VERTICES, QUAD_FACES, &[Some("1C"), None]);

        let mut stored = MeshModel::new();
        stored.vertices = QUAD_VERTICES.to_vec();
        stored.faces = QUAD_FACES.to_vec();
        stored.face_colors = vec![[9, 9, 9, 255], [9, 9, 9, 255]];
        stored.segment_labels = vec![100_001, 100_001];
        stored.next_manual_label = 100_002;

        let header = ProjectState {
            active_plate: 1,
            paint_sources_complete: false,
            unit: "millimeter".to_string(),
            plates: vec![PlateStateHeader {
                history_truncated: false,
                placement_offset: [0.0; 3],
                plater_id: 1,
                name: "Part".to_string(),
                object_ids: vec![1],
                // Same counts, different hash: the geometry changed.
                fingerprint: Fingerprint {
                    vertex_count: fingerprint_of(QUAD_VERTICES, QUAD_FACES).vertex_count,
                    face_count: fingerprint_of(QUAD_VERTICES, QUAD_FACES).face_count,
                    hash: 0xdead_beef,
                },
            }],
        };

        let entries = vec![
            (
                "3D/3dmodel.model",
                model_part(&object, r#"<item objectid="1"/>"#),
            ),
            (
                "Metadata/project_settings.config",
                project_settings(PALETTE, None),
            ),
            (
                "Metadata/cym/project.json",
                encode_project_json(&header).into_bytes(),
            ),
            ("Metadata/cym/plate_1.bin", encode_plate_state(&stored)),
        ];

        let project = read(entries);
        let plate = &project.plates[0];
        assert_eq!(
            plate.engineering_state,
            EngineeringState::Dropped,
            "a matching-counts/different-hash header is a drop, not a plain file"
        );
        assert_eq!(plate.mesh.face_colors, vec![colour(3), colour(0)]);
        // The dropped project's stored labels are gone, so regions fall back to
        // the inherited colours — but they are regions, not one blob.
        assert_eq!(plate.mesh.segments.len(), 2);
        assert_eq!(plate.mesh.next_manual_label, MANUAL_SEGMENT_OFFSET);
    }

    #[test]
    fn a_corrupt_engineering_part_falls_back_to_plain_import() {
        let object = mesh_object(1, "Part", QUAD_VERTICES, QUAD_FACES, &[Some("1C"), None]);

        let state = {
            let mut mesh = MeshModel::new();
            mesh.vertices = QUAD_VERTICES.to_vec();
            mesh.faces = QUAD_FACES.to_vec();
            mesh.face_colors = vec![[9, 9, 9, 255], [9, 9, 9, 255]];
            mesh.segment_labels = vec![0, 0];
            encode_plate_state(&mesh)
        };

        let header = ProjectState {
            active_plate: 1,
            paint_sources_complete: false,
            unit: "millimeter".to_string(),
            plates: vec![PlateStateHeader {
                history_truncated: false,
                placement_offset: [0.0; 3],
                plater_id: 1,
                name: "Part".to_string(),
                object_ids: vec![1],
                fingerprint: fingerprint_of(QUAD_VERTICES, QUAD_FACES),
            }],
        };

        // (a) project.json is not JSON at all.
        let entries = vec![
            (
                "3D/3dmodel.model",
                model_part(&object, r#"<item objectid="1"/>"#),
            ),
            (
                "Metadata/project_settings.config",
                project_settings(PALETTE, None),
            ),
            ("Metadata/cym/project.json", b"{ not json".to_vec()),
        ];
        let project = read(entries);
        assert_eq!(
            project.plates[0].engineering_state,
            EngineeringState::Dropped
        );
        assert_eq!(project.plates[0].mesh.face_colors[0], colour(3));

        // (b) header is readable and matches, but the plate part is truncated.
        let entries = vec![
            (
                "3D/3dmodel.model",
                model_part(&object, r#"<item objectid="1"/>"#),
            ),
            (
                "Metadata/project_settings.config",
                project_settings(PALETTE, None),
            ),
            (
                "Metadata/cym/project.json",
                encode_project_json(&header).into_bytes(),
            ),
            (
                "Metadata/cym/plate_1.bin",
                b"CYMST1\x01\0truncated".to_vec(),
            ),
        ];
        let project = read(entries);
        assert_eq!(
            project.plates[0].engineering_state,
            EngineeringState::Dropped
        );
        assert_eq!(project.plates[0].mesh.face_colors[0], colour(3));

        // (c) an unknown major format_version is ignored, not fatal.
        let future =
            encode_project_json(&header).replace("\"format_version\": 1", "\"format_version\": 2");
        let entries = vec![
            (
                "3D/3dmodel.model",
                model_part(&object, r#"<item objectid="1"/>"#),
            ),
            (
                "Metadata/project_settings.config",
                project_settings(PALETTE, None),
            ),
            ("Metadata/cym/project.json", future.into_bytes()),
            ("Metadata/cym/plate_1.bin", state),
        ];
        let project = read(entries);
        assert_eq!(
            project.plates[0].engineering_state,
            EngineeringState::Dropped
        );
        assert_eq!(project.plates[0].mesh.face_colors[0], colour(3));
    }

    #[test]
    fn a_present_but_unreadable_history_is_reported_while_the_state_restores() {
        let object = mesh_object(1, "Part", QUAD_VERTICES, QUAD_FACES, &[None, None]);

        let mut stored = MeshModel::new();
        stored.vertices = QUAD_VERTICES.to_vec();
        stored.faces = QUAD_FACES.to_vec();
        stored.face_colors = vec![[1, 2, 3, 255], [4, 5, 6, 255]];
        stored.segment_labels = vec![0, 0];
        stored
            .history
            .record(OpKind::Paint, None, &[(0, [0, 0, 0, 255])], &[]);
        let mut history_bytes = encode_history(&stored.history);
        // The first entry's kind byte follows the undo count; byte 5 is the
        // reserved future transform kind this build does not know.
        history_bytes[4] = 5;

        let header = ProjectState {
            active_plate: 1,
            paint_sources_complete: false,
            unit: "millimeter".to_string(),
            plates: vec![PlateStateHeader {
                history_truncated: false,
                placement_offset: [0.0; 3],
                plater_id: 1,
                name: "Part".to_string(),
                object_ids: vec![1],
                fingerprint: fingerprint_of(QUAD_VERTICES, QUAD_FACES),
            }],
        };
        let base_entries = || {
            vec![
                (
                    "3D/3dmodel.model",
                    model_part(&object, r#"<item objectid="1"/>"#),
                ),
                (
                    "Metadata/cym/project.json",
                    encode_project_json(&header).into_bytes(),
                ),
                ("Metadata/cym/plate_1.bin", encode_plate_state(&stored)),
            ]
        };

        // The state restores; the unreadable history is reported, not silently
        // replaced by an empty stack.
        let mut entries = base_entries();
        entries.push(("Metadata/cym/history_1.bin", history_bytes));
        let project = read(entries);
        let plate = &project.plates[0];
        assert_eq!(plate.engineering_state, EngineeringState::Restored);
        assert!(plate.history_dropped);
        assert!(!plate.mesh.history.can_undo());
        assert_eq!(plate.mesh.face_colors, stored.face_colors);

        // A missing history part is not a loss: the file never carried one.
        let project = read(base_entries());
        let plate = &project.plates[0];
        assert_eq!(plate.engineering_state, EngineeringState::Restored);
        assert!(!plate.history_dropped);

        let mut entries = base_entries();
        let header_entry = entries
            .iter_mut()
            .find(|(name, _)| *name == ENGINEERING_HEADER_PART)
            .unwrap();
        let mut json: serde_json::Value = serde_json::from_slice(&header_entry.1).unwrap();
        json["plates"][0]["history_truncated"] = true.into();
        header_entry.1 = serde_json::to_vec(&json).unwrap();
        entries.push((
            "Metadata/cym/history_1.bin",
            encode_history(&stored.history),
        ));
        let project = read(entries);
        assert!(
            project.plates[0].history_dropped,
            "persisted truncation must be reported"
        );
        assert!(
            project.plates[0].mesh.history.can_undo(),
            "remaining history still restores"
        );
    }

    #[test]
    fn a_history_part_that_exceeds_the_package_read_limit_is_reported() {
        let mut mesh = MeshModel::new();
        mesh.vertices = QUAD_VERTICES.to_vec();
        mesh.faces = QUAD_FACES.to_vec();
        let fingerprint = fingerprint_of(&mesh.vertices, &mesh.faces);
        let header = ProjectState {
            active_plate: 1,
            paint_sources_complete: false,
            unit: "millimeter".into(),
            plates: vec![PlateStateHeader {
                history_truncated: false,
                placement_offset: [0.0; 3],
                plater_id: 1,
                name: "Part".into(),
                object_ids: vec![1],
                fingerprint,
            }],
        };
        let fixture = Fixture::new(&[
            ("Metadata/cym/plate_1.bin", encode_plate_state(&mesh)),
            ("Metadata/cym/history_1.bin", vec![0; 2048]),
        ]);
        let mut package = Package::open_with_limit(fixture.path(), 1024).unwrap();
        let report = try_restore(
            &mut package,
            Some(&header),
            true,
            0,
            1,
            Some(fingerprint),
            &mut mesh,
        );
        assert_eq!(report.state, EngineeringState::Restored);
        assert!(
            report.history_dropped,
            "an existing unreadable history is a loss"
        );
    }

    #[test]
    fn a_3mf_without_any_paint_or_palette_still_imports_as_default_grey() {
        let object = mesh_object(1, "Part", TRIANGLE_VERTICES, TRIANGLE_FACE, &[None]);
        let project = read(one_object(&object));
        assert_eq!(
            project.plates[0].engineering_state,
            EngineeringState::Absent
        );
        assert_eq!(project.plates[0].mesh.face_colors, vec![DEFAULT_FACE_COLOR]);

        // A project_settings part without a palette is just as optional.
        let mut entries = one_object(&object);
        entries.push((
            "Metadata/project_settings.config",
            br#"{"printable_area":["0x0","220x0","220x220","0x220"]}"#.to_vec(),
        ));
        let project = read(entries);
        assert_eq!(project.plates[0].mesh.face_colors, vec![DEFAULT_FACE_COLOR]);
    }

    #[test]
    fn plates_are_normalised_to_their_local_grid_cell() {
        // Three plates on a 220 mm bed: cells (0,0), (1,0), (0,1) with the
        // 1.2x spacing, so the world origins are (0,0), (264,0), (0,-264).
        let plate_one = mesh_object(1, "A", TRIANGLE_VERTICES, TRIANGLE_FACE, &[None]);
        let mut plate_two_vertices = TRIANGLE_VERTICES.to_vec();
        for vertex in &mut plate_two_vertices {
            vertex[0] += 264.0;
        }
        let plate_two = mesh_object(2, "B", &plate_two_vertices, TRIANGLE_FACE, &[None]);
        let mut plate_three_vertices = TRIANGLE_VERTICES.to_vec();
        for vertex in &mut plate_three_vertices {
            vertex[1] -= 264.0;
        }
        let plate_three = mesh_object(3, "C", &plate_three_vertices, TRIANGLE_FACE, &[None]);

        let resources = format!("{plate_one}{plate_two}{plate_three}");
        let build = r#"<item objectid="1"/><item objectid="2"/><item objectid="3"/>"#;
        let plates = format!(
            "{}{}{}",
            plate(1, "Front", &[1]),
            plate(2, "Middle", &[2]),
            plate(3, "Back", &[3]),
        );
        let entries = vec![
            ("3D/3dmodel.model", model_part(&resources, build)),
            (
                "Metadata/model_settings.config",
                model_settings("", &plates),
            ),
            (
                "Metadata/project_settings.config",
                project_settings(PALETTE, Some((220.0, 220.0))),
            ),
        ];

        let project = read(entries);
        assert_eq!(project.plates.len(), 3);
        for (index, name) in ["Front", "Middle", "Back"].iter().enumerate() {
            let plate = &project.plates[index];
            assert_eq!(plate.plater_id, index as u32 + 1);
            assert_eq!(&plate.name, name);
            assert_eq!(plate.mesh.vertices[0], [0.0, 0.0, 0.0]);
            assert_eq!(plate.mesh.vertices[1], [1.0, 0.0, 0.0]);
        }
    }

    #[test]
    fn a_plate_without_paint_uses_the_objects_extruder_colour_and_name() {
        let object = mesh_object(1, "Body", TRIANGLE_VERTICES, TRIANGLE_FACE, &[None]);
        let objects = r#"<object id="1"><metadata key="name" value="Body"/>
            <metadata key="extruder" value="1"/></object>"#;
        // The plate has no `plater_name`, so the object's name is the fallback.
        let plates = plate(1, "", &[1]);

        let entries = vec![
            (
                "3D/3dmodel.model",
                model_part(&object, r#"<item objectid="1"/>"#),
            ),
            (
                "Metadata/model_settings.config",
                model_settings(objects, &plates),
            ),
            (
                "Metadata/project_settings.config",
                project_settings(PALETTE, None),
            ),
        ];

        let project = read(entries);
        let plate = &project.plates[0];
        assert_eq!(plate.name, "Body");
        assert_eq!(plate.plater_id, 1);
        assert_eq!(plate.engineering_state, EngineeringState::Absent);
        // Slot 1 is palette index 0.
        assert_eq!(plate.mesh.face_colors, vec![colour(0)]);
    }

    #[test]
    fn each_object_on_a_plate_keeps_its_own_extruder_base_colour() {
        // Two unpainted objects on one plate, on different filaments. The
        // plate must not collapse them onto the first object's colour.
        let object_a = mesh_object(1, "A", TRIANGLE_VERTICES, TRIANGLE_FACE, &[None]);
        let mut b_vertices = TRIANGLE_VERTICES.to_vec();
        for vertex in &mut b_vertices {
            vertex[0] += 10.0;
        }
        let object_b = mesh_object(2, "B", &b_vertices, TRIANGLE_FACE, &[None]);
        let objects = r#"<object id="1"><metadata key="extruder" value="0"/></object>
            <object id="2"><metadata key="extruder" value="2"/></object>"#;
        let entries = vec![
            (
                "3D/3dmodel.model",
                model_part(
                    &format!("{object_a}{object_b}"),
                    r#"<item objectid="1"/><item objectid="2"/>"#,
                ),
            ),
            (
                "Metadata/model_settings.config",
                model_settings(objects, &plate(1, "Mixed", &[1, 2])),
            ),
            (
                "Metadata/project_settings.config",
                project_settings(PALETTE, None),
            ),
        ];

        let project = read(entries);
        let plate = &project.plates[0];
        assert_eq!(plate.mesh.faces.len(), 2);
        // Extruder 0 uses the default slot; slot 2 is palette index 1.
        assert_eq!(plate.mesh.face_colors, vec![colour(0), colour(1)]);
    }

    #[test]
    fn an_unpainted_object_keeps_its_own_base_beside_a_painted_object() {
        // Object A is painted (slot 4 -> palette index 3); object B has no
        // paint at all and must use its own extruder (2), not slot 1.
        let object_a = mesh_object(
            1,
            "Painted",
            TRIANGLE_VERTICES,
            TRIANGLE_FACE,
            &[Some("1C")],
        );
        let mut b_vertices = TRIANGLE_VERTICES.to_vec();
        for vertex in &mut b_vertices {
            vertex[0] += 10.0;
        }
        let object_b = mesh_object(2, "Plain", &b_vertices, TRIANGLE_FACE, &[None]);
        let objects = r#"<object id="1"><metadata key="extruder" value="0"/></object>
            <object id="2"><metadata key="extruder" value="2"/></object>"#;
        let entries = vec![
            (
                "3D/3dmodel.model",
                model_part(
                    &format!("{object_a}{object_b}"),
                    r#"<item objectid="1"/><item objectid="2"/>"#,
                ),
            ),
            (
                "Metadata/model_settings.config",
                model_settings(objects, &plate(1, "Mixed", &[1, 2])),
            ),
            (
                "Metadata/project_settings.config",
                project_settings(PALETTE, None),
            ),
        ];

        let project = read(entries);
        let plate = &project.plates[0];
        assert_eq!(plate.mesh.faces.len(), 2);
        assert_eq!(
            plate.mesh.face_colors,
            vec![colour(3), colour(1)],
            "the painted face follows paint_color; the plain face follows its own extruder"
        );
    }

    #[test]
    fn an_unnamed_plate_falls_back_to_its_ordinal() {
        let object = mesh_object(1, "", TRIANGLE_VERTICES, TRIANGLE_FACE, &[None]);
        let entries = vec![
            (
                "3D/3dmodel.model",
                model_part(&object, r#"<item objectid="1"/>"#),
            ),
            (
                "Metadata/model_settings.config",
                model_settings("", &plate(2, "", &[1])),
            ),
        ];

        let project = read(entries);
        // No plate name and no object metadata: the 1-based ordinal names it.
        assert_eq!(project.plates[0].plater_id, 2);
        assert_eq!(project.plates[0].name, "plate 2");
    }

    #[test]
    fn the_label_high_water_mark_is_clamped_to_the_live_labels() {
        fn restore_with(stored_next: u32, labels: Vec<u32>) -> u32 {
            let object = mesh_object(1, "Part", QUAD_VERTICES, QUAD_FACES, &[None, None]);
            let mut stored = MeshModel::new();
            stored.vertices = QUAD_VERTICES.to_vec();
            stored.faces = QUAD_FACES.to_vec();
            stored.face_colors = vec![[1, 1, 1, 255], [2, 2, 2, 255]];
            stored.segment_labels = labels;
            stored.next_manual_label = stored_next;
            let header = ProjectState {
                active_plate: 1,
                paint_sources_complete: false,
                unit: "millimeter".to_string(),
                plates: vec![PlateStateHeader {
                    history_truncated: false,
                    placement_offset: [0.0; 3],
                    plater_id: 1,
                    name: "Part".to_string(),
                    object_ids: vec![1],
                    fingerprint: fingerprint_of(QUAD_VERTICES, QUAD_FACES),
                }],
            };
            let entries = vec![
                (
                    "3D/3dmodel.model",
                    model_part(&object, r#"<item objectid="1"/>"#),
                ),
                (
                    "Metadata/cym/project.json",
                    encode_project_json(&header).into_bytes(),
                ),
                ("Metadata/cym/plate_1.bin", encode_plate_state(&stored)),
            ];
            let project = read(entries);
            assert_eq!(
                project.plates[0].engineering_state,
                EngineeringState::Restored
            );
            project.plates[0].mesh.next_manual_label
        }

        // A stored high-water mark below a live manual label is raised to
        // `max(live) + 1`; a higher stored value is kept.
        assert_eq!(restore_with(0, vec![0, 100_007]), 100_008);
        assert_eq!(restore_with(100_050, vec![0, 100_007]), 100_050);
        // With no live manual label the mark never drops below the namespace.
        assert_eq!(restore_with(5, vec![0, 0]), MANUAL_SEGMENT_OFFSET);
    }

    #[test]
    fn a_malformed_paint_value_leaves_that_triangle_unpainted() {
        let object = mesh_object(1, "Part", TRIANGLE_VERTICES, TRIANGLE_FACE, &[Some("ZZ")]);
        let project = read(one_object(&object));
        assert_eq!(project.plates[0].mesh.faces.len(), 1);
        assert_eq!(project.plates[0].mesh.face_colors, vec![DEFAULT_FACE_COLOR]);
    }

    #[test]
    fn engineering_state_is_applied_to_the_plate_it_belongs_to() {
        let plate_one = mesh_object(1, "A", TRIANGLE_VERTICES, TRIANGLE_FACE, &[None]);
        // Plate 2's cell origin is one 220 mm bed + 20% spacing to the right
        // (264 mm). Fractional coordinates are deliberate: the fingerprint must
        // be taken on these world values *before* the origin is subtracted, so
        // a regression to the plate-local domain changes the hash.
        let plate_two_vertices = vec![
            [264.125, 0.25, 0.5],
            [265.375, 0.25, 0.5],
            [264.125, 1.75, 0.5],
        ];
        let local: Vec<[f32; 3]> = plate_two_vertices
            .iter()
            .map(|vertex| [vertex[0] - 264.0, vertex[1], vertex[2]])
            .collect();
        let plate_two = mesh_object(2, "B", &plate_two_vertices, TRIANGLE_FACE, &[None]);
        let resources = format!("{plate_one}{plate_two}");
        let build = r#"<item objectid="1"/><item objectid="2"/>"#;
        let plates = format!("{}{}", plate(1, "Front", &[1]), plate(2, "Back", &[2]));

        // Precondition: the two hash domains really are distinguishable for
        // this fixture, so the test below cannot pass with a local-domain hash.
        assert_ne!(
            fingerprint_of(&plate_two_vertices, TRIANGLE_FACE),
            fingerprint_of(&local, TRIANGLE_FACE),
            "world and plate-local fingerprints must differ for this fixture"
        );

        // Plate 2's state is the only one written; plate 1's header matches
        // but its part is absent, so only plate 2 may restore.
        let mut stored = MeshModel::new();
        stored.vertices = plate_two_vertices.clone();
        stored.faces = TRIANGLE_FACE.to_vec();
        stored.face_colors = vec![[7, 8, 9, 255]];
        stored.segment_labels = vec![100_010];
        stored.segment_names = HashMap::from([(100_010, "Back face".to_string())]);
        stored.next_manual_label = 100_011;

        let header = ProjectState {
            active_plate: 1,
            paint_sources_complete: false,
            unit: "millimeter".to_string(),
            plates: vec![
                PlateStateHeader {
                    history_truncated: false,
                    placement_offset: [0.0; 3],
                    plater_id: 1,
                    name: "Front".to_string(),
                    object_ids: vec![1],
                    fingerprint: fingerprint_of(TRIANGLE_VERTICES, TRIANGLE_FACE),
                },
                PlateStateHeader {
                    history_truncated: false,
                    placement_offset: [0.0; 3],
                    plater_id: 2,
                    name: "Back".to_string(),
                    object_ids: vec![2],
                    // The domain is the world coordinates as written.
                    fingerprint: fingerprint_of(&plate_two_vertices, TRIANGLE_FACE),
                },
            ],
        };

        let entries = vec![
            ("3D/3dmodel.model", model_part(&resources, build)),
            (
                "Metadata/model_settings.config",
                model_settings("", &plates),
            ),
            (
                "Metadata/project_settings.config",
                project_settings(PALETTE, Some((220.0, 220.0))),
            ),
            (
                "Metadata/cym/project.json",
                encode_project_json(&header).into_bytes(),
            ),
            ("Metadata/cym/plate_2.bin", encode_plate_state(&stored)),
        ];

        let project = read(entries);
        assert_eq!(project.plates.len(), 2);
        // Plate 1's header exists and matches, but its part is absent: the
        // file carried state for it that could not be applied (a drop, not a
        // plain file). Plate 2 restores.
        assert_eq!(
            project.plates[0].engineering_state,
            EngineeringState::Dropped
        );
        assert_eq!(
            project.plates[1].engineering_state,
            EngineeringState::Restored
        );
        // Both plates are in their own local frame: the shift still applies to
        // the geometry even though the fingerprint was taken before it.
        assert_eq!(project.plates[0].mesh.vertices[0], [0.0, 0.0, 0.0]);
        assert_eq!(project.plates[1].mesh.vertices, local);
        assert_eq!(project.plates[1].mesh.face_colors, vec![[7, 8, 9, 255]]);
        assert_eq!(project.plates[1].mesh.segment_labels, vec![100_010]);
        assert_eq!(project.plates[1].mesh.segment_names[&100_010], "Back face");
        assert_eq!(project.plates[1].mesh.next_manual_label, 100_011);
    }

    #[test]
    fn engineering_state_wire_names_are_stable() {
        assert_eq!(EngineeringState::Restored.as_str(), "restored");
        assert_eq!(EngineeringState::Absent.as_str(), "absent");
        assert_eq!(EngineeringState::Dropped.as_str(), "dropped");
    }

    #[test]
    #[ignore = "needs CYM_REAL_3MF_MULTI"]
    fn a_real_two_plate_file_yields_two_plates_with_measured_face_counts() {
        let Ok(path) = std::env::var("CYM_REAL_3MF_MULTI") else {
            return;
        };
        let p = read_project(Path::new(&path), &|_, _| {}).expect("read");
        assert_eq!(p.plates.len(), 2);
        assert_eq!(p.plates[0].mesh.faces.len(), 148_460);
        assert_eq!(p.plates[1].mesh.faces.len(), 993_568);
        // A slicer-written file carries no CYM engineering parts: plain
        // import, and no "state was dropped" notice.
        for plate in &p.plates {
            assert_eq!(plate.engineering_state, EngineeringState::Absent);
            assert!(!plate.history_dropped);
        }
    }

    #[test]
    #[ignore = "needs CYM_REAL_3MF"]
    fn a_real_single_plate_file_imports_with_the_measured_face_count() {
        let Ok(path) = std::env::var("CYM_REAL_3MF") else {
            return;
        };
        let p = read_project(Path::new(&path), &|_, _| {}).expect("read");
        assert_eq!(p.plates.len(), 1);
        assert_eq!(p.plates[0].mesh.faces.len(), 993_568);
        // 74,226 base vertices plus the midpoints subdivision creates.
        assert_eq!(p.plates[0].mesh.vertices.len(), 652_451);
        assert_eq!(p.plates[0].engineering_state, EngineeringState::Absent);
        assert!(!p.plates[0].history_dropped);
    }
    #[test]
    fn printed_positions_override_stale_plate_membership() {
        let object1 = mesh_object(1, "Right", TRIANGLE_VERTICES, TRIANGLE_FACE, &[Some("4")]);
        let object2 = mesh_object(2, "Left", TRIANGLE_VERTICES, TRIANGLE_FACE, &[Some("8")]);
        let project = read(vec![
            (
                "3D/3dmodel.model",
                model_part(
                    &(object1 + &object2),
                    r#"<item objectid="1" transform="1 0 0 0 1 0 0 0 1 140 20 0"/><item objectid="2" transform="1 0 0 0 1 0 0 0 1 20 20 0"/>"#,
                ),
            ),
            (
                "Metadata/model_settings.config",
                model_settings("", &(plate(1, "Left", &[1]) + &plate(2, "Right", &[2]))),
            ),
            (
                "Metadata/project_settings.config",
                project_settings(PALETTE, Some((100.0, 100.0))),
            ),
        ]);
        assert_eq!(project.plates[0].mesh.face_colors, vec![colour(1)]);
        assert_eq!(project.plates[1].mesh.face_colors, vec![colour(0)]);
        assert_eq!(project.plates[0].mesh.vertices[0][0], 20.0);
        assert!((project.plates[1].mesh.vertices[0][0] - 20.0).abs() <= 4.0 * f32::EPSILON * 140.0);
    }
    #[test]
    fn wrong_plate_identity_never_falls_back_to_the_first_engineering_header() {
        let object = mesh_object(1, "Part", TRIANGLE_VERTICES, TRIANGLE_FACE, &[None]);
        let mut mesh = MeshModel::new();
        mesh.vertices = TRIANGLE_VERTICES.to_vec();
        mesh.faces = TRIANGLE_FACE.to_vec();
        mesh.init_default_colors();
        mesh.segment_labels = vec![123];
        let header = ProjectState {
            active_plate: 9,
            paint_sources_complete: false,
            unit: "millimeter".into(),
            plates: vec![PlateStateHeader {
                plater_id: 9,
                name: "Wrong plate".into(),
                object_ids: vec![1],
                fingerprint: fingerprint_of(&mesh.vertices, &mesh.faces),
                history_truncated: false,
                placement_offset: [0.0; 3],
            }],
        };
        let project = read(vec![
            (
                "3D/3dmodel.model",
                model_part(&object, r#"<item objectid="1"/>"#),
            ),
            (
                "Metadata/cym/project.json",
                encode_project_json(&header).into_bytes(),
            ),
            ("Metadata/cym/plate_1.bin", encode_plate_state(&mesh)),
        ]);
        assert_eq!(
            project.plates[0].engineering_state,
            EngineeringState::Dropped
        );
        assert_ne!(project.plates[0].mesh.segment_labels, vec![123]);
    }
}
