use crate::mesh::model::MeshModel;
use quick_xml::events::{BytesDecl, BytesEnd, BytesStart, Event};
use quick_xml::Writer;
use std::collections::HashMap;
use std::io::Cursor;
use std::path::Path;

const NS_3MF: &str = "http://schemas.microsoft.com/3dmanufacturing/2013/01";
const NS_MAT: &str = "http://schemas.microsoft.com/3dmanufacturing/material/2015/02";

/// Export mesh with per-face colors to a 3MF file
pub fn export_3mf(mesh: &MeshModel, output_path: &Path) -> Result<(), String> {
    // Build model XML
    let model_xml = build_model_xml(mesh)?;

    // Build content types XML
    let content_types = build_content_types();

    // Build relationships XML
    let rels = build_rels();

    // Write ZIP (3MF format)
    let file = std::fs::File::create(output_path)
        .map_err(|e| format!("Failed to create file: {}", e))?;
    let mut zip = zip::ZipWriter::new(file);

    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);

    zip.start_file("3D/3dmodel.model", options)
        .map_err(|e| format!("Failed to create model entry: {}", e))?;
    std::io::Write::write_all(&mut zip, model_xml.as_bytes())
        .map_err(|e| format!("Failed to write model: {}", e))?;

    zip.start_file("[Content_Types].xml", options)
        .map_err(|e| format!("Failed to create content types: {}", e))?;
    std::io::Write::write_all(&mut zip, content_types.as_bytes())
        .map_err(|e| format!("Failed to write content types: {}", e))?;

    zip.start_file("_rels/.rels", options)
        .map_err(|e| format!("Failed to create rels: {}", e))?;
    std::io::Write::write_all(&mut zip, rels.as_bytes())
        .map_err(|e| format!("Failed to write rels: {}", e))?;

    zip.finish()
        .map_err(|e| format!("Failed to finalize ZIP: {}", e))?;

    Ok(())
}

fn build_model_xml(mesh: &MeshModel) -> Result<String, String> {
    let mut writer = Writer::new(Cursor::new(Vec::new()));

    // XML declaration
    writer
        .write_event(Event::Decl(BytesDecl::new("1.0", Some("UTF-8"), None)))
        .map_err(|e| e.to_string())?;

    // <model>
    let mut model_elem = BytesStart::new("model");
    model_elem.push_attribute(("unit", "millimeter"));
    model_elem.push_attribute(("xml:lang", "en-US"));
    model_elem.push_attribute(("xmlns", NS_3MF));
    model_elem.push_attribute(("xmlns:m", NS_MAT));
    writer
        .write_event(Event::Start(model_elem))
        .map_err(|e| e.to_string())?;

    // <resources>
    writer
        .write_event(Event::Start(BytesStart::new("resources")))
        .map_err(|e| e.to_string())?;

    // Extract unique colors
    let mut unique_colors: HashMap<[u8; 4], usize> = HashMap::new();
    for rgba in &mesh.face_colors {
        if !unique_colors.contains_key(rgba) {
            let idx = unique_colors.len();
            unique_colors.insert(*rgba, idx);
        }
    }

    // <m:basematerials id="1">
    let mut bm_elem = BytesStart::new("m:basematerials");
    bm_elem.push_attribute(("id", "1"));
    writer
        .write_event(Event::Start(bm_elem))
        .map_err(|e| e.to_string())?;

    // Sort colors by index for deterministic output
    let mut color_entries: Vec<_> = unique_colors.iter().collect();
    color_entries.sort_by_key(|(_, &idx)| idx);

    for (rgba, idx) in &color_entries {
        let mut base_elem = BytesStart::new("m:base");
        let color_str = format!("#{:02X}{:02X}{:02X}", rgba[0], rgba[1], rgba[2]);
        base_elem.push_attribute(("name", format!("color_{}", idx).as_str()));
        base_elem.push_attribute(("displaycolor", color_str.as_str()));
        writer
            .write_event(Event::Empty(base_elem))
            .map_err(|e| e.to_string())?;
    }

    writer
        .write_event(Event::End(BytesEnd::new("m:basematerials")))
        .map_err(|e| e.to_string())?;

    // <object id="2" type="model" m:basematerials="1">
    let mut obj_elem = BytesStart::new("object");
    obj_elem.push_attribute(("id", "2"));
    obj_elem.push_attribute(("type", "model"));
    obj_elem.push_attribute(("m:basematerials", "1"));
    writer
        .write_event(Event::Start(obj_elem))
        .map_err(|e| e.to_string())?;

    // <mesh>
    writer
        .write_event(Event::Start(BytesStart::new("mesh")))
        .map_err(|e| e.to_string())?;

    // <vertices>
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

    // <triangles>
    writer
        .write_event(Event::Start(BytesStart::new("triangles")))
        .map_err(|e| e.to_string())?;
    for (i, face) in mesh.faces.iter().enumerate() {
        let color_idx = unique_colors[&mesh.face_colors[i]];
        let mut tri_elem = BytesStart::new("triangle");
        tri_elem.push_attribute(("v1", face[0].to_string().as_str()));
        tri_elem.push_attribute(("v2", face[1].to_string().as_str()));
        tri_elem.push_attribute(("v3", face[2].to_string().as_str()));
        tri_elem.push_attribute(("pid", "1"));
        tri_elem.push_attribute(("p1", color_idx.to_string().as_str()));
        writer
            .write_event(Event::Empty(tri_elem))
            .map_err(|e| e.to_string())?;
    }
    writer
        .write_event(Event::End(BytesEnd::new("triangles")))
        .map_err(|e| e.to_string())?;

    // </mesh>
    writer
        .write_event(Event::End(BytesEnd::new("mesh")))
        .map_err(|e| e.to_string())?;

    // </object>
    writer
        .write_event(Event::End(BytesEnd::new("object")))
        .map_err(|e| e.to_string())?;

    // </resources>
    writer
        .write_event(Event::End(BytesEnd::new("resources")))
        .map_err(|e| e.to_string())?;

    // <build>
    writer
        .write_event(Event::Start(BytesStart::new("build")))
        .map_err(|e| e.to_string())?;
    let mut item_elem = BytesStart::new("item");
    item_elem.push_attribute(("objectid", "2"));
    writer
        .write_event(Event::Empty(item_elem))
        .map_err(|e| e.to_string())?;
    writer
        .write_event(Event::End(BytesEnd::new("build")))
        .map_err(|e| e.to_string())?;

    // </model>
    writer
        .write_event(Event::End(BytesEnd::new("model")))
        .map_err(|e| e.to_string())?;

    let result = writer.into_inner().into_inner();
    String::from_utf8(result).map_err(|e| format!("UTF-8 error: {}", e))
}

fn build_content_types() -> String {
    r#"<?xml version="1.0" encoding="UTF-8"?>
<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">
  <Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>
  <Default Extension="model" ContentType="application/vnd.ms-package.3dmanufacturing-3dmodel+xml"/>
</Types>"#
        .to_string()
}

fn build_rels() -> String {
    r#"<?xml version="1.0" encoding="UTF-8"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Target="/3D/3dmodel.model" Id="rel0" Type="http://schemas.microsoft.com/3dmanufacturing/2013/01/3dmodel"/>
</Relationships>"#
        .to_string()
}
