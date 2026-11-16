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
pub mod materials;
pub mod model;
pub mod opc;
pub mod paint;
pub mod plates;
pub mod subdivide;

#[cfg(test)]
pub mod test_support;

use std::collections::HashMap;
use std::path::Path;

use self::materials::Palette;
use self::model::Scene;
use self::opc::Package;
use self::paint::PaintNode;
use self::plates::{ModelSettings, PlateGroup};
use crate::export::project_state;
use crate::mesh::model::{MeshModel, DEFAULT_FACE_COLOR, MANUAL_SEGMENT_OFFSET};

/// Package part holding `model_settings.config`.
const MODEL_SETTINGS_PART: &str = "Metadata/model_settings.config";
/// Package part holding the slicer's project settings (palette, bed size).
const PROJECT_SETTINGS_PART: &str = "Metadata/project_settings.config";
/// Directory the writer owns for engineering state.
const ENGINEERING_DIR: &str = "Metadata/cym";
/// Engineering header: format version, unit, and one fingerprint per plate.
const ENGINEERING_HEADER_PART: &str = "Metadata/cym/project.json";

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
pub fn read_project(path: &Path, progress: &dyn Fn(f32, &str)) -> Result<ImportedProject, String> {
    progress(0.02, "3mf:open");
    let mut package = Package::open(path)?;

    progress(0.10, "3mf:model");
    let scene = model::read_scene(&mut package)?;

    progress(0.25, "3mf:plates");
    let settings = read_settings(&mut package);
    let groups = plates::group(&scene.build, &settings);

    // Read each optional metadata part once, not once per plate. Whether the
    // header part *exists* is tracked separately from whether it parsed: a
    // rejected part must surface as "dropped", never as "plain file".
    let engineering_header_present = package
        .part_names()
        .iter()
        .any(|part| part == ENGINEERING_HEADER_PART);
    let engineering_header = read_engineering_header(&mut package);
    let project_settings = read_optional(&mut package, PROJECT_SETTINGS_PART);
    let palette = project_settings.as_deref().and_then(read_palette);
    let (bed_width, bed_depth) = project_settings
        .as_deref()
        .map(bed_size)
        .unwrap_or((0.0, 0.0));

    let context = PlateContext {
        scene: &scene,
        settings: &settings,
        palette: palette.as_ref(),
        bed_width,
        bed_depth,
        plate_count: groups.len(),
    };

    let total = groups.len().max(1);
    let mut imported = Vec::with_capacity(groups.len());
    for (index, group) in groups.iter().enumerate() {
        progress(
            0.30 + 0.60 * (index as f32 / total as f32),
            &format!("3mf:plate {}/{}", index + 1, total),
        );
        imported.push(import_plate(
            &mut package,
            &context,
            engineering_header.as_ref(),
            engineering_header_present,
            index,
            group,
        )?);
    }

    progress(1.0, "3mf:done");
    Ok(ImportedProject {
        unit: scene.unit,
        plates: imported,
    })
}

/// Everything about the file that is shared by all of its plates.
struct PlateContext<'a> {
    scene: &'a Scene,
    settings: &'a ModelSettings,
    palette: Option<&'a Palette>,
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
    let trees = decode_paint(&assembled.paint);
    let expanded = subdivide::expand(&assembled.vertices, &assembled.faces, &trees);

    // The fingerprint domain is the world coordinates **as written into the
    // file**, i.e. before the plate-origin shift below. The writer hashes the
    // vertices it emits (also world space); hashing the plate-local form here
    // would break on a non-zero origin because `fl(fl(world - origin) + origin)`
    // is not `world` at 264 mm (f32 ulp there dwarfs the `{:.6}` grid).
    let fingerprint = project_state::fingerprint_of(&expanded.vertices, &expanded.faces);

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

    let face_colors = resolve_face_colors(
        &expanded,
        &assembled.faces,
        &trees,
        &assembled.base_slots,
        context.palette,
    );

    let mut mesh = MeshModel::new();
    mesh.vertices = vertices;
    mesh.faces = expanded.faces;
    mesh.face_colors = face_colors;
    let face_count = mesh.faces.len();
    mesh.segment_labels = vec![0; face_count];
    mesh.unit = "millimeter".to_string();

    let restore = try_restore(
        package,
        engineering_header,
        engineering_header_present,
        index,
        fingerprint,
        &mut mesh,
    );

    // A file with colours but no engineering state has no region labels of its
    // own. Importing it with one giant region (`labels = 0`) makes every fill
    // and lasso act on the whole model and reads as a bug in the panel, so the
    // inherited colours become the regions: one region per distinct colour,
    // dominant colour first. A restored project keeps the labels it stored.
    if restore.state != EngineeringState::Restored {
        mesh.segment_labels = labels_from_colors(&mesh.face_colors);
    }

    // Derived state, in the same order every other loader uses.
    mesh.compute_normals();
    mesh.compute_bbox();
    mesh.rebuild_segments();
    mesh.build_kdtree();
    mesh.build_vertex_kdtree();
    mesh.build_adjacency();

    Ok(ImportedPlate {
        plater_id,
        name: plate_name(context.settings, context.scene, group, plater_id),
        mesh,
        engineering_state: restore.state,
        history_dropped: restore.history_dropped,
    })
}

/// Group inherited face colours into region labels.
///
/// Labels are assigned 0..N-1 ordered by descending face count, with the colour
/// bytes as the tie-break, so the result is deterministic and the dominant
/// colour becomes region 0. A single-colour model therefore still yields the
/// single region it had before.
fn labels_from_colors(colors: &[[u8; 4]]) -> Vec<u32> {
    let mut counts: HashMap<[u8; 4], u32> = HashMap::new();
    for color in colors {
        *counts.entry(*color).or_insert(0) += 1;
    }
    let mut ordered: Vec<([u8; 4], u32)> = counts.into_iter().collect();
    ordered.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    let label_of: HashMap<[u8; 4], u32> = ordered
        .iter()
        .enumerate()
        .map(|(index, (color, _))| (*color, index as u32))
        .collect();
    colors.iter().map(|color| label_of[color]).collect()
}

/// Decode every `paint_color` tree, degrading one malformed value to "no
/// paint" instead of failing the import.
fn decode_paint(paint: &[Option<String>]) -> Vec<Option<PaintNode>> {
    paint
        .iter()
        .enumerate()
        .map(|(index, value)| {
            let hex = value.as_deref()?;
            match paint::decode(hex) {
                Ok(node) => Some(node),
                Err(error) => {
                    log::warn!(
                        "3mf: face {index} has an undecodable paint_color; \
                         rendering it unpainted: {error}"
                    );
                    None
                }
            }
        })
        .collect()
}

/// Resolve one colour per expanded face.
///
/// A face with a `paint_color` slot uses that 1-based slot. A face without one
/// — no attribute at all, or a tree leaf in state 0 — uses the base slot its
/// object declared through `model_settings` `extruder` (0-based), falling back
/// to the slicer's unpainted slot 1. Bases travel per face because one plate
/// can mix objects on different filaments.
fn resolve_face_colors(
    expanded: &subdivide::Expanded,
    base_faces: &[[u32; 3]],
    trees: &[Option<PaintNode>],
    base_slots: &[Option<u8>],
    palette: Option<&Palette>,
) -> Vec<[u8; 4]> {
    let Some(palette) = palette else {
        return vec![DEFAULT_FACE_COLOR; expanded.faces.len()];
    };

    // `subdivide::expand` emits the faces of one base triangle contiguously,
    // in base-face order, so a base slot repeats by that tree's leaf count.
    // If the walk does not account for every expanded face (only possible for
    // a hand-built mesh the assembler would otherwise reject), ignore the
    // bases rather than colour one object's faces with a neighbour's base.
    let mapped = base_faces.len() == base_slots.len()
        && trees.len() == base_faces.len()
        && expanded.slots.len() == expanded.faces.len();
    let mut colors = Vec::with_capacity(expanded.faces.len());
    let mut cursor = 0usize;
    for (index, tree) in trees.iter().enumerate().take(base_faces.len()) {
        let count = match tree {
            Some(tree) => paint::leaf_count(tree),
            None => 1,
        };
        let base = if mapped { base_slots[index] } else { None };
        for &slot in expanded.slots.iter().skip(cursor).take(count) {
            colors.push(match slot {
                0 => match base {
                    Some(base) => palette_colour(palette, usize::from(base)),
                    None => palette_colour(palette, 0),
                },
                slot => palette_colour(palette, usize::from(slot) - 1),
            });
        }
        cursor += count;
    }
    if colors.len() != expanded.faces.len() {
        return (0..expanded.faces.len())
            .map(|index| match expanded.slots.get(index).copied().unwrap_or(0) {
                0 => palette_colour(palette, 0),
                slot => palette_colour(palette, usize::from(slot) - 1),
            })
            .collect();
    }
    colors
}

/// Palette index `index`, clamped to the last colour. An empty palette (which
/// the parser rejects, but the public tuple struct can still be built)
/// degrades to the default grey instead of indexing.
///
/// Mirrors `materials::colour_at`, which is private to that module, so the
/// reader can mix per-face paint slots with per-object base slots.
fn palette_colour(palette: &Palette, index: usize) -> [u8; 4] {
    match palette.0.len() {
        0 => DEFAULT_FACE_COLOR,
        len => palette.0[index.min(len - 1)],
    }
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
fn plate_origin(plater_id: u32, plate_count: usize, bed_width: f32, bed_depth: f32) -> [f32; 3] {
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
    fingerprint: project_state::Fingerprint,
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
        .get(plate_index)
        .map(|header| (plate_index, header))
        .or_else(|| project.plates.first().map(|header| (0, header)))
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
    if fingerprint != header.fingerprint {
        log::warn!(
            "3mf: engineering state belongs to different geometry \
             (stored {} vertices / {} faces, imported {} / {}); using plain import",
            header.fingerprint.vertex_count,
            header.fingerprint.face_count,
            fingerprint.vertex_count,
            fingerprint.face_count,
        );
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

    // History is optional on its own: a missing part means "open with an empty
    // stack", never "reject the file". A part that exists but cannot decode is
    // a real loss, so the DTO reports it instead of silently emptying the
    // stack.
    let mut history_dropped = false;
    if let Some(history_bytes) = read_optional(package, &history_part) {
        match project_state::decode_history(&history_bytes, mesh.faces.len()) {
            Ok(Some(history)) => mesh.history = history,
            Ok(None) => {
                log::warn!("3mf: persisted history is unreadable; keeping an empty stack");
                history_dropped = true;
            }
            Err(error) => {
                log::warn!("3mf: persisted history ignored ({error})");
                history_dropped = true;
            }
        }
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
fn read_palette(bytes: &[u8]) -> Option<Palette> {
    let Ok(text) = std::str::from_utf8(bytes) else {
        log::warn!("3mf: project_settings.config is not UTF-8; importing without a palette");
        return None;
    };
    match materials::palette_from_project_settings(text) {
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
fn bed_size(bytes: &[u8]) -> (f32, f32) {
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
        let object = mesh_object(1, "Part", QUAD_VERTICES, QUAD_FACES, &[Some("1C"), Some("1C")]);
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
                kind: OpKind::Paint,
                stroke_id: Some(4),
                colors: vec![(0, [9, 9, 9, 255])],
                labels: vec![],
            }],
            vec![],
        );

        let header = ProjectState {
            active_plate: 1,
            unit: "millimeter".to_string(),
            plates: vec![PlateStateHeader {
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
            unit: "millimeter".to_string(),
            plates: vec![PlateStateHeader {
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
            unit: "millimeter".to_string(),
            plates: vec![PlateStateHeader {
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
            unit: "millimeter".to_string(),
            plates: vec![PlateStateHeader {
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
        // `extruder` is 0-based, so extruder 1 is palette index 1.
        assert_eq!(plate.mesh.face_colors, vec![colour(1)]);
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
        // Extruder 0 -> palette index 0, extruder 2 -> palette index 2.
        assert_eq!(plate.mesh.face_colors, vec![colour(0), colour(2)]);
    }

    #[test]
    fn an_unpainted_object_keeps_its_own_base_beside_a_painted_object() {
        // Object A is painted (slot 4 -> palette index 3); object B has no
        // paint at all and must use its own extruder (2), not slot 1.
        let object_a = mesh_object(1, "Painted", TRIANGLE_VERTICES, TRIANGLE_FACE, &[Some("1C")]);
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
            vec![colour(3), colour(2)],
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
                unit: "millimeter".to_string(),
                plates: vec![PlateStateHeader {
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
            unit: "millimeter".to_string(),
            plates: vec![
                PlateStateHeader {
                    plater_id: 1,
                    name: "Front".to_string(),
                    object_ids: vec![1],
                    fingerprint: fingerprint_of(TRIANGLE_VERTICES, TRIANGLE_FACE),
                },
                PlateStateHeader {
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
}
