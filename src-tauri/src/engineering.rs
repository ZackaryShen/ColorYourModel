use serde::{Deserialize, Serialize};

use crate::export::project_state::Fingerprint;
use crate::export::texture_mapping::TextureSlotMapping;
use crate::mesh::model::MeshModel;
use crate::threemf::opc::Package;

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

pub(crate) fn texture_part_name(plate: usize) -> String {
    format!("Metadata/cym/texture_{}.json", plate + 1)
}

pub(crate) fn encode_texture_part(
    mesh: &MeshModel,
    fingerprint: Fingerprint,
    mapping: Option<&TextureSlotMapping>,
) -> Result<Vec<u8>, String> {
    let bytes = serde_json::to_vec(&TextureState {
        format_version: 1,
        fingerprint,
        mapping: mapping.cloned(),
        has_segments: !mesh.segments.is_empty(),
    })
    .map_err(|error| format!("Could not save texture assignments: {error}"))?;
    if bytes.len() as u64 > MAX_TEXTURE_STATE_BYTES {
        return Err("The texture assignments are too large for a project".into());
    }
    Ok(bytes)
}

/// Return whether to rebuild regions from the saved labels. A just-imported
/// texture has only zero labels; it stays unsegmented across project reopen.
pub(crate) fn restore_texture_state(
    package: &mut Package,
    plate_index: usize,
    fingerprint: Option<Fingerprint>,
    mesh: &mut MeshModel,
) -> Result<bool, String> {
    let part = texture_part_name(plate_index);
    if !package.part_names().iter().any(|name| name == &part) {
        return Ok(true);
    }
    let expected = fingerprint.ok_or("Texture state has no matching 3MF engineering plate")?;
    let bytes = package.read_at_most(&part, MAX_TEXTURE_STATE_BYTES)?;
    let saved: TextureState = serde_json::from_slice(&bytes)
        .map_err(|error| format!("Unreadable texture project state: {error}"))?;
    if saved.format_version != 1 || saved.fingerprint != expected {
        return Err("Texture state version or geometry does not match this 3MF".into());
    }
    mesh.texture_painting = true;
    // Texture stroke groups need the session's underpaint to repaint accurately.
    mesh.paint_sources.clear();
    if let Some(mapping) = &saved.mapping {
        mapping.validate()?;
    }
    mesh.texture_slot_mapping = saved.mapping;
    Ok(saved.has_segments || mesh.segment_labels.iter().any(|&label| label != 0))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::export::presets::ExportSelection;
    use crate::export::texture_mapping::{self, TextureColorAssignment, TextureExportSnapshot};
    use std::path::Path;

    #[test]
    fn texture_state_cannot_attach_to_mismatching_geometry() {
        use crate::export::project_state::fingerprint_of;
        use crate::threemf::test_support::Fixture;
        let mesh = MeshModel::new();
        let saved = fingerprint_of(&[[0., 0., 0.], [1., 0., 0.], [0., 1., 0.]], &[[0, 1, 2]]);
        let other = fingerprint_of(&[[0., 0., 0.], [2., 0., 0.], [0., 1., 0.]], &[[0, 1, 2]]);
        let bytes = encode_texture_part(&mesh, saved, None).unwrap();
        let file = Fixture::new(&[("Metadata/cym/texture_2.json", bytes)]);
        let mut package = Package::open(file.path()).unwrap();
        let mut plate = MeshModel::new();
        plate.record_paint_sources(Some([255; 4]), &[(0, [127, 127, 127, 255])]);
        assert!(restore_texture_state(&mut package, 1, Some(other), &mut plate).is_err());
        assert!(!plate.texture_painting);
        assert_eq!(plate.paint_sources.len(), 1);
        assert!(restore_texture_state(&mut package, 0, Some(saved), &mut plate).unwrap());
        assert!(!restore_texture_state(&mut package, 1, Some(saved), &mut plate).unwrap());
        assert!(plate.texture_painting);
        assert!(plate.paint_sources.is_empty());
    }

    #[test]
    #[ignore = "requires a version-1 texture project from the single-plate writer"]
    fn legacy_single_plate_texture_project_opens_in_multi_plate_reader() {
        let path =
            std::env::var_os("CYM_LEGACY_TEXTURE_PROJECT").expect("set CYM_LEGACY_TEXTURE_PROJECT");
        let path = Path::new(&path);
        let mut package = Package::open(path).unwrap();
        let bytes = package.read(&texture_part_name(0)).unwrap();
        let saved: TextureState = serde_json::from_slice(&bytes).unwrap();
        let mapping = saved.mapping.expect("the fixture includes confirmed slots");
        let (_, plate, _) = crate::threemf::open_active_plate(path, &|_, _| {}).unwrap();
        assert_eq!(
            plate.engineering_state,
            crate::threemf::EngineeringState::Restored
        );
        assert!(!plate.mesh.texture_state_dropped);
        assert!(plate.mesh.texture_painting);
        assert_eq!(plate.mesh.texture_slot_mapping.as_ref(), Some(&mapping));
        assert_eq!(
            plate.mesh.faces.len(),
            saved.fingerprint.face_count as usize
        );
        assert_eq!(!plate.mesh.segments.is_empty(), saved.has_segments);
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
        let output = Path::new(&output);
        crate::export::threemf::write_mapped_texture_3mf(
            &crate::export::threemf::ProjectPlate {
                name: "Acceptance plate".into(),
                mesh: &mesh,
            },
            output,
            &snapshot,
            &|_, _| {},
        )
        .unwrap();
        let reopened = crate::threemf::load_plate(output, 0, &|_, _| {})
            .unwrap()
            .mesh;
        assert!(!reopened.texture_state_dropped);
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
