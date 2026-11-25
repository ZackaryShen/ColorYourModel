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
    build_project_settings_config, build_selected_filament_config, build_selected_machine_config,
    build_selected_process_config, filament_preset_path, MACHINE_PRESET_PATH, PROCESS_PRESET_PATH,
};
use super::project_state;
use super::quantize::{
    bounded_color_histogram, map_colors_to_palette, quantize_face_colors, quantize_histogram,
    Quantized,
};
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

/// Engineering state this writer owns: the project header (unit + geometry
/// fingerprint), the exact per-face state and the undo history. Versioned and
/// additive — a slicer ignores the whole directory, and the reader drops it
/// whenever the fingerprint does not match the mesh it just assembled.
const ENGINEERING_HEADER_PATH: &str = "Metadata/cym/project.json";
const ENGINEERING_PLATE_PATH: &str = "Metadata/cym/plate_1.bin";
const ENGINEERING_HISTORY_PATH: &str = "Metadata/cym/history_1.bin";

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
    progress: &dyn Fn(f32, &str),
) -> Result<(), String> {
    export_3mf_with_source(mesh, output_path, selection, false, progress)
}

/// Save with source preset preservation when editing an imported project.
#[derive(Debug)]
pub struct ProjectExportReport {
    pub history_truncated: Vec<bool>,
}

pub struct ProjectPlate<'a> {
    pub name: String,
    pub mesh: &'a MeshModel,
}

pub fn export_3mf_with_source(
    mesh: &MeshModel,
    output_path: &Path,
    selection: Option<&ExportSelection>,
    preserve_source: bool,
    progress: &dyn Fn(f32, &str),
) -> Result<(), String> {
    let source = if preserve_source {
        mesh.source_presets
            .as_ref()
            .map(|s| s.as_ref().map_err(Clone::clone))
            .transpose()?
    } else {
        None
    };
    export_project_3mf(
        &[ProjectPlate {
            name: "plate 1".into(),
            mesh,
        }],
        0,
        output_path,
        selection,
        source,
        progress,
    )
    .map(|_| ())
}

/// Saving editing work does not require the mesh to be printable yet.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProjectWritePurpose {
    EngineeringSave,
    PrintExport,
}

/// All plates share one palette and one source preset snapshot.
pub fn export_project_3mf(
    plates: &[ProjectPlate<'_>],
    active_index: usize,
    output_path: &Path,
    selection: Option<&ExportSelection>,
    source: Option<&super::source_presets::SourcePresets>,
    progress: &dyn Fn(f32, &str),
) -> Result<ProjectExportReport, String> {
    write_project_3mf(
        plates,
        active_index,
        output_path,
        selection,
        source,
        ProjectWritePurpose::PrintExport,
        progress,
    )
}

pub fn write_project_3mf(
    plates: &[ProjectPlate<'_>],
    active_index: usize,
    output_path: &Path,
    selection: Option<&ExportSelection>,
    source: Option<&super::source_presets::SourcePresets>,
    purpose: ProjectWritePurpose,
    progress: &dyn Fn(f32, &str),
) -> Result<ProjectExportReport, String> {
    if plates.is_empty() || active_index >= plates.len() {
        return Err("Project has no valid active plate".into());
    }
    let colors = project_colors(plates)?;
    let quantized = if let Some(source) = source {
        source.quantize(&colors, colors.len())?
    } else {
        quantize_project(plates, &colors)
    };
    let resolved = if source.is_some() {
        None
    } else {
        selection
            .map(|s| s.resolve(quantized.palette.len()))
            .transpose()
            .map_err(|e| format!("export selection: {e}"))?
    };
    let mut entries = vec![
        (
            PROJECT_CONFIG_PATH.to_string(),
            build_project_settings_config(&quantized.palette, resolved.as_ref()).into_bytes(),
        ),
        (
            "[Content_Types].xml".to_string(),
            build_content_types().into_bytes(),
        ),
        ("_rels/.rels".to_string(), build_rels().into_bytes()),
    ];
    match &resolved {
        // Selected path: the machine + process presets are embedded so Orca's
        // dropdown can show the full vendor name ("Snapmaker U1 (0.4 nozzle)")
        // even when the vendor profile is not installed. Filament presets are
        // deliberately NOT embedded — the project file's N-length arrays carry
        // everything the spool card needs, and Orca itself writes zero
        // filament files for "stock filament, custom colours" (REFUTE-7).
        Some(sel) => {
            entries.push((
                MACHINE_PRESET_PATH.to_string(),
                build_selected_machine_config(sel).into_bytes(),
            ));
            entries.push((
                PROCESS_PRESET_PATH.to_string(),
                build_selected_process_config(sel).into_bytes(),
            ));
            // Self-contained filament presets: one per slot, named after the
            // real vendor preset so it matches project_settings.filament_settings_id.
            // Without these the spool card is incomplete unless the vendor
            // profile happens to be installed (REFUTE-7, superseded).
            for (i, (f, rgb)) in sel
                .filaments
                .iter()
                .zip(quantized.palette.iter())
                .enumerate()
            {
                let slot = (i + 1) as u8;
                entries.push((
                    filament_preset_path(slot),
                    build_selected_filament_config(f, slot, [rgb[0], rgb[1], rgb[2]]).into_bytes(),
                ));
            }
        }
        // Generic path: keep the original 1 + N layout (machine + process +
        // one filament preset per slot).
        None => {
            let machine_config = build_machine_profile_config();
            let process_config = build_process_profile_config();
            entries.push((MACHINE_PRESET_PATH.to_string(), machine_config.into_bytes()));
            entries.push((PROCESS_PRESET_PATH.to_string(), process_config.into_bytes()));
            for (i, rgb) in quantized.palette.iter().enumerate() {
                let slot = (i + 1) as u8;
                let slot_rgb = [rgb[0], rgb[1], rgb[2]];
                entries.push((
                    filament_preset_path(slot),
                    build_filament_profile_config(slot, slot_rgb).into_bytes(),
                ));
            }
        }
    }

    if let Some(source) = source {
        source.apply(&quantized.palette, &mut entries)?;
    }
    let config = entries
        .iter_mut()
        .find(|(path, _)| path == PROJECT_CONFIG_PATH)
        .unwrap();
    let mut settings: serde_json::Value =
        serde_json::from_slice(&config.1).map_err(|e| format!("project settings: {e}"))?;
    let (mut bed_width, mut bed_depth) = crate::threemf::bed_size(&config.1);
    if bed_width <= 0.0 || bed_depth <= 0.0 {
        // The built-in generic profile uses a 220 mm square bed.
        bed_width = 220.0;
        bed_depth = 220.0;
        settings["printable_area"] = serde_json::json!(["0x0", "220x0", "220x220", "0x220"]);
        config.1 = serde_json::to_vec_pretty(&settings).map_err(|e| e.to_string())?;
    }
    let mut worlds = Vec::with_capacity(plates.len());
    let mut headers = Vec::with_capacity(plates.len());
    for (i, plate) in plates.iter().enumerate() {
        let origin = crate::threemf::plate_origin(i as u32 + 1, plates.len(), bed_width, bed_depth);
        let placement = placement_offset(plate.mesh, bed_width, bed_depth, purpose)?;
        let world: Vec<_> = plate
            .mesh
            .vertices
            .iter()
            .map(|v| {
                [
                    v[0] + placement[0] + origin[0],
                    v[1] + placement[1] + origin[1],
                    v[2] + placement[2],
                ]
            })
            .collect();
        if world.iter().flatten().any(|value| !value.is_finite()) {
            return Err("Model placement exceeds the supported numeric range".into());
        }
        let (history, history_truncated) =
            project_state::encode_history_report(&plate.mesh.history);
        headers.push(project_state::PlateStateHeader {
            plater_id: i as u32 + 1,
            name: plate.name.clone(),
            object_ids: vec![i as u32 + 1],
            fingerprint: project_state::fingerprint_of(&world, &plate.mesh.faces),
            history_truncated,
            placement_offset: placement,
        });
        worlds.push(world);
        entries.push((
            format!("Metadata/cym/plate_{}.bin", i + 1),
            project_state::encode_plate_state(plate.mesh),
        ));
        entries.push((format!("Metadata/cym/history_{}.bin", i + 1), history));
        if !plate.mesh.paint_sources.is_empty() {
            entries.push((
                format!("Metadata/cym/paint_sources_{}.bin", i + 1),
                project_state::encode_paint_sources(plate.mesh),
            ));
        }
    }
    let state = project_state::ProjectState {
        unit: "millimeter".into(),
        active_plate: active_index as u32 + 1,
        paint_sources_complete: true,
        plates: headers,
    };
    let history_truncated = state
        .plates
        .iter()
        .map(|plate| plate.history_truncated)
        .collect();
    entries.push((
        ENGINEERING_HEADER_PATH.into(),
        project_state::encode_project_json(&state).into_bytes(),
    ));
    entries.push((
        MODEL_PATH.into(),
        build_project_model_xml(plates, &worlds, &quantized.face_slots)?.into_bytes(),
    ));
    entries.push((
        "Metadata/model_settings.config".into(),
        build_model_settings(plates, quantized.palette.len()).into_bytes(),
    ));
    progress(0.7, "3mf:zip");
    let (file, temporary) = TemporaryOutput::create(output_path)?;
    let mut zip = zip::ZipWriter::new(file);
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);
    for (path, body) in entries {
        zip.start_file(&path, options)
            .map_err(|e| format!("Failed to create {path} entry: {e}"))?;
        std::io::Write::write_all(&mut zip, &body)
            .map_err(|e| format!("Failed to write {path}: {e}"))?;
    }
    let file = zip
        .finish()
        .map_err(|e| format!("Failed to finalize ZIP: {e}"))?;
    file.sync_all()
        .map_err(|e| format!("Failed to sync ZIP: {e}"))?;
    drop(file);
    std::fs::rename(&temporary.path, output_path)
        .map_err(|e| format!("Failed to replace project: {e}"))?;
    progress(0.97, "3mf:zip");
    Ok(ProjectExportReport { history_truncated })
}

fn quantize_project(plates: &[ProjectPlate<'_>], colors: &[[u8; 4]]) -> Quantized {
    let mut merged = std::collections::BTreeMap::new();
    for plate in plates {
        for (rgb, count) in bounded_color_histogram(&plate.mesh.face_colors, plate.mesh.faces.len())
        {
            *merged.entry(rgb).or_insert(0usize) += count;
        }
    }
    let mut palette = quantize_histogram(
        &merged.into_iter().collect::<Vec<_>>(),
        MAX_EXTRUDER_SLOT as usize,
    );
    if palette.is_empty() {
        palette.push([200, 200, 200]);
    }
    let face_slots = map_colors_to_palette(colors, &palette);
    Quantized {
        palette,
        face_slots,
    }
}

pub fn project_colors(plates: &[ProjectPlate<'_>]) -> Result<Vec<[u8; 4]>, String> {
    let mut colors = Vec::new();
    for plate in plates {
        let mesh = plate.mesh;
        if mesh.face_colors.is_empty() {
            colors.extend(std::iter::repeat([200, 200, 200, 255]).take(mesh.faces.len()));
        } else if mesh.face_colors.len() != mesh.faces.len() {
            return Err(format!(
                "Plate {} has inconsistent face colours",
                plate.name
            ));
        } else {
            colors.extend_from_slice(&mesh.face_colors);
        }
    }
    Ok(colors)
}

fn placement_offset(
    mesh: &MeshModel,
    width: f32,
    depth: f32,
    purpose: ProjectWritePurpose,
) -> Result<[f32; 3], String> {
    if mesh.vertices.is_empty() {
        return Ok([0.0; 3]);
    }
    let mut min = [f32::INFINITY; 3];
    let mut max = [f32::NEG_INFINITY; 3];
    for v in &mesh.vertices {
        for i in 0..3 {
            if !v[i].is_finite() {
                return Err("Model contains non-finite coordinates".into());
            }
            min[i] = min[i].min(v[i]);
            max[i] = max[i].max(v[i]);
        }
    }
    if (0..3).any(|axis| !(max[axis] - min[axis]).is_finite()) {
        return Err("Model coordinate extent exceeds the supported numeric range".into());
    }
    if purpose == ProjectWritePurpose::PrintExport
        && (max[0] - min[0] > width || max[1] - min[1] > depth)
    {
        return Err("Model does not fit the selected print bed; choose a larger printer".into());
    }
    let mut shift = [0.0; 3];
    for (axis, size) in [(0, width), (1, depth)] {
        if min[axis] < 0.0 || max[axis] > size {
            shift[axis] = (size - (max[axis] - min[axis])) * 0.5 - min[axis];
        }
    }
    if min[2] < 0.0 {
        shift[2] = -min[2];
    }
    Ok(shift)
}

fn build_project_model_xml(
    plates: &[ProjectPlate<'_>],
    worlds: &[Vec<[f32; 3]>],
    slots: &[u8],
) -> Result<String, String> {
    use std::fmt::Write;
    let mut xml = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?><model unit="millimeter" xml:lang="en-US" xmlns="{NS_CORE}" xmlns:BambuStudio="{NS_BAMBU}"><resources>"#
    );
    let mut offset = 0;
    for (i, plate) in plates.iter().enumerate() {
        write!(
            xml,
            r#"<object id="{}" type="model"><mesh><vertices>"#,
            i + 1
        )
        .unwrap();
        for v in &worlds[i] {
            write!(
                xml,
                r#"<vertex x="{:.6}" y="{:.6}" z="{:.6}"/>"#,
                v[0], v[1], v[2]
            )
            .unwrap();
        }
        xml.push_str("</vertices><triangles>");
        for face in &plate.mesh.faces {
            if face
                .iter()
                .any(|&v| v as usize >= plate.mesh.vertices.len())
            {
                return Err("Model contains an invalid face index".into());
            }
            let paint = encode_paint_color(slots[offset]).ok_or("Invalid extruder slot")?;
            offset += 1;
            write!(
                xml,
                r#"<triangle v1="{}" v2="{}" v3="{}" paint_color="{}"/>"#,
                face[0], face[1], face[2], paint
            )
            .unwrap();
        }
        xml.push_str("</triangles></mesh></object>");
    }
    xml.push_str("</resources><build>");
    for i in 0..plates.len() {
        write!(xml, r#"<item objectid="{}" printable="1"/>"#, i + 1).unwrap();
    }
    xml.push_str("</build></model>");
    Ok(xml)
}

fn build_model_settings(plates: &[ProjectPlate<'_>], slots: usize) -> String {
    use std::fmt::Write;
    let mut xml = String::from(r#"<?xml version="1.0" encoding="UTF-8"?><config>"#);
    for (i, plate) in plates.iter().enumerate() {
        let name = quick_xml::escape::escape(&plate.name);
        write!(xml,r#"<object id="{}"><metadata key="name" value="{}"/><metadata key="extruder" value="0"/><part id="{}" subtype="normal_part"><metadata key="name" value="{}"/></part></object>"#,i+1,name,i+1,name).unwrap();
    }
    let maps = vec!["1"; slots].join(" ");
    for (i, plate) in plates.iter().enumerate() {
        let name = quick_xml::escape::escape(&plate.name);
        write!(xml,r#"<plate><metadata key="plater_id" value="{}"/><metadata key="plater_name" value="{}"/><metadata key="locked" value="false"/><metadata key="filament_map_mode" value="Auto For Flush"/><metadata key="filament_maps" value="{}"/><model_instance><metadata key="object_id" value="{}"/><metadata key="instance_id" value="0"/><metadata key="identify_id" value="{}"/></model_instance></plate>"#,i+1,name,maps,i+1,i+1).unwrap();
    }
    xml.push_str("<assemble>");
    for i in 0..plates.len() {
        write!(xml,r#"<assemble_item object_id="{}" instance_id="0" transform="1 0 0 0 1 0 0 0 1 0 0 0" offset="0 0 0"/>"#,i+1).unwrap();
    }
    xml.push_str("</assemble></config>");
    xml
}

/// Keep the original project intact until a complete ZIP has reached disk.
struct TemporaryOutput {
    path: std::path::PathBuf,
}

impl TemporaryOutput {
    fn create(output: &Path) -> Result<(std::fs::File, Self), String> {
        use std::sync::atomic::{AtomicU64, Ordering};
        static NEXT_ID: AtomicU64 = AtomicU64::new(0);
        let parent = output.parent().unwrap_or_else(|| Path::new("."));
        output.file_name().ok_or("Output path has no filename")?;
        loop {
            let path = parent.join(format!(
                ".cym-save-{}-{}.tmp",
                std::process::id(),
                NEXT_ID.fetch_add(1, Ordering::Relaxed)
            ));
            match std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&path)
            {
                Ok(file) => return Ok((file, Self { path })),
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(error) => return Err(format!("Failed to create temporary project: {error}")),
            }
        }
    }
}

impl Drop for TemporaryOutput {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
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

/// Mirrors what `_add_content_types_file_to_archive` writes in `bbs_3mf.cpp`,
/// plus the two default types the engineering parts are declared under.
/// Note that `.config` is deliberately absent there too; the reader locates
/// `project_settings.config` by path, not by content type.
///
/// The engineering parts are declared by *extension* rather than per-part
/// `Override` entries: the reader is path-driven, and two defaults cover every
/// part under `Metadata/cym/` without restating each name.
fn build_content_types() -> String {
    r#"<?xml version="1.0" encoding="UTF-8"?>
<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">
 <Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>
 <Default Extension="model" ContentType="application/vnd.ms-package.3dmanufacturing-3dmodel+xml"/>
 <Default Extension="png" ContentType="image/png"/>
 <Default Extension="gcode" ContentType="text/x.gcode"/>
 <Default Extension="bin" ContentType="application/octet-stream"/>
 <Default Extension="json" ContentType="application/json"/>
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
    use crate::mesh::history::OpKind;
    use crate::mesh::model::{MeshModel, MANUAL_SEGMENT_OFFSET};
    use std::collections::HashMap;

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
    fn an_interrupted_save_preserves_the_existing_project_and_cleans_temporary_files() {
        let dir = std::env::temp_dir().join(format!("cym-atomic-save-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("original.3mf");
        let previous = b"the user's existing project";
        std::fs::write(&path, previous).unwrap();
        let mesh = two_face_mesh(vec![[255, 0, 0, 255], [0, 0, 255, 255]]);
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            export_3mf(&mesh, &path, None, &|_, stage| {
                if stage == "3mf:zip" {
                    panic!("interrupted while writing ZIP");
                }
            })
        }));
        assert!(result.is_err());
        assert_eq!(std::fs::read(&path).unwrap(), previous);
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 1);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_failed_replacement_preserves_the_destination_and_removes_the_temporary_zip() {
        let dir = std::env::temp_dir().join(format!("cym-atomic-error-{}", std::process::id()));
        let path = dir.join("original.3mf");
        std::fs::create_dir_all(&path).unwrap();
        std::fs::write(path.join("sentinel"), b"keep").unwrap();
        let mesh = two_face_mesh(vec![[255, 0, 0, 255], [0, 0, 255, 255]]);
        assert!(export_3mf(&mesh, &path, None, &|_, _| {}).is_err());
        assert_eq!(std::fs::read(path.join("sentinel")).unwrap(), b"keep");
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 1);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_long_valid_project_filename_can_still_be_saved() {
        let dir = std::env::temp_dir().join(format!("cym-atomic-long-name-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(format!("{}.3mf", "x".repeat(245)));
        std::fs::write(&path, b"previous project").unwrap();
        let mesh = two_face_mesh(vec![[255, 0, 0, 255], [0, 0, 255, 255]]);
        export_3mf(&mesh, &path, None, &|_, _| {}).unwrap();
        assert!(crate::threemf::read_project(&path, &|_, _| {}).is_ok());
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 1);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_successful_save_replaces_the_existing_project_with_a_readable_zip() {
        let dir = std::env::temp_dir().join(format!("cym-atomic-success-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("original.3mf");
        std::fs::write(&path, b"previous project").unwrap();
        let mesh = two_face_mesh(vec![[255, 0, 0, 255], [0, 0, 255, 255]]);
        export_3mf(&mesh, &path, None, &|_, _| {}).unwrap();
        let imported = crate::threemf::read_project(&path, &|_, _| {}).unwrap();
        assert_eq!(imported.plates[0].mesh.face_colors, mesh.face_colors);
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 1);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_full_export_round_trips_through_a_real_zip() {
        let mesh = two_face_mesh(vec![[255, 0, 0, 255], [0, 0, 255, 255]]);
        let dir = std::env::temp_dir().join("cym-3mf-test");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("roundtrip.3mf");

        export_3mf(&mesh, &path, None, &|_, _| {}).expect("export must succeed");

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

        export_3mf(&mesh, &path, Some(&sel), &|_, _| {}).expect("selected export must succeed");

        let file = std::fs::File::open(&path).unwrap();
        let mut archive = zip::ZipArchive::new(file).expect("output must be a valid zip");
        let names: Vec<String> = archive.file_names().map(|s| s.to_string()).collect();

        // Selected path: machine + process + one self-contained filament
        // preset per slot (REFUTE-7 superseded — the file must be complete even
        // without the vendor profile installed).
        for expected in [
            MACHINE_PRESET_PATH,
            PROCESS_PRESET_PATH,
            "Metadata/filament_settings_1.config",
        ] {
            assert!(names.iter().any(|n| n == expected), "{} missing", expected);
        }

        // The embedded filament file uses the real vendor preset name, matching
        // project_settings.filament_settings_id so Orca resolves it.
        let mut body = String::new();
        {
            let mut f = archive
                .by_name("Metadata/filament_settings_1.config")
                .expect("filament preset must be present");
            std::io::Read::read_to_string(&mut f, &mut body).unwrap();
        }
        let filament: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(filament["name"], "Generic PLA");
        assert_eq!(
            filament["filament_settings_id"],
            serde_json::json!(["Generic PLA"])
        );
        assert_eq!(filament["filament_type"], "PLA");

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
        // The project's filament reference must match the embedded file's name.
        assert!(
            project["filament_settings_id"]
                .as_array()
                .unwrap()
                .iter()
                .any(|v| v == "Generic PLA"),
            "project filament_settings_id must reference the embedded preset"
        );

        let _ = std::fs::remove_file(&path);
    }

    // -----------------------------------------------------------------------
    // Engineering parts (`Metadata/cym/*`)
    // -----------------------------------------------------------------------

    /// A four-face tetrahedron with every piece of engineering state set to a
    /// non-default value: a hand-drawn region with a Chinese name, a second
    /// named region, an exact RGBA the quantiser would change, and one paint
    /// operation in history.
    ///
    /// `history` records the state *before* the paint, which is why undo must
    /// take face 1 back to the neutral grey while the other faces keep theirs.
    fn painted_mesh() -> MeshModel {
        let mut mesh = MeshModel::new();
        mesh.vertices = vec![
            [0.0, 0.0, 0.0],
            [20.0, 0.0, 0.0],
            [0.0, 20.0, 0.0],
            [0.0, 0.0, 20.0],
        ];
        mesh.faces = vec![[0, 2, 1], [0, 1, 3], [0, 3, 2], [1, 2, 3]];

        const GREY: [u8; 4] = [138, 138, 138, 255];
        mesh.face_colors = vec![
            GREY,
            [12, 200, 30, 128],
            [0, 0, 255, 255],
            [255, 255, 0, 255],
        ];
        mesh.segment_labels = vec![0, MANUAL_SEGMENT_OFFSET + 1, 0, MANUAL_SEGMENT_OFFSET];
        mesh.next_manual_label = MANUAL_SEGMENT_OFFSET + 2;
        mesh.segment_names = HashMap::from([
            (MANUAL_SEGMENT_OFFSET, "左臂".to_string()),
            (MANUAL_SEGMENT_OFFSET + 1, "right arm".to_string()),
        ]);
        // `record` takes the values from before the write: the paint applied to
        // face 1 is already reflected in `face_colors` above.
        mesh.history
            .record(OpKind::Paint, Some(7), &[(1, GREY)], &[]);
        mesh
    }

    /// Export to a unique path under the shared temp directory.
    fn exported(mesh: &MeshModel, name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join("cym-3mf-test");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(name);
        export_3mf(mesh, &path, None, &|_, _| {}).expect("export must succeed");
        path
    }

    fn read_entry(archive: &mut zip::ZipArchive<std::fs::File>, name: &str) -> Vec<u8> {
        let mut body = Vec::new();
        std::io::Read::read_to_end(
            &mut archive
                .by_name(name)
                .unwrap_or_else(|_| panic!("{name} is missing")),
            &mut body,
        )
        .unwrap();
        body
    }

    /// The writer emits every component with `{:.6}`, so a round trip can only
    /// promise the values the file actually carries.
    fn canonical(vertices: &[[f32; 3]]) -> Vec<[f32; 3]> {
        vertices
            .iter()
            .map(|vertex| {
                vertex.map(|component| {
                    format!("{:.6}", component)
                        .parse::<f32>()
                        .expect("every f32 formats to a parsable string")
                })
            })
            .collect()
    }

    #[test]
    fn an_export_carries_the_engineering_parts_and_declares_their_content_types() {
        let mesh = painted_mesh();
        let path = exported(&mesh, "engineering-parts.3mf");

        let file = std::fs::File::open(&path).unwrap();
        let mut archive = zip::ZipArchive::new(file).expect("output must be a valid zip");
        let names: Vec<String> = archive.file_names().map(str::to_string).collect();
        for expected in [
            "Metadata/cym/project.json",
            "Metadata/cym/plate_1.bin",
            "Metadata/cym/history_1.bin",
        ] {
            assert!(
                names.iter().any(|name| name == expected),
                "{expected} is missing from the archive"
            );
        }

        let content_types =
            String::from_utf8(read_entry(&mut archive, "[Content_Types].xml")).unwrap();
        assert!(
            content_types
                .contains(r#"<Default Extension="bin" ContentType="application/octet-stream"/>"#),
            "the bin parts need a content type: {content_types}"
        );
        assert!(
            content_types.contains(r#"<Default Extension="json" ContentType="application/json"/>"#),
            "the json part needs a content type: {content_types}"
        );

        // The header must carry the unit and the fingerprint of the geometry
        // that was actually written.
        let json =
            String::from_utf8(read_entry(&mut archive, "Metadata/cym/project.json")).unwrap();
        let header = crate::export::project_state::decode_project_json(&json)
            .expect("the engineering header must parse");
        assert_eq!(header.unit, mesh.unit);
        assert_eq!(header.active_plate, 1);
        assert_eq!(header.plates.len(), 1);
        assert_eq!(header.plates[0].plater_id, 1);
        assert_eq!(
            header.plates[0].fingerprint,
            crate::export::project_state::fingerprint_of(&mesh.vertices, &mesh.faces),
            "the header must fingerprint the coordinates the writer emitted"
        );

        // Both binary parts must decode against the geometry in the same file.
        let plate_bytes = read_entry(&mut archive, "Metadata/cym/plate_1.bin");
        let state =
            crate::export::project_state::decode_plate_state(&plate_bytes, mesh.faces.len())
                .expect("plate_1.bin must parse");
        assert_eq!(state.face_colors, mesh.face_colors);
        assert_eq!(state.segment_labels, mesh.segment_labels);
        assert_eq!(state.next_manual_label, mesh.next_manual_label);
        assert_eq!(state.segment_names, mesh.segment_names);

        let history_bytes = read_entry(&mut archive, "Metadata/cym/history_1.bin");
        let history =
            crate::export::project_state::decode_history(&history_bytes, mesh.faces.len())
                .expect("history_1.bin must parse")
                .expect("a known history must decode");
        assert_eq!(history.undo_depth(), mesh.history.undo_depth());
        assert_eq!(history.redo_depth(), mesh.history.redo_depth());

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn a_saved_history_loss_notice_survives_reopening_and_saving_again() {
        let mut mesh = painted_mesh();
        mesh.history.set_persisted_truncated(true);
        let path = exported(&mesh, "engineering-truncated-history.3mf");
        let first = crate::threemf::read_project(&path, &|_, _| {}).unwrap();
        assert!(first.plates[0].history_dropped);
        assert!(first.plates[0].mesh.history.can_undo());
        export_3mf(&first.plates[0].mesh, &path, None, &|_, _| {}).unwrap();
        let second = crate::threemf::read_project(&path, &|_, _| {}).unwrap();
        assert!(second.plates[0].history_dropped);
        assert!(second.plates[0].mesh.history.can_undo());
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn export_then_read_restores_geometry_colours_labels_names_and_history() {
        let mesh = painted_mesh();
        let path = exported(&mesh, "engineering-roundtrip.3mf");

        let mut project =
            crate::threemf::read_project(&path, &|_, _| {}).expect("the export must read back");
        assert_eq!(project.plates.len(), 1, "P0 writes exactly one plate");
        assert_eq!(project.unit, "millimeter");
        let plate = &mut project.plates[0];
        assert_eq!(
            plate.engineering_state,
            crate::threemf::EngineeringState::Restored,
            "a file this build wrote must restore its engineering state"
        );
        assert_eq!(plate.plater_id, 1);

        assert_eq!(plate.mesh.faces, mesh.faces, "face order is the contract");
        assert_eq!(
            plate.mesh.vertices,
            canonical(&mesh.vertices),
            "geometry must survive the writer's {{:.6}} precision"
        );

        // The engineering state is authoritative, not the quantised palette:
        // alpha 128 survives even though the 3MF colour channel carries slots.
        assert_eq!(
            plate.mesh.face_colors, mesh.face_colors,
            "exact RGBA must come back, not the quantised palette"
        );
        assert_eq!(plate.mesh.segment_labels, mesh.segment_labels);
        assert_eq!(plate.mesh.segment_names, mesh.segment_names);
        assert_eq!(plate.mesh.next_manual_label, mesh.next_manual_label);

        // A restored history must be usable, not merely present.
        assert!(
            plate.mesh.history.can_undo(),
            "the saved paint operation must be undoable"
        );
        let mut colors = plate.mesh.face_colors.clone();
        let mut labels = plate.mesh.segment_labels.clone();
        assert!(
            plate.mesh.history.undo(&mut colors, &mut labels).is_some(),
            "one undo is available"
        );
        assert_eq!(
            colors,
            vec![
                [138, 138, 138, 255],
                [138, 138, 138, 255],
                [0, 0, 255, 255],
                [255, 255, 0, 255],
            ],
            "undo must revert only face 1, to the colour it had before the saved paint"
        );

        let _ = std::fs::remove_file(&path);
    }

    /// Pins the builder review's finding: a coordinate with more than six
    /// decimals changes its f32 bit pattern through `{:.6}`, so a writer that
    /// hashes raw bits would write a fingerprint the reader can never match —
    /// and the whole engineering state would be silently dropped on reopen.
    #[test]
    fn the_fingerprint_survives_the_writer_precision_round_trip() {
        let mut mesh = painted_mesh();
        mesh.vertices[0] = [0.123456789_f32, -1.23456789, 2.3456789];
        assert_ne!(
            format!("{:.6}", mesh.vertices[0][0])
                .parse::<f32>()
                .unwrap()
                .to_bits(),
            mesh.vertices[0][0].to_bits(),
            "precondition: this coordinate must actually lose bits through {{:.6}}"
        );

        let path = exported(&mesh, "engineering-precision.3mf");
        let project =
            crate::threemf::read_project(&path, &|_, _| {}).expect("the export must read back");
        let plate = &project.plates[0];
        assert_eq!(
            plate.engineering_state,
            crate::threemf::EngineeringState::Restored,
            "a high-precision coordinate must not make the reader drop the state"
        );
        let expected = canonical(&mesh.vertices);
        let shift =
            placement_offset(&mesh, 220.0, 220.0, ProjectWritePurpose::PrintExport).unwrap();
        let world_scale = mesh
            .vertices
            .iter()
            .flat_map(|vertex| (0..3).map(move |axis| (vertex[axis] + shift[axis]).abs()))
            .fold(1.0f32, f32::max);
        // Six decimal XML quantisation plus two f32 add/subtract roundings.
        let tolerance = 1e-6 + 2.0 * f32::EPSILON * world_scale;
        assert!(plate
            .mesh
            .vertices
            .iter()
            .zip(&expected)
            .all(|(a, b)| (0..3).all(|axis| (a[axis] - b[axis]).abs() <= tolerance)));

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
        export_3mf(&mesh, path, Some(&sel), &|_, _| {}).expect("sample export must succeed");
        println!("wrote sample to {}", path.display());
    }
}

#[cfg(test)]
mod multi_plate_tests {
    use super::*;
    use crate::mesh::history::OpKind;
    fn triangle(color: [u8; 4]) -> MeshModel {
        let mut m = MeshModel::new();
        m.vertices = vec![[30.0, 30.0, 0.0], [40.0, 30.0, 0.0], [30.0, 40.0, 0.0]];
        m.faces = vec![[0, 1, 2]];
        m.face_colors = vec![color];
        m.segment_labels = vec![100001];
        m.segment_names.insert(100001, "精确区域".into());
        m.next_manual_label = 100002;
        m.history
            .record(OpKind::Paint, None, &[(0, [8, 9, 10, 255])], &[]);
        m
    }
    #[test]
    fn blended_stroke_source_survives_a_project_round_trip() {
        let mut mesh = triangle([170, 140, 190, 255]);
        let purple = [110, 45, 145, 255];
        mesh.record_paint_sources(Some(purple), &[(0, mesh.face_colors[0])]);
        let path =
            std::env::temp_dir().join(format!("cym-stroke-source-{}.3mf", std::process::id()));
        let plates = [ProjectPlate {
            name: "Stroke".into(),
            mesh: &mesh,
        }];
        export_project_3mf(&plates, 0, &path, None, None, &|_, _| {}).unwrap();
        let loaded = crate::threemf::load_plate(&path, 0, &|_, _| {})
            .unwrap()
            .mesh;
        std::fs::remove_file(path).unwrap();
        assert_eq!(loaded.face_colors, mesh.face_colors);
        assert_eq!(loaded.paint_source_groups(), vec![(purple, 1)]);
        assert_eq!(loaded.used_colors()[0].color, purple);
    }

    #[test]
    fn all_plates_names_active_exact_colours_labels_and_history_round_trip() {
        let models = [
            triangle([255, 0, 0, 255]),
            triangle([0, 255, 0, 123]),
            triangle([0, 0, 255, 255]),
        ];
        let plates: Vec<_> = models
            .iter()
            .enumerate()
            .map(|(i, m)| ProjectPlate {
                name: format!("盘 {i}").into(),
                mesh: m,
            })
            .collect();
        let path =
            std::env::temp_dir().join(format!("cym-multi-roundtrip-{}.3mf", std::process::id()));
        export_project_3mf(&plates, 2, &path, None, None, &|_, _| {}).unwrap();
        let index = crate::threemf::load_plates(&path).unwrap();
        assert_eq!(index.active_index, 2);
        assert_eq!(index.plates.len(), 3);
        for (i, m) in models.iter().enumerate() {
            let mut p = crate::threemf::load_plate(&path, i, &|_, _| {}).unwrap();
            assert_eq!(p.name, format!("盘 {i}"));
            assert_eq!(
                p.engineering_state,
                crate::threemf::EngineeringState::Restored
            );
            assert_eq!(p.mesh.face_colors, m.face_colors);
            assert_eq!(p.mesh.segment_labels, m.segment_labels);
            assert_eq!(p.mesh.segment_names, m.segment_names);
            assert!(p.mesh.history.can_undo());
            p.mesh
                .history
                .undo(&mut p.mesh.face_colors, &mut p.mesh.segment_labels);
            assert_eq!(p.mesh.face_colors[0], [8, 9, 10, 255]);
        }
        std::fs::remove_file(path).unwrap();
    }
    #[test]
    fn negative_stl_is_placed_on_its_plate_and_restores_edit_coordinates() {
        let mut m = triangle([23, 45, 67, 255]);
        for p in &mut m.vertices {
            p[0] -= 35.0;
            p[1] -= 35.0;
        }
        let path =
            std::env::temp_dir().join(format!("cym-negative-roundtrip-{}.3mf", std::process::id()));
        export_project_3mf(
            &[ProjectPlate {
                name: "Centered".into(),
                mesh: &m,
            }],
            0,
            &path,
            None,
            None,
            &|_, _| {},
        )
        .unwrap();
        let p = crate::threemf::load_plate(&path, 0, &|_, _| {}).unwrap();
        assert_eq!(p.mesh.vertices, m.vertices);
        assert_eq!(
            p.engineering_state,
            crate::threemf::EngineeringState::Restored
        );
        std::fs::remove_file(path).unwrap();
    }
}

#[cfg(test)]
mod project_boundaries {
    use super::*;
    fn mesh() -> MeshModel {
        let mut m = MeshModel::new();
        m.vertices = vec![[30.0, 30.0, 0.0], [40.0, 30.0, 0.0], [30.0, 40.0, 0.0]];
        m.faces = vec![[0, 1, 2]];
        m.face_colors = vec![[1, 2, 3, 255]];
        m.segment_labels = vec![0];
        m
    }
    #[test]
    fn engineering_save_rejects_coordinate_extent_overflow_without_touching_output() {
        let mut m = mesh();
        m.vertices[0][0] = -3.0e38;
        m.vertices[1][0] = 3.0e38;
        let path =
            std::env::temp_dir().join(format!("cym-overflow-save-{}.3mf", std::process::id()));
        std::fs::write(&path, b"existing project").unwrap();
        let result = write_project_3mf(
            &[ProjectPlate {
                name: "Out of range".into(),
                mesh: &m,
            }],
            0,
            &path,
            None,
            None,
            ProjectWritePurpose::EngineeringSave,
            &|_, _| {},
        );
        assert!(
            result.is_err(),
            "finite inputs must not generate infinite saved geometry"
        );
        assert_eq!(std::fs::read(&path).unwrap(), b"existing project");
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn an_oversize_plate_does_not_damage_an_existing_project() {
        let mut m = mesh();
        m.vertices[1][0] = 300.0;
        let path =
            std::env::temp_dir().join(format!("cym-oversize-project-{}.3mf", std::process::id()));
        std::fs::write(&path, b"existing project").unwrap();
        assert!(export_project_3mf(
            &[ProjectPlate {
                name: "Too large".into(),
                mesh: &m
            }],
            0,
            &path,
            None,
            None,
            &|_, _| {}
        )
        .unwrap_err()
        .contains("does not fit"));
        assert_eq!(std::fs::read(&path).unwrap(), b"existing project");
        std::fs::remove_file(path).unwrap();
    }
    #[test]
    fn an_empty_plate_survives_next_to_a_coloured_plate() {
        let blank = MeshModel::new();
        let m = mesh();
        let path =
            std::env::temp_dir().join(format!("cym-empty-project-{}.3mf", std::process::id()));
        export_project_3mf(
            &[
                ProjectPlate {
                    name: "空盘".into(),
                    mesh: &blank,
                },
                ProjectPlate {
                    name: "Paint".into(),
                    mesh: &m,
                },
            ],
            0,
            &path,
            None,
            None,
            &|_, _| {},
        )
        .unwrap();
        let project = crate::threemf::read_project(&path, &|_, _| {}).unwrap();
        assert_eq!(project.plates.len(), 2);
        assert_eq!(project.plates[0].mesh.faces.len(), 0);
        assert_eq!(project.plates[1].mesh.face_colors, m.face_colors);
        std::fs::remove_file(path).unwrap();
    }
    #[test]
    #[ignore = "needs CYM_REAL_3MF_MULTI"]
    fn real_two_plate_project_export_preserves_every_plate_colour_and_identity() {
        let input = std::env::var("CYM_REAL_3MF_MULTI").unwrap();
        let original = crate::threemf::read_project(Path::new(&input), &|_, _| {}).unwrap();
        let source = super::super::source_presets::SourcePresets::read(Path::new(&input)).unwrap();
        let plates: Vec<_> = original
            .plates
            .iter()
            .map(|p| ProjectPlate {
                name: p.name.clone(),
                mesh: &p.mesh,
            })
            .collect();
        let path =
            std::env::temp_dir().join(format!("cym-real-multi-project-{}.3mf", std::process::id()));
        export_project_3mf(&plates, 1, &path, None, source.as_ref(), &|_, _| {}).unwrap();
        let index = crate::threemf::load_plates(&path).unwrap();
        assert_eq!(index.plates.len(), original.plates.len());
        assert_eq!(index.active_index, 1);
        for (i, old) in original.plates.iter().enumerate() {
            let p = crate::threemf::load_plate(&path, i, &|_, _| {}).unwrap();
            assert_eq!(
                p.engineering_state,
                crate::threemf::EngineeringState::Restored
            );
            assert_eq!(p.mesh.face_colors, old.mesh.face_colors);
            assert_eq!(p.mesh.faces.len(), old.mesh.faces.len());
            assert_eq!(p.name, old.name);
        }
        std::fs::remove_file(path).unwrap();
    }
}

#[cfg(test)]
mod histogram_palette_contract {
    use super::*;
    #[test]
    fn writing_uses_the_same_bounded_histogram_palette_as_preview() {
        let mut mesh = MeshModel::new();
        mesh.face_colors = (0..5000u32)
            .map(|i| [(i % 256) as u8, (i / 256) as u8, 31, 255])
            .collect();
        mesh.faces = vec![[0, 0, 0]; mesh.face_colors.len()];
        let plates = [ProjectPlate {
            name: "Many colours".into(),
            mesh: &mesh,
        }];
        let expected = super::super::quantize::quantize_histogram(
            &super::super::quantize::bounded_color_histogram(&mesh.face_colors, mesh.faces.len()),
            16,
        );
        assert_eq!(
            quantize_project(&plates, &mesh.face_colors).palette,
            expected
        );
    }
}
