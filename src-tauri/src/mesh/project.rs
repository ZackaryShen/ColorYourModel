//! .cym project format (0.2.0-P1) — a ZIP container holding the full project
//! state: geometry, per-face segment labels, per-face paint colors, region
//! names, and the manual-label high-water mark.
//!
//! # Layout (format_version 1)
//!
//! ```text
//! project.json   JSON: format_version / app / unit / vertex_count /
//!                face_count / next_manual_label / segment_names
//! model.bin      magic "CYMOBJ1\x01" + u32 vertex_count + u32 face_count,
//!                then little-endian: vertices f32×3V, faces u32×3F,
//!                segment_labels u32×F, face_colors u8×4F
//! ```
//!
//! # Design decisions (adversarial-review round, journal iter 53)
//!
//! - **Flat single-object schema.** No `objects[]`/`transform` reservation:
//!   MeshModel has no transform anywhere in the stack, and a reserved-but-
//!   ignored field turns forward compatibility into silent data loss (a v2
//!   file with a real transform would open "fine" in v1 and render wrong).
//!   Multi-object support is a major version bump away, guarded by the
//!   version gate below.
//! - **Never trust the file.** Every section length is checked against the
//!   ZIP entry's uncompressed size *before* any allocation that depends on a
//!   file-supplied count (the whole entry is read into a buffer bounded by
//!   what is physically in the file), and every face index is range-checked
//!   before the rebuild pipeline touches `vertices[]` — the mesh-Mutex
//!   poisoning history (see loader.rs notes) must not repeat here.
//! - **`next_manual_label` is normalized, not trusted**: clamped up to
//!   `max(live manual label) + 1` so a corrupted file can only *lose* retired
//!   names, never collide with live ones (`alloc_manual_label` already floors
//!   at the live max).
//! - **Derived state is rebuilt**, never stored: normals, bbox, segments,
//!   both KD-trees, adjacency. The undo history is deliberately not
//!   persisted (loading resets it, same semantics as auto-segmentation).

use std::collections::HashMap;
use std::io::{Read, Write};
use std::path::Path;

use serde::{Deserialize, Serialize};

use super::model::{MeshModel, MANUAL_SEGMENT_OFFSET};

/// Current .cym format version. A mismatched major version is rejected.
pub const CYM_FORMAT_VERSION: u32 = 1;

const PROJECT_ENTRY: &str = "project.json";
const MODEL_ENTRY: &str = "model.bin";
const MODEL_MAGIC: &[u8; 8] = b"CYMOBJ1\x01";
const MODEL_HEADER_BYTES: usize = 8 + 4 + 4; // magic + vertex_count + face_count

/// project.json payload. Unknown fields are ignored on read (serde default),
/// so older builds tolerate newer minor additions.
#[derive(Debug, Serialize, Deserialize)]
pub struct CymProject {
    pub format_version: u32,
    pub app: String,
    pub unit: String,
    pub vertex_count: u32,
    pub face_count: u32,
    pub next_manual_label: u32,
    #[serde(default)]
    pub segment_names: HashMap<u32, String>,
}

/// Serialize the model's paint/label state into the .cym container at `path`.
pub fn save_cym(mesh: &MeshModel, path: &Path) -> Result<(), String> {
    let project = CymProject {
        format_version: CYM_FORMAT_VERSION,
        app: "ColorYourModel".to_string(),
        unit: mesh.unit.clone(),
        vertex_count: mesh.vertices.len() as u32,
        face_count: mesh.faces.len() as u32,
        next_manual_label: mesh.next_manual_label,
        segment_names: mesh.segment_names.clone(),
    };
    let project_json = serde_json::to_string_pretty(&project)
        .map_err(|e| format!("project: serialize project.json: {e}"))?;

    let mut bin: Vec<u8> = Vec::with_capacity(
        MODEL_HEADER_BYTES
            + mesh.vertices.len() * 12
            + mesh.faces.len() * 12
            + mesh.segment_labels.len() * 4
            + mesh.face_colors.len() * 4,
    );
    bin.extend_from_slice(MODEL_MAGIC);
    bin.extend_from_slice(&(mesh.vertices.len() as u32).to_le_bytes());
    bin.extend_from_slice(&(mesh.faces.len() as u32).to_le_bytes());
    for v in &mesh.vertices {
        for c in v {
            bin.extend_from_slice(&c.to_le_bytes());
        }
    }
    for f in &mesh.faces {
        for idx in f {
            bin.extend_from_slice(&idx.to_le_bytes());
        }
    }
    for l in &mesh.segment_labels {
        bin.extend_from_slice(&l.to_le_bytes());
    }
    for c in &mesh.face_colors {
        bin.extend_from_slice(c);
    }

    let file = std::fs::File::create(path)
        .map_err(|e| format!("project: create file: {e}"))?;
    let mut zip = zip::ZipWriter::new(file);
    let options: zip::write::SimpleFileOptions = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);
    zip.start_file(PROJECT_ENTRY, options)
        .map_err(|e| format!("project: zip write {PROJECT_ENTRY}: {e}"))?;
    zip.write_all(project_json.as_bytes())
        .map_err(|e| format!("project: zip write {PROJECT_ENTRY}: {e}"))?;
    zip.start_file(MODEL_ENTRY, options)
        .map_err(|e| format!("project: zip write {MODEL_ENTRY}: {e}"))?;
    zip.write_all(&bin)
        .map_err(|e| format!("project: zip write {MODEL_ENTRY}: {e}"))?;
    zip.finish()
        .map_err(|e| format!("project: zip finish: {e}"))?;
    Ok(())
}

/// Load a .cym project, rebuilding all derived state. The returned model has
/// a fresh (empty) undo history.
pub fn load_cym(path: &Path) -> Result<MeshModel, String> {
    let file =
        std::fs::File::open(path).map_err(|e| format!("project: open file: {e}"))?;
    let mut archive =
        zip::ZipArchive::new(file).map_err(|e| format!("project: read zip: {e}"))?;

    let project_json = {
        let mut f = archive
            .by_name(PROJECT_ENTRY)
            .map_err(|_| "project: missing entry project.json".to_string())?;
        let mut s = String::new();
        f.read_to_string(&mut s)
            .map_err(|e| format!("project: read project.json: {e}"))?;
        s
    };
    let project: CymProject = serde_json::from_str(&project_json)
        .map_err(|e| format!("project: parse project.json: {e}"))?;
    if project.format_version != CYM_FORMAT_VERSION {
        return Err(format!(
            "project: unsupported format version {} (expected {})",
            project.format_version, CYM_FORMAT_VERSION
        ));
    }

    // Read the whole model entry into a buffer bounded by what is physically
    // in the file (read_to_end never pre-allocates from file-supplied counts),
    // then validate the layout against exact expected sizes BEFORE slicing —
    // a corrupted count can only produce an Err, never an oversized alloc or
    // an index panic.
    let bin = {
        let mut f = archive
            .by_name(MODEL_ENTRY)
            .map_err(|_| "project: missing entry model.bin".to_string())?;
        let mut buf = Vec::new();
        f.read_to_end(&mut buf)
            .map_err(|e| format!("project: read model.bin: {e}"))?;
        buf
    };
    if bin.len() < MODEL_HEADER_BYTES || &bin[0..8] != MODEL_MAGIC {
        return Err("project: model.bin bad magic".to_string());
    }
    let vertex_count =
        u32::from_le_bytes(bin[8..12].try_into().unwrap()) as usize;
    let face_count = u32::from_le_bytes(bin[12..16].try_into().unwrap()) as usize;
    let expected = MODEL_HEADER_BYTES
        + vertex_count * 12
        + face_count * 12
        + face_count * 4
        + face_count * 4;
    if bin.len() != expected {
        return Err(format!(
            "project: model.bin size mismatch (expected {expected}, got {})",
            bin.len()
        ));
    }

    let mut offset = MODEL_HEADER_BYTES;
    let mut read_f32x3 = |buf: &[u8], offset: &mut usize| -> [f32; 3] {
        let v: [f32; 3] = [
            f32::from_le_bytes(buf[*offset..*offset + 4].try_into().unwrap()),
            f32::from_le_bytes(buf[*offset + 4..*offset + 8].try_into().unwrap()),
            f32::from_le_bytes(buf[*offset + 8..*offset + 12].try_into().unwrap()),
        ];
        *offset += 12;
        v
    };

    let mut vertices = Vec::with_capacity(vertex_count);
    for _ in 0..vertex_count {
        vertices.push(read_f32x3(&bin, &mut offset));
    }
    let mut faces = Vec::with_capacity(face_count);
    for _ in 0..face_count {
        let mut f = [0u32; 3];
        for idx in &mut f {
            *idx = u32::from_le_bytes(bin[offset..offset + 4].try_into().unwrap());
            offset += 4;
        }
        faces.push(f);
    }
    let mut segment_labels = Vec::with_capacity(face_count);
    for _ in 0..face_count {
        segment_labels.push(u32::from_le_bytes(
            bin[offset..offset + 4].try_into().unwrap(),
        ));
        offset += 4;
    }
    let mut face_colors = Vec::with_capacity(face_count);
    for _ in 0..face_count {
        face_colors.push([
            bin[offset],
            bin[offset + 1],
            bin[offset + 2],
            bin[offset + 3],
        ]);
        offset += 4;
    }

    // Face indices must be in range or the rebuild pipeline (normals, face
    // centers) would index out of bounds — an Err here, never a panic.
    let vertex_len = vertices.len() as u32;
    if let Some(bad) = faces
        .iter()
        .flat_map(|f| f.iter())
        .find(|idx| **idx >= vertex_len)
    {
        return Err(format!("project: face index out of range ({bad} >= {vertex_len})"));
    }

    let mut mesh = MeshModel::new();
    mesh.vertices = vertices;
    mesh.faces = faces;
    mesh.face_colors = face_colors;
    mesh.segment_labels = segment_labels;
    mesh.segment_names = project.segment_names;
    mesh.unit = project.unit;

    // Normalize the high-water mark: a truncated/corrupted stored value must
    // never collide with a live manual label. Live manual labels are >=
    // MANUAL_SEGMENT_OFFSET by construction.
    let live_max = mesh
        .segment_labels
        .iter()
        .filter(|l| **l >= MANUAL_SEGMENT_OFFSET)
        .copied()
        .max()
        .unwrap_or(MANUAL_SEGMENT_OFFSET - 1);
    mesh.next_manual_label = project.next_manual_label.max(live_max + 1);

    // Rebuild every derived state, same order as loader.rs (normals → bbox →
    // segments → indices). face_colors/segment_labels come from the file, so
    // init_default_colors is deliberately skipped.
    mesh.compute_normals();
    mesh.compute_bbox();
    mesh.rebuild_segments();
    mesh.build_kdtree();
    mesh.build_vertex_kdtree();
    mesh.build_adjacency();
    Ok(mesh)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mesh::model::MeshModel;

    /// A 4-face tetrahedron with manual regions, a region name, paint on one
    /// face, and a bumped high-water mark — everything the format must carry.
    fn painted_model() -> MeshModel {
        let mut m = MeshModel::new();
        m.vertices = vec![[0.0, 0.0, 0.0], [10.0, 0.0, 0.0], [0.0, 10.0, 0.0], [0.0, 0.0, 10.0]];
        m.faces = vec![[0, 2, 1], [0, 1, 3], [0, 3, 2], [1, 2, 3]];
        m.compute_normals();
        m.compute_bbox();
        m.init_default_colors();
        m.build_kdtree();
        m.build_vertex_kdtree();
        m.build_adjacency();
        m.segment_labels = vec![0u32; m.faces.len()];
        // Paint face 1 red-ish and assign faces 2..4 to a manual region.
        m.face_colors[1] = [200, 30, 30, 255];
        let manual = MANUAL_SEGMENT_OFFSET + 7;
        for (i, l) in m.segment_labels.iter_mut().enumerate() {
            if i >= 2 {
                *l = manual;
            }
        }
        m.segment_names.insert(manual, "手动分区甲".to_string());
        m.next_manual_label = manual + 1;
        m
    }

    fn tmp_path(name: &str) -> std::path::PathBuf {
        let mut p = std::env::temp_dir();
        p.push(format!("cym_project_test_{}_{}", std::process::id(), name));
        p
    }

    #[test]
    fn round_trip_preserves_geometry_paint_labels_names_and_high_water() {
        let model = painted_model();
        let path = tmp_path("roundtrip.cym");
        save_cym(&model, &path).expect("save");

        let loaded = load_cym(&path).expect("load");
        assert_eq!(loaded.vertices, model.vertices);
        assert_eq!(loaded.faces, model.faces);
        assert_eq!(loaded.face_colors, model.face_colors);
        assert_eq!(loaded.segment_labels, model.segment_labels);
        assert_eq!(loaded.segment_names, model.segment_names);
        assert_eq!(loaded.next_manual_label, model.next_manual_label);
        assert_eq!(loaded.unit, model.unit);
        // Derived state rebuilt, not restored.
        assert_eq!(loaded.normals.len(), model.faces.len());
        assert_eq!(loaded.segments.len(), 2);
        let manual = MANUAL_SEGMENT_OFFSET + 7;
        let seg = loaded.segments.get(&manual).expect("manual segment rebuilt");
        assert_eq!(seg.name, "手动分区甲");
        assert_eq!(seg.face_count, 2);
        // History is deliberately fresh.
        assert!(!loaded.history.can_undo());

        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn load_rejects_wrong_format_version() {
        let model = painted_model();
        let path = tmp_path("badversion.cym");
        save_cym(&model, &path).expect("save");
        // Rewrite project.json with a future version. Read model.bin BEFORE
        // re-creating the file (File::create truncates immediately).
        let bin = {
            let file = std::fs::File::open(&path).unwrap();
            let mut archive = zip::ZipArchive::new(file).unwrap();
            let mut f = archive.by_name(MODEL_ENTRY).unwrap();
            let mut b = Vec::new();
            std::io::Read::read_to_end(&mut f, &mut b).unwrap();
            b
        };
        let file = std::fs::File::create(&path).unwrap();
        let mut writer = zip::ZipWriter::new(file);
        let opts = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated);
        let project_json = r#"{"format_version":2,"app":"x","unit":"millimeter","vertex_count":4,"face_count":4,"next_manual_label":100000,"segment_names":{}}"#;
        writer.start_file(PROJECT_ENTRY, opts).unwrap();
        writer.write_all(project_json.as_bytes()).unwrap();
        writer.start_file(MODEL_ENTRY, opts).unwrap();
        writer.write_all(&bin).unwrap();
        writer.finish().unwrap();

        let err = match load_cym(&path) {
            Err(e) => e,
            Ok(_) => panic!("expected unsupported-version error"),
        };
        assert!(err.contains("unsupported format version"), "{err}");
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn load_rejects_corrupted_counts_without_excess_allocation() {
        // Craft model.bin with a huge vertex_count but a tiny body: the size
        // precheck must reject it before any count-based allocation.
        let mut bin = MODEL_MAGIC.to_vec();
        bin.extend_from_slice(&0xFFFF_FFF0u32.to_le_bytes()); // vertex_count
        bin.extend_from_slice(&4u32.to_le_bytes()); // face_count
        bin.extend_from_slice(&[0u8; 16]); // a few junk bytes
        let path = tmp_path("oom.cym");
        let file = std::fs::File::create(&path).unwrap();
        let mut writer = zip::ZipWriter::new(file);
        let opts = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated);
        writer
            .start_file(PROJECT_ENTRY, opts)
            .unwrap();
        writer
            .write_all(br#"{"format_version":1,"app":"x","unit":"millimeter","vertex_count":0,"face_count":0,"next_manual_label":100000}"#)
            .unwrap();
        writer.start_file(MODEL_ENTRY, opts).unwrap();
        writer.write_all(&bin).unwrap();
        writer.finish().unwrap();

        let err = match load_cym(&path) {
            Err(e) => e,
            Ok(_) => panic!("expected size mismatch error"),
        };
        assert!(err.contains("size mismatch"), "{err}");
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn load_rejects_face_index_out_of_range() {
        let model = painted_model();
        let path = tmp_path("badindex.cym");
        save_cym(&model, &path).expect("save");
        // Flip one face index to 9999 (>= vertex_count=4).
        let file = std::fs::File::open(&path).unwrap();
        let mut archive = zip::ZipArchive::new(file).unwrap();
        let mut bin = {
            let mut f = archive.by_name(MODEL_ENTRY).unwrap();
            let mut b = Vec::new();
            std::io::Read::read_to_end(&mut f, &mut b).unwrap();
            b
        };
        let first_face = MODEL_HEADER_BYTES + 4 * 12; // skip all 4 vertices
        bin[first_face..first_face + 4].copy_from_slice(&9999u32.to_le_bytes());
        drop(archive);

        let file = std::fs::File::create(&path).unwrap();
        let mut writer = zip::ZipWriter::new(file);
        let opts = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated);
        let project_json = r#"{"format_version":1,"app":"x","unit":"millimeter","vertex_count":4,"face_count":4,"next_manual_label":100000,"segment_names":{}}"#;
        writer.start_file(PROJECT_ENTRY, opts).unwrap();
        writer.write_all(project_json.as_bytes()).unwrap();
        writer.start_file(MODEL_ENTRY, opts).unwrap();
        writer.write_all(&bin).unwrap();
        writer.finish().unwrap();

        let err = match load_cym(&path) {
            Err(e) => e,
            Ok(_) => panic!("expected face-index error"),
        };
        assert!(err.contains("face index out of range"), "{err}");
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn load_tolerates_unknown_project_fields() {
        let model = painted_model();
        let path = tmp_path("unknown.cym");
        save_cym(&model, &path).expect("save");
        let file = std::fs::File::open(&path).unwrap();
        let mut archive = zip::ZipArchive::new(file).unwrap();
        let mut bin = {
            let mut f = archive.by_name(MODEL_ENTRY).unwrap();
            let mut b = Vec::new();
            std::io::Read::read_to_end(&mut f, &mut b).unwrap();
            b
        };
        drop(archive);
        let file = std::fs::File::create(&path).unwrap();
        let mut writer = zip::ZipWriter::new(file);
        let opts = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated);
        // A future minor version adds fields the v1 reader has never seen.
        let project_json = r#"{"format_version":1,"app":"x","unit":"millimeter","vertex_count":4,"face_count":4,"next_manual_label":100000,"segment_names":{},"someFutureField":{"nested":true}}"#;
        writer.start_file(PROJECT_ENTRY, opts).unwrap();
        writer.write_all(project_json.as_bytes()).unwrap();
        writer.start_file(MODEL_ENTRY, opts).unwrap();
        writer.write_all(&bin).unwrap();
        writer.finish().unwrap();

        let loaded = load_cym(&path).expect("unknown fields must be tolerated");
        assert_eq!(loaded.faces.len(), 4);
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn saved_project_survives_the_full_chain_into_3mf_export() {
        // Full-chain guard: save → load → export must work end to end (the
        // export reads face_colors, i.e. the paint actually made it through).
        let model = painted_model();
        let cym_path = tmp_path("chain.cym");
        save_cym(&model, &cym_path).expect("save");
        let loaded = load_cym(&cym_path).expect("load");
        assert_eq!(loaded.face_colors[1], [200, 30, 30, 255]);

        let out_path = tmp_path("chain.3mf");
        crate::export::threemf::export_3mf(&loaded, &out_path, None, &|_,_| {})
            .expect("export 3mf from loaded project");
        let meta = std::fs::metadata(&out_path).expect("3mf exists");
        assert!(meta.len() > 0);

        std::fs::remove_file(&cym_path).ok();
        std::fs::remove_file(&out_path).ok();
    }
}
