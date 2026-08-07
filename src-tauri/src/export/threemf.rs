//! 3MF writer targeting the BambuStudio / OrcaSlicer / Snapmaker Orca family.
//!
//! The previous revision emitted the 3MF core `basematerials` resource with
//! `displaycolor` entries and referenced them from each triangle through
//! `pid`/`p1`. That is the standards-compliant way to colour a mesh, and it is
//! also completely inert here: an audit of `libslic3r/Format/` in both the
//! upstream stock tree and the Snapmaker fork found no reader for
//! `basematerials`, and `bbs_3mf.cpp` explicitly discards `pid`/`p1`/`p2`/`p3`
//! while parsing triangles. Files written that way opened as a single grey
//! blob.
//!
//! The channel those slicers actually read is the private `paint_color`
//! attribute on `<triangle>`, which stores an extruder slot index rather than
//! a colour. The slot is resolved to RGB through `filament_colour` in
//! `Metadata/project_settings.config`, so both pieces have to be written
//! together or the colours fall back to whatever preset the user has loaded.

use super::paint_color::{encode_paint_color, MAX_EXTRUDER_SLOT};
use super::presets::{ExportSelection, ResolvedSelection};
use super::project_config::{
    build_filament_profile_config, build_machine_profile_config, build_process_profile_config,
    build_project_settings_config, build_selected_machine_config, build_selected_process_config,
    filament_preset_path, MACHINE_PRESET_PATH, PROCESS_PRESET_PATH,
};
use super::quantize::quantize_face_colors;
use crate::mesh::model::MeshModel;
use quick_xml::events::{BytesDecl, BytesEnd, BytesStart, Event};
use quick_xml::Writer;
use std::io::Cursor;
use std::path::Path;

/// 3MF core namespace. The 2013/01 URI the previous revision used is the
/// *specification* URI, not the schema namespace; readers matching on the
/// namespace would reject the document.
const NS_CORE: &str = "http://schemas.microsoft.com/3dmanufacturing/core/2015/02";
/// Namespace declared by BambuStudio-derived writers. Declaring it keeps the
/// document shape identical to files those slicers produce themselves.
const NS_BAMBU: &str = "http://schemas.bambulab.com/package/2021";

const MODEL_PATH: &str = "3D/3dmodel.model";
const PROJECT_CONFIG_PATH: &str = "Metadata/project_settings.config";

/// Export mesh with per-face colours to a 3MF file readable by OrcaSlicer and
/// Snapmaker Orca.
///
/// `selection` is the machine / process / filament combination the user picked
/// in the export dialog. `None` falls back to the built-in generic profile,
/// which keeps old exports working but does not make the slicer recognise the
/// machine (see `presets.rs`).
pub fn export_3mf(
    mesh: &MeshModel,
    output_path: &Path,
    selection: Option<&ExportSelection>,
) -> Result<(), String> {
    let quantized = quantize_face_colors(&mesh.face_colors, MAX_EXTRUDER_SLOT as usize);

    if quantized.face_slots.len() != mesh.faces.len() {
        return Err(format!(
            "colour quantisation produced {} slots for {} faces",
            quantized.face_slots.len(),
            mesh.faces.len()
        ));
    }

    // Resolve the user's pick against the preset library. A dangling name is
    // an error the dialog should have prevented; failing here is better than
    // silently writing a file that opens with the wrong machine.
    let resolved: Option<ResolvedSelection<'_>> = match selection {
        Some(sel) => Some(
            sel.resolve(quantized.palette.len())
                .map_err(|e| format!("export selection: {}", e))?,
        ),
        None => None,
    };

    let model_xml = build_model_xml(mesh, &quantized.face_slots)?;
    let project_config =
        build_project_settings_config(&quantized.palette, resolved.as_ref());
    let content_types = build_content_types();
    let rels = build_rels();

    let file = std::fs::File::create(output_path)
        .map_err(|e| format!("Failed to create file: {}", e))?;
    let mut zip = zip::ZipWriter::new(file);

    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);

    let mut entries: Vec<(String, String)> = vec![
        (MODEL_PATH.to_string(), model_xml),
        (PROJECT_CONFIG_PATH.to_string(), project_config),
        ("[Content_Types].xml".to_string(), content_types),
        ("_rels/.rels".to_string(), rels),
    ];

    match &resolved {
        // Selected path: the machine + process presets are embedded so Orca's
        // dropdown can show the full vendor name ("Snapmaker U1 (0.4 nozzle)")
        // even when the vendor profile is not installed. Filament presets are
        // deliberately NOT embedded — the project file's N-length arrays carry
        // everything the spool card needs, and Orca itself writes zero
        // filament files for "stock filament, custom colours" (REFUTE-7).
        Some(sel) => {
            entries.push((MACHINE_PRESET_PATH.to_string(), build_selected_machine_config(sel)));
            entries.push((PROCESS_PRESET_PATH.to_string(), build_selected_process_config(sel)));
        }
        // Generic path: keep the original 1 + N layout (machine + process +
        // one filament preset per slot).
        None => {
            let machine_config = build_machine_profile_config();
            let process_config = build_process_profile_config();
            entries.push((MACHINE_PRESET_PATH.to_string(), machine_config));
            entries.push((PROCESS_PRESET_PATH.to_string(), process_config));
            for (i, rgb) in quantized.palette.iter().enumerate() {
                let slot = (i + 1) as u8;
                let slot_rgb = [rgb[0], rgb[1], rgb[2]];
                entries.push((
                    filament_preset_path(slot),
                    build_filament_profile_config(slot, slot_rgb),
                ));
            }
        }
    }

    for (path, body) in &entries {
        zip.start_file(path.as_str(), options)
            .map_err(|e| format!("Failed to create {} entry: {}", path, e))?;
        std::io::Write::write_all(&mut zip, body.as_bytes())
            .map_err(|e| format!("Failed to write {}: {}", path, e))?;
    }

    zip.finish()
        .map_err(|e| format!("Failed to finalize ZIP: {}", e))?;

    Ok(())
}

/// Build `3D/3dmodel.model`.
///
/// `face_slots` is parallel to `mesh.faces` and holds 1-based extruder slots.
/// Every face gets an explicit `paint_color`, including slot 1: leaving it off
/// would make the face inherit the volume's default extruder, and nothing in
/// this writer guarantees that default is 1.
fn build_model_xml(mesh: &MeshModel, face_slots: &[u8]) -> Result<String, String> {
    let mut writer = Writer::new(Cursor::new(Vec::new()));

    writer
        .write_event(Event::Decl(BytesDecl::new("1.0", Some("UTF-8"), None)))
        .map_err(|e| e.to_string())?;

    let mut model_elem = BytesStart::new("model");
    model_elem.push_attribute(("unit", "millimeter"));
    model_elem.push_attribute(("xml:lang", "en-US"));
    model_elem.push_attribute(("xmlns", NS_CORE));
    model_elem.push_attribute(("xmlns:BambuStudio", NS_BAMBU));
    writer
        .write_event(Event::Start(model_elem))
        .map_err(|e| e.to_string())?;

    writer
        .write_event(Event::Start(BytesStart::new("resources")))
        .map_err(|e| e.to_string())?;

    let mut obj_elem = BytesStart::new("object");
    obj_elem.push_attribute(("id", "1"));
    obj_elem.push_attribute(("type", "model"));
    writer
        .write_event(Event::Start(obj_elem))
        .map_err(|e| e.to_string())?;

    writer
        .write_event(Event::Start(BytesStart::new("mesh")))
        .map_err(|e| e.to_string())?;

    writer
        .write_event(Event::Start(BytesStart::new("vertices")))
        .map_err(|e| e.to_string())?;
    for v in &mesh.vertices {
        let mut vert_elem = BytesStart::new("vertex");
        vert_elem.push_attribute(("x", format!("{:.6}", v[0]).as_str()));
        vert_elem.push_attribute(("y", format!("{:.6}", v[1]).as_str()));
        vert_elem.push_attribute(("z", format!("{:.6}", v[2]).as_str()));
        writer
            .write_event(Event::Empty(vert_elem))
            .map_err(|e| e.to_string())?;
    }
    writer
        .write_event(Event::End(BytesEnd::new("vertices")))
        .map_err(|e| e.to_string())?;

    // Triangles are emitted in face order. The reader indexes its
    // mmu_segmentation array by triangle position, so this order is the
    // contract between the two sides.
    writer
        .write_event(Event::Start(BytesStart::new("triangles")))
        .map_err(|e| e.to_string())?;
    for (i, face) in mesh.faces.iter().enumerate() {
        let mut tri_elem = BytesStart::new("triangle");
        tri_elem.push_attribute(("v1", face[0].to_string().as_str()));
        tri_elem.push_attribute(("v2", face[1].to_string().as_str()));
        tri_elem.push_attribute(("v3", face[2].to_string().as_str()));
        if let Some(encoded) = encode_paint_color(face_slots[i]) {
            tri_elem.push_attribute(("paint_color", encoded.as_str()));
        }
        writer
            .write_event(Event::Empty(tri_elem))
            .map_err(|e| e.to_string())?;
    }
    writer
        .write_event(Event::End(BytesEnd::new("triangles")))
        .map_err(|e| e.to_string())?;

    writer
        .write_event(Event::End(BytesEnd::new("mesh")))
        .map_err(|e| e.to_string())?;
    writer
        .write_event(Event::End(BytesEnd::new("object")))
        .map_err(|e| e.to_string())?;
    writer
        .write_event(Event::End(BytesEnd::new("resources")))
        .map_err(|e| e.to_string())?;

    writer
        .write_event(Event::Start(BytesStart::new("build")))
        .map_err(|e| e.to_string())?;
    let mut item_elem = BytesStart::new("item");
    item_elem.push_attribute(("objectid", "1"));
    writer
        .write_event(Event::Empty(item_elem))
        .map_err(|e| e.to_string())?;
    writer
        .write_event(Event::End(BytesEnd::new("build")))
        .map_err(|e| e.to_string())?;

    writer
        .write_event(Event::End(BytesEnd::new("model")))
        .map_err(|e| e.to_string())?;

    let result = writer.into_inner().into_inner();
    String::from_utf8(result).map_err(|e| format!("UTF-8 error: {}", e))
}

/// Mirrors what `_add_content_types_file_to_archive` writes in `bbs_3mf.cpp`.
/// Note that `.config` is deliberately absent there too; the reader locates
/// `project_settings.config` by path, not by content type.
fn build_content_types() -> String {
    r#"<?xml version="1.0" encoding="UTF-8"?>
<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">
 <Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>
 <Default Extension="model" ContentType="application/vnd.ms-package.3dmanufacturing-3dmodel+xml"/>
 <Default Extension="png" ContentType="image/png"/>
 <Default Extension="gcode" ContentType="text/x.gcode"/>
</Types>"#
        .to_string()
}

fn build_rels() -> String {
    r#"<?xml version="1.0" encoding="UTF-8"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
 <Relationship Target="/3D/3dmodel.model" Id="rel-1" Type="http://schemas.microsoft.com/3dmanufacturing/2013/01/3dmodel"/>
</Relationships>"#
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mesh::model::MeshModel;

    /// Two triangles sharing an edge. Only the fields the writer touches are
    /// populated; the spatial indices stay empty because export never queries
    /// them.
    fn two_face_mesh(colors: Vec<[u8; 4]>) -> MeshModel {
        let mut mesh = MeshModel::new();
        mesh.vertices = vec![
            [0.0, 0.0, 0.0],
            [1.0, 0.0, 0.0],
            [0.0, 1.0, 0.0],
            [1.0, 1.0, 0.0],
        ];
        mesh.faces = vec![[0u32, 1, 2], [1, 3, 2]];
        mesh.face_colors = colors;
        mesh
    }

    fn model_xml_of(colors: Vec<[u8; 4]>) -> String {
        let mesh = two_face_mesh(colors);
        let quantized = quantize_face_colors(&mesh.face_colors, MAX_EXTRUDER_SLOT as usize);
        build_model_xml(&mesh, &quantized.face_slots).expect("model xml must build")
    }

    #[test]
    fn no_basematerials_or_pid_are_emitted() {
        let xml = model_xml_of(vec![[255, 0, 0, 255], [0, 0, 255, 255]]);
        assert!(
            !xml.contains("basematerials"),
            "basematerials has no reader and must not be written"
        );
        assert!(
            !xml.contains("displaycolor"),
            "displaycolor has no reader and must not be written"
        );
        assert!(
            !xml.contains("pid="),
            "pid is discarded by the reader and must not be written"
        );
    }

    #[test]
    fn every_triangle_carries_a_paint_color() {
        let xml = model_xml_of(vec![[255, 0, 0, 255], [0, 0, 255, 255]]);
        assert_eq!(
            xml.matches("paint_color=").count(),
            2,
            "each of the two faces needs its own slot"
        );
    }

    #[test]
    fn distinct_colours_land_on_distinct_slots() {
        let mesh = two_face_mesh(vec![[255, 0, 0, 255], [0, 0, 255, 255]]);
        let quantized = quantize_face_colors(&mesh.face_colors, MAX_EXTRUDER_SLOT as usize);
        assert_ne!(quantized.face_slots[0], quantized.face_slots[1]);
    }

    #[test]
    fn uses_the_core_2015_namespace() {
        let xml = model_xml_of(vec![[255, 0, 0, 255], [0, 0, 255, 255]]);
        assert!(xml.contains("3dmanufacturing/core/2015/02"));
        assert!(!xml.contains("3dmanufacturing/2013/01\""));
    }

    #[test]
    fn triangle_order_matches_face_order() {
        let mesh = two_face_mesh(vec![[255, 0, 0, 255], [0, 0, 255, 255]]);
        let quantized = quantize_face_colors(&mesh.face_colors, MAX_EXTRUDER_SLOT as usize);
        let xml = build_model_xml(&mesh, &quantized.face_slots).unwrap();
        let first = xml.find("v1=\"0\"").expect("face 0 must be present");
        let second = xml.find("v1=\"1\"").expect("face 1 must be present");
        assert!(
            first < second,
            "the reader indexes segmentation by triangle position, so face order is load bearing"
        );
    }

    #[test]
    fn a_full_export_round_trips_through_a_real_zip() {
        let mesh = two_face_mesh(vec![[255, 0, 0, 255], [0, 0, 255, 255]]);
        let dir = std::env::temp_dir().join("cym-3mf-test");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("roundtrip.3mf");

        export_3mf(&mesh, &path, None).expect("export must succeed");

        let file = std::fs::File::open(&path).unwrap();
        let mut archive = zip::ZipArchive::new(file).expect("output must be a valid zip");
        let names: Vec<String> = archive.file_names().map(|s| s.to_string()).collect();
        for expected in [
            MODEL_PATH,
            PROJECT_CONFIG_PATH,
            MACHINE_PRESET_PATH,
            PROCESS_PRESET_PATH,
            "[Content_Types].xml",
            "_rels/.rels",
            // Two palette entries, two filament preset files.
            "Metadata/filament_settings_1.config",
            "Metadata/filament_settings_2.config",
        ] {
            assert!(
                names.iter().any(|n| n == expected),
                "{} is missing from the archive",
                expected
            );
        }

        // The embedded machine preset MUST carry `printer_settings_id`, because
        // bbs_3mf.cpp:2596 returns early otherwise and the preset is silently
        // dropped. Same invariant for `print_settings_id` and
        // `filament_settings_id`.
        let mut f = archive
            .by_name(MACHINE_PRESET_PATH)
            .expect("machine preset must be present");
        let mut body = String::new();
        std::io::Read::read_to_string(&mut f, &mut body).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(parsed["printer_settings_id"], "Generic Printer");

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn a_selected_export_embeds_the_vendor_machine_and_process_presets() {
        let mesh = two_face_mesh(vec![[255, 0, 0, 255], [0, 0, 255, 255]]);
        let dir = std::env::temp_dir().join("cym-3mf-test");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("roundtrip-selected.3mf");

        let sel = crate::export::presets::ExportSelection {
            machine_id: "snapmaker_u1".into(),
            nozzle_diameter: "0.4".into(),
            process_name: "0.20 Standard @Snapmaker U1 (0.4 nozzle)".into(),
            filament_names: vec!["Generic PLA".into()],
            target_slicer: "orcaslicer".into(),
        };

        export_3mf(&mesh, &path, Some(&sel)).expect("selected export must succeed");

        let file = std::fs::File::open(&path).unwrap();
        let mut archive = zip::ZipArchive::new(file).expect("output must be a valid zip");
        let names: Vec<String> = archive.file_names().map(|s| s.to_string()).collect();

        // Selected path: machine + process embedded, NO per-slot filament
        // files (REFUTE-7 — the project file's arrays feed the spool card).
        for expected in [MACHINE_PRESET_PATH, PROCESS_PRESET_PATH] {
            assert!(names.iter().any(|n| n == expected), "{} missing", expected);
        }
        assert!(
            !names.iter().any(|n| n.contains("filament_settings_")),
            "selected path must not embed filament preset files"
        );

        let mut body = String::new();
        {
            let mut f = archive
                .by_name(MACHINE_PRESET_PATH)
                .expect("machine preset must be present");
            std::io::Read::read_to_string(&mut f, &mut body).unwrap();
        }
        let machine: serde_json::Value = serde_json::from_str(&body).unwrap();
        // The name Orca's dropdown shows; must equal the project file's ID.
        assert_eq!(machine["printer_settings_id"], "Snapmaker U1 (0.4 nozzle)");
        assert_eq!(machine["printer_model"], "Snapmaker U1");
        assert!(machine.get("machine_start_gcode").is_some());

        let mut body = String::new();
        {
            let mut f = archive
                .by_name(PROJECT_CONFIG_PATH)
                .expect("project config must be present");
            std::io::Read::read_to_string(&mut f, &mut body).unwrap();
        }
        let project: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(project["printer_settings_id"], "Snapmaker U1 (0.4 nozzle)");

        let _ = std::fs::remove_file(&path);
    }

    /// Writes a small multi-colour sample next to the repo's example models so
    /// it can be opened in OrcaSlicer and Snapmaker Orca by hand. Ignored by
    /// default because it leaves a file behind; run with
    /// `cargo test --lib -- --ignored emit_sample`.
    #[test]
    #[ignore]
    fn emit_sample_for_manual_inspection() {
        let mut mesh = MeshModel::new();
        // A 20 mm cube, one colour per face pair.
        let s = 20.0f32;
        mesh.vertices = vec![
            [0.0, 0.0, 0.0],
            [s, 0.0, 0.0],
            [s, s, 0.0],
            [0.0, s, 0.0],
            [0.0, 0.0, s],
            [s, 0.0, s],
            [s, s, s],
            [0.0, s, s],
        ];
        mesh.faces = vec![
            [0, 2, 1],
            [0, 3, 2], // bottom
            [4, 5, 6],
            [4, 6, 7], // top
            [0, 1, 5],
            [0, 5, 4], // front
            [2, 3, 7],
            [2, 7, 6], // back
            [3, 0, 4],
            [3, 4, 7], // left
            [1, 2, 6],
            [1, 6, 5], // right
        ];
        let palette = [
            [255, 0, 0, 255],
            [0, 255, 0, 255],
            [0, 0, 255, 255],
            [255, 255, 0, 255],
            [255, 0, 255, 255],
            [0, 255, 255, 255],
        ];
        mesh.face_colors = (0..12).map(|i| palette[i / 2]).collect();

        let path = std::path::Path::new("../examples/paint_color_sample.3mf");
        let sel = crate::export::presets::ExportSelection {
            machine_id: "snapmaker_u1".into(),
            nozzle_diameter: "0.4".into(),
            process_name: "0.20 Standard @Snapmaker U1 (0.4 nozzle)".into(),
            filament_names: vec!["Generic PLA".into()],
            target_slicer: "orcaslicer".into(),
        };
        export_3mf(&mesh, path, Some(&sel)).expect("sample export must succeed");
        println!("wrote sample to {}", path.display());
    }
}
