use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::export::project_state::{self, Fingerprint, PlateStateHeader, ProjectState};
use crate::export::texture_mapping::TextureSlotMapping;
use crate::mesh::loader::ProgressFn;
use crate::mesh::model::MeshModel;
use crate::threemf::{self, opc::Package, EngineeringState};

const HEADER_PART: &str = "Metadata/cym/project.json";
const PLATE_PART: &str = "Metadata/cym/plate_1.bin";
const HISTORY_PART: &str = "Metadata/cym/history_1.bin";
const TEXTURE_PART: &str = "Metadata/cym/texture_1.json";
const MAX_TEXTURE_STATE_BYTES: u64 = 64 * 1024;

#[derive(Serialize, Deserialize)]
struct TextureState {
    format_version: u8,
    fingerprint: Fingerprint,
    mapping: Option<TextureSlotMapping>,
    #[serde(default = "has_segments_by_default")]
    has_segments: bool,
}

fn has_segments_by_default() -> bool {
    true
}

pub(crate) fn encode_project_parts(
    mesh: &MeshModel,
    mapping: Option<&TextureSlotMapping>,
) -> Result<Vec<(&'static str, Vec<u8>)>, String> {
    if mesh.face_colors.len() != mesh.faces.len() || mesh.segment_labels.len() != mesh.faces.len() {
        return Err("Cannot save a project with incomplete per-face data".into());
    }
    let fingerprint = project_state::fingerprint_of(&mesh.vertices, &mesh.faces);
    let header = ProjectState {
        unit: mesh.unit.clone(),
        active_plate: 1,
        plates: vec![PlateStateHeader {
            plater_id: 1,
            name: "plate 1".into(),
            object_ids: vec![1],
            fingerprint,
        }],
    };
    let texture_bytes = serde_json::to_vec(&TextureState {
        format_version: 1,
        fingerprint,
        mapping: mapping.cloned(),
        has_segments: !mesh.segments.is_empty(),
    })
    .map_err(|error| format!("Could not save texture assignments: {error}"))?;
    if texture_bytes.len() as u64 > MAX_TEXTURE_STATE_BYTES {
        return Err("The texture assignments are too large for a project".into());
    }
    Ok(vec![
        (
            HEADER_PART,
            project_state::encode_project_json(&header).into_bytes(),
        ),
        (PLATE_PART, project_state::encode_plate_state(mesh)),
        (HISTORY_PART, project_state::encode_history(&mesh.history)),
        (TEXTURE_PART, texture_bytes),
    ])
}

pub(crate) fn restore_texture_state(
    package: &mut Package,
    plate_index: usize,
    fingerprint: Fingerprint,
    mesh: &mut MeshModel,
) -> Result<bool, String> {
    let part = format!("Metadata/cym/texture_{}.json", plate_index + 1);
    if !package.part_names().iter().any(|name| name == &part) {
        return Ok(true);
    }
    let bytes = package.read_at_most(&part, MAX_TEXTURE_STATE_BYTES)?;
    let saved: TextureState = serde_json::from_slice(&bytes)
        .map_err(|error| format!("Unreadable texture project state: {error}"))?;
    if saved.format_version != 1 || saved.fingerprint != fingerprint {
        return Err("Texture state version or geometry does not match this 3MF".into());
    }
    mesh.texture_painting = true;
    if let Some(mapping) = &saved.mapping {
        mapping.validate()?;
    }
    mesh.texture_slot_mapping = saved.mapping;
    Ok(saved.has_segments)
}

pub(crate) fn open_project(
    path: &Path,
    progress: &ProgressFn,
) -> Result<(MeshModel, Option<String>), String> {
    let plate = threemf::read_single_plate(path, progress)?;
    let warning = if plate.engineering_state == EngineeringState::Dropped {
        Some("3mf-state")
    } else if plate.texture_state_dropped {
        Some("3mf-texture")
    } else if plate.history_dropped {
        Some("3mf-history")
    } else {
        None
    };
    Ok((plate.mesh, warning.map(String::from)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::export::presets::ExportSelection;
    use crate::export::texture_mapping::{self, TextureColorAssignment, TextureExportSnapshot};
    use crate::mesh::history::OpKind;
    use crate::threemf::test_support::{mesh_object, model_xml, Fixture};

    #[test]
    fn texture_project_reopens_exact_face_colours_and_undo_without_glb() {
        let mut mesh = MeshModel::new();
        mesh.texture_painting = true;
        mesh.vertices = vec![[0., 0., 0.], [20., 0., 0.], [0., 20., 0.]];
        mesh.faces = vec![[0, 1, 2]];
        let original = [201, 63, 25, 255];
        let edited = [48, 91, 177, 255];
        mesh.face_colors = vec![edited];
        mesh.segment_labels = vec![0];
        mesh.history
            .record(OpKind::Paint, None, &[(0, original)], &[]);
        let mapping = TextureSlotMapping {
            selection: ExportSelection {
                machine_id: "snapmaker_u1".into(),
                nozzle_diameter: "0.4".into(),
                process_name: "0.20 Standard @Snapmaker U1 (0.4 nozzle)".into(),
                filament_names: vec!["Generic PLA".into()],
                target_slicer: "snapmaker_orca".into(),
            },
            nominal_colors: vec![Some([48, 91, 177])],
            assignments: vec![TextureColorAssignment {
                rgb: [48, 91, 177],
                slot: 1,
            }],
        };
        let mut entries = encode_project_parts(&mesh, Some(&mapping)).unwrap();
        entries.push((
            "3D/3dmodel.model",
            model_xml(
                "millimeter",
                &mesh_object(1, "Test", &mesh.vertices, &mesh.faces, &[]),
                r#"<item objectid="1"/>"#,
            ),
        ));
        let zip = Fixture::new(&entries);
        let (mut opened, warning) = open_project(zip.path(), &|_, _| {}).unwrap();
        assert_eq!(warning, None);
        assert!(opened.texture_painting);
        assert_eq!(opened.faces, mesh.faces);
        assert!(
            opened.segments.is_empty(),
            "a pristine texture remains unsegmented"
        );
        assert_eq!(opened.face_colors, [edited]);
        let reopened_mapping = opened.texture_slot_mapping.unwrap();
        assert_eq!(reopened_mapping.selection.machine_id, "snapmaker_u1");
        assert_eq!(reopened_mapping.nominal_colors, mapping.nominal_colors);
        assert_eq!(reopened_mapping.assignments, mapping.assignments);
        assert!(opened.history.can_undo());
        opened
            .history
            .undo(&mut opened.face_colors, &mut opened.segment_labels);
        assert_eq!(opened.face_colors, [original]);
        assert!(opened.history.can_redo());
    }

    #[test]
    fn stale_texture_mapping_cannot_attach_to_different_geometry() {
        let mut mesh = MeshModel::new();
        mesh.texture_painting = true;
        mesh.vertices = vec![[0., 0., 0.], [20., 0., 0.], [0., 20., 0.]];
        mesh.faces = vec![[0, 1, 2]];
        mesh.face_colors = vec![[48, 91, 177, 255]];
        mesh.segment_labels = vec![0];
        let mut entries = encode_project_parts(&mesh, None).unwrap();
        mesh.vertices[1][0] = 40.;
        entries.push((
            "3D/3dmodel.model",
            model_xml(
                "millimeter",
                &mesh_object(1, "Test", &mesh.vertices, &mesh.faces, &[]),
                r#"<item objectid="1"/>"#,
            ),
        ));
        let zip = Fixture::new(&entries);
        let (opened, warning) = open_project(zip.path(), &|_, _| {}).unwrap();
        assert!(warning.is_some());
        assert!(!opened.texture_painting);
        assert!(opened.texture_slot_mapping.is_none());
    }

    #[test]
    #[ignore = "requires a generated GLB and an explicit output path"]
    fn export_texture_acceptance_fixture() {
        let input = std::env::var_os("CYM_TEXTURE_INPUT").expect("set CYM_TEXTURE_INPUT");
        let output = std::env::var_os("CYM_TEXTURE_OUTPUT").expect("set CYM_TEXTURE_OUTPUT");
        let baked = crate::mesh::texture::load_texture(
            Path::new(&input),
            16,
            &std::sync::atomic::AtomicBool::new(false),
        )
        .unwrap();
        let mut mesh = MeshModel::new();
        mesh.vertices = baked.vertices;
        mesh.faces = baked.faces;
        mesh.face_colors = baked.face_colors;
        mesh.segment_labels = vec![0; mesh.faces.len()];
        mesh.texture_painting = true;
        let preview = texture_mapping::preview(&mesh).unwrap();
        let side_fixture = std::env::var_os("CYM_TEXTURE_SIDES_BY_SLOT").is_some();
        assert_eq!(preview.rows.len(), 16);
        let side_colors = [
            [
                [238, 223, 192],
                [215, 60, 38],
                [249, 154, 39],
                [246, 211, 91],
            ],
            [[166, 104, 67], [130, 63, 99], [71, 97, 137], [25, 58, 95]],
            [
                [19, 122, 138],
                [124, 182, 162],
                [46, 143, 83],
                [147, 180, 64],
            ],
            [
                [191, 221, 227],
                [237, 151, 168],
                [85, 78, 171],
                [46, 50, 54],
            ],
        ];
        let mapping = TextureSlotMapping {
            selection: ExportSelection {
                machine_id: "snapmaker_u1".into(),
                nozzle_diameter: "0.4".into(),
                process_name: "0.20 Standard @Snapmaker U1 (0.4 nozzle)".into(),
                filament_names: vec!["Generic PLA".into(); 4],
                target_slicer: "snapmaker_orca".into(),
            },
            nominal_colors: side_colors.iter().map(|colors| Some(colors[0])).collect(),
            // Deliberately distribute all 16 editable colours across four
            // real tools. This exercises a non-default many-to-one mapping.
            assignments: preview
                .rows
                .iter()
                .enumerate()
                .map(|(index, row)| TextureColorAssignment {
                    rgb: row.rgb,
                    slot: if side_fixture {
                        side_colors
                            .iter()
                            .position(|colors| colors.contains(&row.rgb))
                            .expect("each wall must use its unblended atlas colours")
                            as u8
                            + 1
                    } else {
                        (index % 4 + 1) as u8
                    },
                })
                .collect(),
        };
        let snapshot = TextureExportSnapshot::build(&mesh, &preview.revision, &mapping).unwrap();
        let parts = encode_project_parts(&mesh, Some(&snapshot.mapping)).unwrap();
        let entries: Vec<_> = parts
            .iter()
            .map(|(name, bytes)| (*name, bytes.as_slice()))
            .collect();
        let output = Path::new(&output);
        crate::export::threemf::export_3mf_with_project(
            &mesh,
            output,
            None,
            Some(&snapshot),
            &entries,
            &|_, _| {},
        )
        .unwrap();
        let (reopened, warning) = open_project(output, &|_, _| {}).unwrap();
        assert_eq!(warning, None);
        assert_eq!(reopened.face_colors, mesh.face_colors);
        assert_eq!(
            reopened.texture_slot_mapping.unwrap().assignments,
            snapshot.mapping.assignments
        );
        eprintln!(
            "exported {} faces from {} colours to 4 tools: {}",
            mesh.faces.len(),
            preview.rows.len(),
            output.display()
        );
    }
}
