//! Explicit RGB-to-filament assignments for editable texture projects.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::hash::{Hash, Hasher};

use serde::{Deserialize, Serialize};

use super::paint_color::MAX_EXTRUDER_SLOT;
use super::presets::{library, ExportSelection, FilamentPreset, ProcessPreset, ResolvedSelection};
use super::quantize::{quantize_face_colors, Quantized};
use crate::mesh::model::{MeshModel, DEFAULT_FACE_COLOR};
use crate::mesh::texture::MAX_FACES;

pub type Rgb = [u8; 3];
const MAX_EXPORT_ROWS: usize = MAX_EXTRUDER_SLOT as usize;
// Keep one previous palette's worth of absent colours for undo and recolouring.
const MAX_INACTIVE_ASSIGNMENTS: usize = MAX_EXPORT_ROWS;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "camelCase")]
pub struct TextureColorAssignment {
    pub rgb: Rgb,
    pub slot: u8,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TextureSlotMapping {
    pub selection: ExportSelection,
    pub nominal_colors: Vec<Option<Rgb>>,
    pub assignments: Vec<TextureColorAssignment>,
}

impl TextureSlotMapping {
    pub fn validate(&self) -> Result<(), String> {
        let resolved = resolve_slots(&self.selection, &self.nominal_colors)?;
        assignment_index(&self.assignments, resolved.filaments.len())?;
        Ok(())
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TextureMappingRow {
    pub rgb: Rgb,
    pub face_count: usize,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TextureMappingPreview {
    pub revision: String,
    pub rows: Vec<TextureMappingRow>,
    pub accepted: Option<TextureSlotMapping>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub grouping: Option<TextureColorGrouping>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TextureColorGrouping {
    pub source_color_count: usize,
    /// Zero-based index into the export rows for each face in mesh order.
    pub face_rows: Vec<u8>,
}

// Another export can replace the accepted anchors without touching geometry or paint.
fn revision(mesh: &MeshModel) -> String {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    mesh.vertices.len().hash(&mut hasher);
    for vertex in &mesh.vertices {
        vertex.map(f32::to_bits).hash(&mut hasher);
    }
    mesh.faces.hash(&mut hasher);
    mesh.face_colors.hash(&mut hasher);
    mesh.texture_slot_mapping
        .as_ref()
        .map(|mapping| &mapping.assignments)
        .hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

fn check_texture_mesh(mesh: &MeshModel) -> Result<(), String> {
    if !mesh.texture_painting {
        return Err("The current model is not a texture project".into());
    }
    if mesh.faces.is_empty() || mesh.face_colors.len() != mesh.faces.len() {
        return Err("The texture project must have one colour per face".into());
    }
    Ok(())
}

fn rgb([r, g, b, _]: [u8; 4]) -> Rgb {
    [r, g, b]
}

pub fn preview(mesh: &MeshModel) -> Result<TextureMappingPreview, String> {
    check_texture_mesh(mesh)?;
    let (rows, grouping) = export_rows(mesh)?;
    let mut accepted = mesh.texture_slot_mapping.clone();
    if let Some(accepted) = &mut accepted {
        accepted
            .assignments
            .retain(|assignment| rows.iter().any(|row| row.rgb == assignment.rgb));
    }
    Ok(TextureMappingPreview {
        revision: revision(mesh),
        rows,
        accepted,
        grouping,
    })
}

fn export_rows(
    mesh: &MeshModel,
) -> Result<(Vec<TextureMappingRow>, Option<TextureColorGrouping>), String> {
    let exceeds_grouping_budget = mesh.faces.len() > MAX_FACES;
    let mut tally = BTreeMap::new();
    for &face in &mesh.face_colors {
        *tally.entry(rgb(face)).or_insert(0) += 1;
        if exceeds_grouping_budget && tally.len() > MAX_EXPORT_ROWS {
            return Err(format!(
                "Grouped texture export supports up to {MAX_FACES} triangles"
            ));
        }
    }
    if tally.len() <= MAX_EXPORT_ROWS {
        return Ok((
            tally
                .into_iter()
                .map(|(rgb, face_count)| TextureMappingRow { rgb, face_count })
                .collect(),
            None,
        ));
    }
    // Reuse confirmed RGBs still present in the model; recent rows precede history.
    let mut representatives: BTreeSet<Rgb> = mesh
        .texture_slot_mapping
        .iter()
        .flat_map(|mapping| &mapping.assignments)
        .map(|assignment| assignment.rgb)
        .filter(|color| tally.contains_key(color))
        .take(MAX_EXPORT_ROWS)
        .collect();
    if representatives.len() < MAX_EXPORT_ROWS {
        let unassigned_faces: Vec<_> = mesh
            .face_colors
            .iter()
            .copied()
            .filter(|&face| !representatives.contains(&rgb(face)))
            .collect();
        let quantized =
            quantize_face_colors(&unassigned_faces, MAX_EXPORT_ROWS - representatives.len());
        // Use a distinct source RGB near each centroid so reopening preserves its anchor.
        for center in quantized.palette {
            if let Some(&nearest) = tally
                .keys()
                .filter(|color| !representatives.contains(*color))
                .min_by_key(|&&color| (distance_squared(center, color), color))
            {
                representatives.insert(nearest);
            }
        }
    }

    let mut rows: Vec<_> = representatives
        .into_iter()
        .map(|rgb| TextureMappingRow { rgb, face_count: 0 })
        .collect();
    let row_by_color: BTreeMap<_, _> = tally
        .into_iter()
        .map(|(color, count)| {
            let (row_index, row) = rows
                .iter_mut()
                .enumerate()
                .min_by_key(|(_, row)| (distance_squared(color, row.rgb), row.rgb))
                .expect("grouped colours have representatives");
            row.face_count += count;
            (color, row_index as u8)
        })
        .collect();
    let face_rows = mesh
        .face_colors
        .iter()
        .map(|&face| row_by_color[&rgb(face)])
        .collect();
    Ok((
        rows,
        Some(TextureColorGrouping {
            source_color_count: row_by_color.len(),
            face_rows,
        }),
    ))
}

pub struct FilamentCandidate<'a> {
    pub slot: u8,
    pub filament: &'a FilamentPreset,
    pub nominal_color: Option<Rgb>,
}

pub struct FilamentPrediction {
    pub slot: u8,
    pub rgb: Option<Rgb>,
}

/// Predictions are advisory. Only explicit assignments enter the archive.
pub trait FilamentColorPredictor {
    fn predict(
        &self,
        slots: &[FilamentCandidate<'_>],
        process: Option<&ProcessPreset>,
    ) -> Vec<FilamentPrediction>;
}

pub struct NominalColorPredictor;

impl FilamentColorPredictor for NominalColorPredictor {
    fn predict(
        &self,
        slots: &[FilamentCandidate<'_>],
        _: Option<&ProcessPreset>,
    ) -> Vec<FilamentPrediction> {
        slots
            .iter()
            .map(|candidate| FilamentPrediction {
                slot: candidate.slot,
                rgb: nominal_color(candidate),
            })
            .collect()
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TextureSlotPreview {
    pub slot: u8,
    pub nominal_color: Option<Rgb>,
    pub display_color: Rgb,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TextureMappingSuggestions {
    pub slots: Vec<TextureSlotPreview>,
    pub assignments: Vec<TextureColorAssignment>,
}

pub fn suggestions(
    selection: &ExportSelection,
    nominal_colors: &[Option<Rgb>],
    colors: &[Rgb],
    predictor: &dyn FilamentColorPredictor,
) -> Result<TextureMappingSuggestions, String> {
    let resolved = resolve_slots(selection, nominal_colors)?;
    let candidates = candidates(&resolved, nominal_colors);
    let estimates: BTreeMap<u8, Rgb> = predictor
        .predict(&candidates, Some(resolved.process))
        .into_iter()
        .filter(|prediction| (1..=candidates.len()).contains(&usize::from(prediction.slot)))
        .filter_map(|prediction| Some((prediction.slot, prediction.rgb?)))
        .collect();
    let assignments = colors
        .iter()
        .filter_map(|&rgb| {
            estimates
                .iter()
                .min_by_key(|(_, estimated)| distance_squared(rgb, **estimated))
                .map(|(&slot, _)| TextureColorAssignment { rgb, slot })
        })
        .collect();
    Ok(TextureMappingSuggestions {
        slots: slot_previews(&candidates),
        assignments,
    })
}

fn distance_squared(a: Rgb, b: Rgb) -> i32 {
    a.into_iter()
        .zip(b)
        .map(|(x, y)| (i32::from(x) - i32::from(y)).pow(2))
        .sum()
}

/// A single export plan supplies both triangle slots and ordered spool settings.
pub struct TextureExportSnapshot {
    pub mapping: TextureSlotMapping,
    pub(crate) quantized: Quantized,
    pub(crate) resolved: ResolvedSelection<'static>,
}

impl TextureExportSnapshot {
    pub fn build(
        mesh: &MeshModel,
        expected_revision: &str,
        mapping: &TextureSlotMapping,
    ) -> Result<Self, String> {
        check_texture_mesh(mesh)?;
        if revision(mesh) != expected_revision {
            return Err(
                "The texture project changed. Reopen export to confirm the new colours.".into(),
            );
        }

        let resolved = resolve_slots(&mapping.selection, &mapping.nominal_colors)?;
        let palette = slot_previews(&candidates(&resolved, &mapping.nominal_colors))
            .into_iter()
            .map(|preview| preview.display_color)
            .collect();
        let (rows, grouping) = export_rows(mesh)?;
        let quantized = match &grouping {
            Some(grouping) => materialize_rows(
                grouping
                    .face_rows
                    .iter()
                    .map(|&row| rows[usize::from(row)].rgb),
                palette,
                &mapping.assignments,
            )?,
            None => materialize_plan(&mesh.face_colors, palette, &mapping.assignments)?,
        };

        let mut mapping = mapping.clone();
        let slot_count = resolved.filaments.len();
        mapping.nominal_colors.resize(slot_count, None);
        mapping.selection.filament_names = resolved
            .filaments
            .iter()
            .map(|filament| filament.name.clone())
            .collect();

        let confirmed: HashMap<_, _> = mapping
            .assignments
            .iter()
            .map(|assignment| (assignment.rgb, assignment.slot))
            .collect();
        let mut accepted: Vec<_> = rows
            .iter()
            .map(|row| TextureColorAssignment {
                rgb: row.rgb,
                slot: confirmed[&row.rgb],
            })
            .collect();
        if let Some(previous) = mesh
            .texture_slot_mapping
            .as_ref()
            .filter(|previous| previous.selection.machine_id == mapping.selection.machine_id)
        {
            // Keep unused assignments for undo, bounded independently of edit/export count.
            accepted.extend(
                previous
                    .assignments
                    .iter()
                    .filter(|assignment| {
                        (1..=slot_count).contains(&usize::from(assignment.slot))
                            && !rows.iter().any(|row| row.rgb == assignment.rgb)
                    })
                    .take(MAX_INACTIVE_ASSIGNMENTS)
                    .cloned(),
            );
        }
        mapping.assignments = accepted;

        Ok(Self {
            mapping,
            quantized,
            resolved,
        })
    }
}

fn resolve_slots(
    selection: &ExportSelection,
    nominal_colors: &[Option<Rgb>],
) -> Result<ResolvedSelection<'static>, String> {
    let machine = library()
        .iter()
        .find(|machine| machine.id == selection.machine_id)
        .ok_or_else(|| format!("Unknown machine preset {:?}", selection.machine_id))?;
    let slot_count = machine
        .extruder_count(&selection.nozzle_diameter)
        .min(usize::from(MAX_EXTRUDER_SLOT));
    if nominal_colors.len() > slot_count || selection.filament_names.len() > slot_count {
        return Err(format!(
            "{} has {slot_count} available filament slots",
            machine.name
        ));
    }
    let resolved = selection.resolve(slot_count)?;
    if !resolved
        .process
        .nozzles
        .contains(&selection.nozzle_diameter)
    {
        return Err(format!(
            "Process {:?} is incompatible with the selected nozzle",
            resolved.process.name
        ));
    }
    for filament in &resolved.filaments {
        if !filament.nozzles.contains(&selection.nozzle_diameter) {
            return Err(format!(
                "Filament {:?} is incompatible with the selected nozzle",
                filament.name
            ));
        }
    }
    if !machine.model_ids.contains_key(&selection.target_slicer) {
        return Err(format!(
            "Unknown target slicer {:?}",
            selection.target_slicer
        ));
    }
    Ok(resolved)
}

fn candidates<'a>(
    selection: &ResolvedSelection<'a>,
    nominal_colors: &[Option<Rgb>],
) -> Vec<FilamentCandidate<'a>> {
    selection
        .filaments
        .iter()
        .enumerate()
        .map(|(i, &filament)| FilamentCandidate {
            slot: (i + 1) as u8,
            filament,
            nominal_color: nominal_colors.get(i).copied().flatten(),
        })
        .collect()
}

fn nominal_color(candidate: &FilamentCandidate<'_>) -> Option<Rgb> {
    candidate.nominal_color.or_else(|| {
        let value = candidate.filament.config.get("filament_colour")?;
        let hex = value
            .as_str()
            .or_else(|| value.as_array()?.first()?.as_str())?;
        let digits = hex.strip_prefix('#')?;
        if digits.len() != 6 || !digits.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return None;
        }
        let color = u32::from_str_radix(digits, 16).ok()?;
        let [_, r, g, b] = color.to_be_bytes();
        Some([r, g, b])
    })
}

fn slot_previews(candidates: &[FilamentCandidate<'_>]) -> Vec<TextureSlotPreview> {
    candidates
        .iter()
        .map(|candidate| {
            let nominal_color = nominal_color(candidate);
            TextureSlotPreview {
                slot: candidate.slot,
                nominal_color,
                display_color: nominal_color.unwrap_or_else(|| rgb(DEFAULT_FACE_COLOR)),
            }
        })
        .collect()
}

fn assignment_index(
    assignments: &[TextureColorAssignment],
    slot_count: usize,
) -> Result<HashMap<Rgb, u8>, String> {
    let mut by_color = HashMap::with_capacity(assignments.len());
    for &TextureColorAssignment { rgb, slot } in assignments {
        if !(1..=slot_count).contains(&usize::from(slot)) {
            return Err(format!(
                "Filament slot {slot} is outside the available range 1–{slot_count}"
            ));
        }
        if by_color.insert(rgb, slot).is_some() {
            return Err(format!(
                "Model colour {} has multiple assignments",
                hex(rgb)
            ));
        }
    }
    Ok(by_color)
}

pub(crate) fn materialize_plan(
    face_colors: &[[u8; 4]],
    palette: Vec<Rgb>,
    assignments: &[TextureColorAssignment],
) -> Result<Quantized, String> {
    materialize_rows(face_colors.iter().copied().map(rgb), palette, assignments)
}

fn materialize_rows(
    face_colors: impl Iterator<Item = Rgb>,
    palette: Vec<Rgb>,
    assignments: &[TextureColorAssignment],
) -> Result<Quantized, String> {
    if palette.is_empty() || palette.len() > usize::from(MAX_EXTRUDER_SLOT) {
        return Err(format!(
            "The 3MF format supports 1–{MAX_EXTRUDER_SLOT} filament slots"
        ));
    }
    let by_color = assignment_index(assignments, palette.len())?;
    let face_slots = face_colors
        .map(|color| {
            by_color
                .get(&color)
                .copied()
                .ok_or_else(|| format!("Choose a filament slot for model colour {}", hex(color)))
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(Quantized {
        palette,
        face_slots,
    })
}

fn hex([r, g, b]: Rgb) -> String {
    format!("#{r:02X}{g:02X}{b:02X}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::export::presets::ExportSelection;

    const RED: [u8; 3] = [255, 0, 0];
    const BLUE: [u8; 3] = [0, 0, 255];
    const ORANGE: [u8; 3] = [255, 120, 0];

    fn u1() -> ExportSelection {
        ExportSelection {
            machine_id: "snapmaker_u1".into(),
            nozzle_diameter: "0.4".into(),
            process_name: "0.20 Standard @Snapmaker U1 (0.4 nozzle)".into(),
            filament_names: vec!["Generic PLA".into()],
            target_slicer: "snapmaker_orca".into(),
        }
    }

    fn tetrahedron(face_colors: Vec<[u8; 4]>) -> MeshModel {
        let mut mesh = MeshModel::new();
        mesh.texture_painting = true;
        mesh.vertices = vec![[0., 0., 0.], [1., 0., 0.], [0., 1., 0.], [0., 0., 1.]];
        mesh.faces = vec![[0, 2, 1], [0, 1, 3], [0, 3, 2], [1, 2, 3]];
        mesh.face_colors = face_colors;
        mesh
    }

    #[test]
    fn blended_project_has_sixteen_export_colours_and_saves_its_original_faces() {
        let face_colors: Vec<_> = (0..4096)
            .map(|i| [(i >> 8) as u8, i as u8, 125, 255])
            .collect();
        let mut mesh = tetrahedron(face_colors);
        mesh.faces.resize(mesh.face_colors.len(), [0, 1, 2]);
        mesh.segment_labels.resize(mesh.faces.len(), 0);
        let original = mesh.face_colors.clone();

        let displayed = preview(&mesh).unwrap();
        assert_eq!(displayed.rows.len(), 16);
        assert_eq!(
            displayed
                .rows
                .iter()
                .map(|row| row.face_count)
                .sum::<usize>(),
            original.len()
        );
        let mapping = TextureSlotMapping {
            selection: u1(),
            nominal_colors: vec![
                Some([255, 0, 0]),
                Some([0, 255, 0]),
                Some([0, 0, 255]),
                None,
            ],
            assignments: displayed
                .rows
                .iter()
                .enumerate()
                .map(|(i, row)| TextureColorAssignment {
                    rgb: row.rgb,
                    slot: (i % 4 + 1) as u8,
                })
                .collect(),
        };
        let exported = TextureExportSnapshot::build(&mesh, &displayed.revision, &mapping).unwrap();
        assert_eq!(exported.mapping.assignments.len(), 16);
        let displayed_groups = displayed.grouping.as_ref().unwrap();
        assert_eq!(displayed_groups.source_color_count, original.len());
        assert_eq!(displayed_groups.face_rows.len(), original.len());
        let preview_slots: Vec<_> = displayed_groups
            .face_rows
            .iter()
            .map(|&row| mapping.assignments[usize::from(row)].slot)
            .collect();
        assert_eq!(exported.quantized.face_slots, preview_slots);
        let project =
            crate::engineering::encode_project_parts(&mesh, Some(&exported.mapping)).unwrap();
        let texture_part = project
            .iter()
            .find(|(name, _)| *name == "Metadata/cym/texture_1.json")
            .unwrap();
        assert!(texture_part.1.len() < 2048);
        let entries: Vec<_> = project
            .iter()
            .map(|(name, bytes)| (*name, bytes.as_slice()))
            .collect();
        let dir = tempfile::tempdir().unwrap();
        let output = dir.path().join("blended.3mf");
        crate::export::threemf::export_3mf_with_project(
            &mesh,
            &output,
            None,
            Some(&exported),
            &entries,
            &|_, _| {},
        )
        .unwrap();
        let mut archive = zip::ZipArchive::new(std::fs::File::open(&output).unwrap()).unwrap();
        let mut xml = String::new();
        std::io::Read::read_to_string(&mut archive.by_name("3D/3dmodel.model").unwrap(), &mut xml)
            .unwrap();
        let paint = xml
            .match_indices("paint_color=\"")
            .map(|(i, key)| xml[i + key.len()..].split('"').next().unwrap());
        let expected_paint: Vec<_> = preview_slots
            .into_iter()
            .map(|slot| super::super::paint_color::encode_paint_color(slot).unwrap())
            .collect();
        assert_eq!(paint.collect::<Vec<_>>(), expected_paint);

        let (reopened, warning) = crate::engineering::open_project(&output, &|_, _| {}).unwrap();
        assert_eq!(warning, None);
        assert_eq!(reopened.face_colors, original);
        let reopened_preview = preview(&reopened).unwrap();
        assert_eq!(
            reopened_preview
                .rows
                .iter()
                .map(|row| row.rgb)
                .collect::<Vec<_>>(),
            displayed.rows.iter().map(|row| row.rgb).collect::<Vec<_>>(),
        );
        assert_eq!(
            reopened_preview.accepted.unwrap().assignments,
            mapping.assignments
        );
    }

    #[test]
    fn active_rgb_anchors_survive_dominance_and_recolouring_at_capacity() {
        let anchors: Vec<_> = (0..16u8).map(|i| [i * 16, 0, 0, 255]).collect();
        let mut face_colors = anchors.clone();
        face_colors.resize(64, anchors[15]);
        let mut mesh = tetrahedron(face_colors);
        mesh.faces.resize(mesh.face_colors.len(), [0, 1, 2]);
        let exact = preview(&mesh).unwrap();
        assert!(exact.grouping.is_none());
        let mapping = TextureSlotMapping {
            selection: u1(),
            nominal_colors: vec![None; 4],
            assignments: exact
                .rows
                .iter()
                .enumerate()
                .map(|(i, row)| TextureColorAssignment {
                    rgb: row.rgb,
                    slot: (i % 4 + 1) as u8,
                })
                .collect(),
        };
        let first = TextureExportSnapshot::build(&mesh, &exact.revision, &mapping).unwrap();
        mesh.texture_slot_mapping = Some(first.mapping);
        mesh.face_colors[16..].fill([21, 2, 1, 255]);
        mesh.face_colors[63] = [250, 13, 105, 255];
        let regrouped = preview(&mesh).unwrap();
        assert_eq!(
            regrouped.accepted.as_ref().unwrap().assignments,
            mapping.assignments
        );
        let nearest_row = usize::from(regrouped.grouping.as_ref().unwrap().face_rows[16]);
        assert_eq!(regrouped.rows[nearest_row].rgb, rgb(anchors[1]));
        let second = TextureExportSnapshot::build(&mesh, &regrouped.revision, &mapping).unwrap();
        assert_eq!(
            &second.quantized.face_slots[0..16],
            &first.quantized.face_slots[0..16]
        );
        assert_eq!(second.quantized.face_slots[16], mapping.assignments[1].slot);

        mesh.texture_slot_mapping = Some(second.mapping);
        mesh.face_colors[8] = [128, 2, 1, 255];
        let recoloured = preview(&mesh).unwrap();
        assert_eq!(recoloured.rows.len(), 16);
        let preserved: Vec<_> = mapping
            .assignments
            .into_iter()
            .filter(|assignment| assignment.rgb != rgb(anchors[8]))
            .collect();
        assert_eq!(recoloured.accepted.unwrap().assignments, preserved);
    }

    #[test]
    fn inactive_assignments_keep_the_previous_palette_with_a_fixed_cap() {
        let mut mesh = tetrahedron(vec![]);
        mesh.faces.resize(16, [0, 1, 2]);
        for generation in 0..8 {
            mesh.face_colors = (0..16).map(|i| [generation, i, 0, 255]).collect();
            let current = preview(&mesh).unwrap();
            let mapping = TextureSlotMapping {
                selection: u1(),
                nominal_colors: vec![None; 4],
                assignments: current
                    .rows
                    .iter()
                    .map(|row| TextureColorAssignment {
                        rgb: row.rgb,
                        slot: generation % 4 + 1,
                    })
                    .collect(),
            };
            mesh.texture_slot_mapping = Some(
                TextureExportSnapshot::build(&mesh, &current.revision, &mapping)
                    .unwrap()
                    .mapping,
            );
        }
        let saved = mesh.texture_slot_mapping.as_ref().unwrap();
        assert_eq!(saved.assignments.len(), 32);
        assert!(saved
            .assignments
            .iter()
            .all(|assignment| assignment.rgb[0] >= 6));
        mesh.face_colors.iter_mut().for_each(|color| color[0] = 6);
        let undone = preview(&mesh).unwrap();
        let restored = undone.accepted.unwrap().assignments;
        assert_eq!(restored.len(), 16);
        assert!(restored.iter().all(|assignment| assignment.slot == 3));
    }

    #[test]
    fn grouped_preview_has_a_bounded_per_face_payload() {
        let mut mesh = tetrahedron(
            (0..MAX_FACES)
                .map(|i| [(i % 17) as u8, 0, 0, 255])
                .collect(),
        );
        mesh.faces.resize(MAX_FACES, [0, 1, 2]);
        let grouped = preview(&mesh).unwrap().grouping.unwrap();
        assert_eq!(grouped.face_rows.len(), MAX_FACES);

        mesh.faces.push([0, 1, 2]);
        mesh.face_colors.push([0, 0, 0, 255]);
        assert!(preview(&mesh)
            .err()
            .unwrap()
            .contains(&MAX_FACES.to_string()));

        mesh.face_colors.fill([0, 0, 0, 255]);
        let exact = serde_json::to_value(preview(&mesh).unwrap()).unwrap();
        assert!(exact.get("grouping").is_none());
    }

    #[test]
    fn dominant_colour_changes_leave_accepted_slots_intact() {
        let mut mesh = tetrahedron(vec![
            [255, 0, 0, 255],
            [255, 0, 0, 255],
            [255, 0, 0, 255],
            [0, 0, 255, 255],
        ]);
        let mapping = TextureSlotMapping {
            selection: u1(),
            nominal_colors: vec![None, Some(BLUE), None, Some(RED)],
            assignments: vec![
                TextureColorAssignment { rgb: RED, slot: 4 },
                TextureColorAssignment { rgb: BLUE, slot: 2 },
            ],
        };
        let first =
            TextureExportSnapshot::build(&mesh, &preview(&mesh).unwrap().revision, &mapping)
                .unwrap();
        assert_eq!(first.quantized.face_slots, [4, 4, 4, 2]);

        mesh.texture_slot_mapping = Some(first.mapping);
        mesh.face_colors[1..3].fill([0, 0, 255, 255]);
        let changed = preview(&mesh).unwrap();
        let second = TextureExportSnapshot::build(
            &mesh,
            &changed.revision,
            changed.accepted.as_ref().unwrap(),
        )
        .unwrap();
        assert_eq!(second.quantized.face_slots, [4, 2, 2, 2]);
        assert_eq!(
            second.quantized.palette,
            [[138, 138, 138], BLUE, [138, 138, 138], RED]
        );
    }

    #[test]
    fn confirmation_rejects_stale_snapshot_and_requires_new_colours() {
        let mut mesh = tetrahedron(vec![[255, 0, 0, 255]; 4]);
        let mapping = TextureSlotMapping {
            selection: u1(),
            nominal_colors: vec![None; 4],
            assignments: vec![TextureColorAssignment { rgb: RED, slot: 1 }],
        };
        let before = preview(&mesh).unwrap();
        mesh.face_colors[0] = [255, 120, 0, 255];
        assert!(
            TextureExportSnapshot::build(&mesh, &before.revision, &mapping)
                .err()
                .unwrap()
                .contains("changed")
        );
        let after = preview(&mesh).unwrap();
        assert!(
            TextureExportSnapshot::build(&mesh, &after.revision, &mapping)
                .err()
                .unwrap()
                .contains("#FF7800")
        );
    }

    #[test]
    fn u1_rejects_a_valid_format_slot_when_physical_slot_is_unavailable() {
        let mesh = tetrahedron(vec![[255, 0, 0, 255]; 4]);
        let mapping = TextureSlotMapping {
            selection: u1(),
            nominal_colors: vec![],
            assignments: vec![TextureColorAssignment { rgb: RED, slot: 5 }],
        };
        assert!(
            TextureExportSnapshot::build(&mesh, &preview(&mesh).unwrap().revision, &mapping)
                .err()
                .unwrap()
                .contains("1–4")
        );
    }

    #[test]
    fn predictor_can_change_suggestions_without_altering_confirmed_slots() {
        struct SwappedColours;
        impl FilamentColorPredictor for SwappedColours {
            fn predict(
                &self,
                slots: &[FilamentCandidate<'_>],
                _: Option<&ProcessPreset>,
            ) -> Vec<FilamentPrediction> {
                slots
                    .iter()
                    .rev()
                    .map(|slot| FilamentPrediction {
                        slot: slot.slot,
                        rgb: match slot.slot {
                            1 => Some(BLUE),
                            4 => Some(RED),
                            _ => None,
                        },
                    })
                    .collect()
            }
        }
        let nominal_colors = [Some(RED), None, None, Some(BLUE)];
        let nominal =
            suggestions(&u1(), &nominal_colors, &[RED, BLUE], &NominalColorPredictor).unwrap();
        let swapped = suggestions(&u1(), &nominal_colors, &[RED, BLUE], &SwappedColours).unwrap();
        assert_eq!(
            nominal.assignments,
            [
                TextureColorAssignment { rgb: RED, slot: 1 },
                TextureColorAssignment { rgb: BLUE, slot: 4 },
            ]
        );
        assert_eq!(
            swapped.assignments,
            [
                TextureColorAssignment { rgb: RED, slot: 4 },
                TextureColorAssignment { rgb: BLUE, slot: 1 },
            ]
        );
        let mesh = tetrahedron(vec![[255, 0, 0, 255]; 4]);
        let mapping = TextureSlotMapping {
            selection: u1(),
            nominal_colors: nominal_colors.to_vec(),
            assignments: vec![TextureColorAssignment { rgb: RED, slot: 1 }],
        };
        let snapshot =
            TextureExportSnapshot::build(&mesh, &preview(&mesh).unwrap().revision, &mapping)
                .unwrap();
        assert_eq!(snapshot.quantized.face_slots, [1; 4]);
    }

    #[test]
    fn unknown_spools_need_manual_assignments_but_can_be_exported() {
        assert!(
            suggestions(&u1(), &[None; 4], &[ORANGE], &NominalColorPredictor)
                .unwrap()
                .assignments
                .is_empty()
        );
        let mesh = tetrahedron(vec![[255, 120, 0, 255]; 4]);
        let mapping = TextureSlotMapping {
            selection: u1(),
            nominal_colors: vec![None; 4],
            assignments: vec![TextureColorAssignment {
                rgb: ORANGE,
                slot: 3,
            }],
        };
        let snapshot =
            TextureExportSnapshot::build(&mesh, &preview(&mesh).unwrap().revision, &mapping)
                .unwrap();
        assert_eq!(snapshot.quantized.face_slots, [3; 4]);
    }
}
