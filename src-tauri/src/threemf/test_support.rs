#![cfg(test)]
//! Test-only helpers shared by the 3MF reader tests: a real-file paint scanner
//! and a minimal synthetic-3MF builder.
//!
//! The builder is deliberately not a 3MF library. It writes the handful of
//! parts the reader tests need and nothing else; every fixture is a few lines
//! of XML in the test that owns it.

use std::fmt::Write as _;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipWriter};

/// A synthetic 3MF written to a unique temp path, removed on drop.
pub struct Fixture {
    path: PathBuf,
}

impl Fixture {
    pub fn new(entries: &[(&str, Vec<u8>)]) -> Fixture {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let path = std::env::temp_dir().join(format!(
            "cym-threemf-{}-{}.3mf",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        write_3mf(&path, entries);
        Fixture { path }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

/// Write an OPC/ZIP package from `(part name, bytes)` pairs.
pub fn write_3mf(path: &Path, entries: &[(&str, Vec<u8>)]) {
    let file =
        std::fs::File::create(path).unwrap_or_else(|e| panic!("create {}: {e}", path.display()));
    let mut zip = ZipWriter::new(file);
    let options = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
    for (name, bytes) in entries {
        zip.start_file(*name, options)
            .unwrap_or_else(|e| panic!("zip entry {name}: {e}"));
        zip.write_all(bytes)
            .unwrap_or_else(|e| panic!("write {name}: {e}"));
    }
    zip.finish()
        .unwrap_or_else(|e| panic!("finish {}: {e}", path.display()));
}

/// A complete `3D/3dmodel.model` document around a `<resources>` and a
/// `<build>` body.
pub fn model_xml(unit: &str, resources: &str, build: &str) -> Vec<u8> {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<model unit="{unit}" xml:lang="en-US" xmlns="http://schemas.microsoft.com/3dmanufacturing/core/2015/02">
 <resources>{resources}</resources>
 <build>{build}</build>
</model>"#
    )
    .into_bytes()
}

/// One inline-mesh `<object>`. `paint[i]` is the raw `paint_color` of
/// `faces[i]`; a missing entry means the triangle carries no paint attribute.
pub fn mesh_object(
    id: u32,
    name: &str,
    vertices: &[[f32; 3]],
    faces: &[[u32; 3]],
    paint: &[Option<&str>],
) -> String {
    let mut xml = format!(r#"<object id="{id}" name="{name}" type="model"><mesh><vertices>"#);
    for v in vertices {
        write!(xml, r#"<vertex x="{}" y="{}" z="{}"/>"#, v[0], v[1], v[2])
            .expect("writing to a String cannot fail");
    }
    xml.push_str("</vertices><triangles>");
    for (index, face) in faces.iter().enumerate() {
        match paint.get(index).copied().flatten() {
            Some(paint) => write!(
                xml,
                r#"<triangle v1="{}" v2="{}" v3="{}" paint_color="{paint}"/>"#,
                face[0], face[1], face[2]
            ),
            None => write!(
                xml,
                r#"<triangle v1="{}" v2="{}" v3="{}"/>"#,
                face[0], face[1], face[2]
            ),
        }
        .expect("writing to a String cannot fail");
    }
    xml.push_str("</triangles></mesh></object>");
    xml
}

/// `Metadata/model_settings.config` from an object block body and plate blocks
/// built with [`plate`].
pub fn model_settings(objects: &str, plates: &str) -> Vec<u8> {
    format!("<config>{objects}{plates}</config>").into_bytes()
}

/// One `<plate>` listing `object_ids` on `plater_id`.
pub fn plate(plater_id: u32, name: &str, object_ids: &[u32]) -> String {
    let mut xml = format!(r#"<plate><metadata key="plater_id" value="{plater_id}"/>"#);
    if !name.is_empty() {
        write!(xml, r#"<metadata key="plater_name" value="{name}"/>"#)
            .expect("writing to a String cannot fail");
    }
    for id in object_ids {
        write!(
            xml,
            r#"<model_instance><metadata key="object_id" value="{id}"/></model_instance>"#
        )
        .expect("writing to a String cannot fail");
    }
    xml.push_str("</plate>");
    xml
}

/// `Metadata/project_settings.config` with a filament palette and, optionally,
/// a rectangular `printable_area` of `bed` millimetres.
pub fn project_settings(colours: &[&str], bed: Option<(f32, f32)>) -> Vec<u8> {
    let mut config = serde_json::json!({ "filament_colour": colours });
    if let Some((width, depth)) = bed {
        config["printable_area"] = serde_json::json!([
            "0x0",
            format!("{width}x0"),
            format!("{width}x{depth}"),
            format!("0x{depth}"),
        ]);
    }
    serde_json::to_vec(&config).expect("test project settings are plain JSON")
}

/// Every `paint_color="..."` value found in every `.model` part of a 3MF, in
/// file order and with duplicates kept.
///
/// A plain byte scan is enough here: the tests only need the distinct values,
/// not a real XML parse. Panics on I/O errors because this is test support.
pub fn paint_colors_in(path: &Path) -> Vec<String> {
    let file = std::fs::File::open(path).unwrap_or_else(|e| panic!("open {}: {e}", path.display()));
    let mut zip = zip::ZipArchive::new(file)
        .unwrap_or_else(|e| panic!("{} is not a zip archive: {e}", path.display()));

    let mut values = Vec::new();
    for index in 0..zip.len() {
        let mut entry = zip
            .by_index(index)
            .unwrap_or_else(|e| panic!("zip entry {index}: {e}"));
        let name = entry.name().to_string();
        if !name.ends_with(".model") {
            continue;
        }
        let mut bytes = Vec::new();
        entry
            .read_to_end(&mut bytes)
            .unwrap_or_else(|e| panic!("read {name}: {e}"));
        values.extend(scan_paint_colors(&bytes, &name));
    }
    values
}

const NEEDLE: &[u8] = b"paint_color=\"";

fn scan_paint_colors(bytes: &[u8], part: &str) -> Vec<String> {
    let mut values = Vec::new();
    let mut cursor = 0;
    while cursor + NEEDLE.len() <= bytes.len() {
        if &bytes[cursor..cursor + NEEDLE.len()] == NEEDLE {
            let start = cursor + NEEDLE.len();
            let Some(len) = bytes[start..].iter().position(|&b| b == b'"') else {
                break;
            };
            let value = std::str::from_utf8(&bytes[start..start + len])
                .unwrap_or_else(|e| panic!("paint_color in {part} is not UTF-8: {e}"));
            values.push(value.to_string());
            cursor = start + len + 1;
        } else {
            cursor += 1;
        }
    }
    values
}
