//! Wavefront OBJ writer that carries per-face colour.
//!
//! OBJ has no native per-face colour channel. The portable convention is a
//! companion `.mtl` material library: one `newmtl` entry per distinct colour,
//! referenced from the geometry through `usemtl` groups. A viewer that honours
//! `usemtl` (Blender, MeshLab, most DCC tools) then paints each face with its
//! own RGB.
//!
//! Vertices are written once, shared across colour groups, and referenced by
//! global 1-based index from every `usemtl` block. This is exactly what
//! Blender's own OBJ exporter does: it does not split vertices on material
//! boundaries, and renderers split corners per-face at load time, so colours
//! never bleed between adjacent differently-coloured faces.

use crate::export::quantize::{nearest_palette_index, quantize_histogram};
use crate::mesh::face_colors::srgb_to_linear;
use crate::mesh::model::MeshModel;
use std::collections::BTreeMap;
use std::io::{ErrorKind, Write};
use std::path::Path;

/// Cap on the number of distinct materials written to the `.mtl`.
///
/// A hand-painted model uses a small palette (tens of colours), which we keep
/// exact. A gradient / airbrush job can produce thousands of unique RGB values;
/// beyond this cap we quantise so the `.mtl` stays readable and the OBJ loader
/// does not choke on a huge material list. The visual loss is negligible for a
/// colour-export use case.
const MAX_OBJ_COLORS: usize = 256;

/// Resolve the loaded mesh's per-face colours into a material palette and a
/// parallel `face_material` array (0-based material index per face).
fn build_palette(mesh: &MeshModel) -> (Vec<[u8; 4]>, Vec<usize>) {
    if mesh.face_colors.is_empty() {
        // An unpainted mesh still deserves a file: a single neutral material
        // for every face.
        return (vec![[200, 200, 200, 255]], vec![0; mesh.faces.len()]);
    }

    // Exact unique colours first so small palettes survive untouched.
    let mut palette = Vec::new();
    let mut lookup = BTreeMap::new();
    for &color in &mesh.face_colors {
        lookup.entry(color).or_insert_with(|| {
            let index = palette.len();
            palette.push(color);
            index
        });
    }

    if palette.len() > MAX_OBJ_COLORS {
        let mut counts = BTreeMap::new();
        for &color in &mesh.face_colors {
            *counts.entry(color).or_insert(0usize) += 1;
        }
        let histogram: Vec<_> = counts.into_iter().collect();
        palette = quantize_histogram(&histogram, MAX_OBJ_COLORS);
        // OBJ material 255 is valid; passing through a 1-based u8 slicer slot
        // would wrap its value 256 to zero and silently select material 0.
        for (color, material) in &mut lookup {
            *material = nearest_palette_index(&palette, *color);
        }
    }
    let face_material = mesh.face_colors.iter().map(|color| lookup[color]).collect();
    (palette, face_material)
}

/// Export mesh with per-face colours to a `.obj` + sibling `.mtl`.
///
/// `output_path` is the exact OBJ destination confirmed by the user. The
/// material library is written alongside and its filename is returned.
pub fn export_obj(
    mesh: &MeshModel,
    output_path: &Path,
    progress: &dyn Fn(f32, &str),
) -> Result<String, String> {
    if !output_path
        .extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("obj"))
    {
        return Err("Choose an .obj filename for this export".into());
    }
    let stem = output_path
        .file_stem()
        .and_then(|name| name.to_str())
        .ok_or("OBJ material filename is not valid UTF-8")?;
    if stem.contains(['\r', '\n']) {
        return Err("OBJ material filename cannot contain a line break".into());
    }
    let directory = output_path
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));

    let (palette, face_material) = build_palette(mesh);
    if face_material.len() != mesh.faces.len() {
        return Err(format!(
            "colour mapping produced {} materials for {} faces",
            face_material.len(),
            mesh.faces.len()
        ));
    }
    progress(0.2, "obj:colors");

    // --- Material library (.mtl) -------------------------------------------
    let mtl_body = {
        let mut body = String::new();
        body.push_str("# ColorYourModel material library\n");
        body.push_str("# Kd uses linear sRGB\n");
        body.push_str("# One material per exported face colour.\n\n");
        for (i, &[r, g, b, alpha]) in palette.iter().enumerate() {
            let [r, g, b] = [r, g, b].map(|byte| srgb_to_linear(f32::from(byte) / 255.0));
            body.push_str(&format!("newmtl m{i}\nKd {r:.6} {g:.6} {b:.6}\n",));
            if alpha < 255 {
                body.push_str(&format!("d {:.6}\n", f32::from(alpha) / 255.0));
            }
            body.push('\n');
        }
        body
    };
    progress(0.4, "obj:mtl");

    // --- Geometry (.obj) ---------------------------------------------------
    let obj_body = {
        let mut body = String::new();
        // Vertices. OBJ indices are 1-based; the mesh's vertices are reused
        // verbatim across colour groups.
        for v in &mesh.vertices {
            body.push_str(&format!("v {} {} {}\n", v[0], v[1], v[2]));
        }
        body.push('\n');
        progress(0.6, "obj:vertices");

        // Group faces by material so each `usemtl` block is contiguous.
        let mut by_material: Vec<Vec<usize>> = vec![Vec::new(); palette.len()];
        for (fi, &mi) in face_material.iter().enumerate() {
            by_material[mi].push(fi);
        }
        for (mi, faces) in by_material.iter().enumerate() {
            if faces.is_empty() {
                continue;
            }
            body.push_str(&format!("usemtl m{}\n", mi));
            for &fi in faces {
                let face = mesh.faces[fi];
                // +1: OBJ is 1-based, the mesh stores 0-based indices.
                body.push_str(&format!(
                    "f {} {} {}\n",
                    face[0] + 1,
                    face[1] + 1,
                    face[2] + 1
                ));
            }
        }
        progress(0.9, "obj:faces");

        body
    };
    let mut staged_mtl = stage(directory, &[mtl_body.as_bytes()])?;
    // Only the OBJ went through the overwrite dialog. Use an exclusive
    // install for the MTL, including when two exports pick the same name.
    for index in 0..=u32::MAX {
        let mtl_name = if index == 0 {
            format!("{stem}.mtl")
        } else {
            format!("{stem}.{index}.mtl")
        };
        let mtl_path = directory.join(&mtl_name);
        if mtl_path.symlink_metadata().is_ok() {
            continue;
        }
        let header = format!("# ColorYourModel OBJ export\nmtllib {mtl_name}\n\n");
        let staged_obj = stage(directory, &[header.as_bytes(), obj_body.as_bytes()])?;
        match staged_mtl.persist_noclobber(&mtl_path) {
            Ok(file) => drop(file),
            Err(error) if error.error.kind() == ErrorKind::AlreadyExists => {
                staged_mtl = error.file;
                continue;
            }
            Err(error) => {
                return Err(format!(
                    "Failed to install {}: {}",
                    mtl_path.display(),
                    error.error
                ));
            }
        }
        if let Err(error) = staged_obj.persist(output_path) {
            let mut message = format!(
                "Failed to install {}: {}",
                output_path.display(),
                error.error
            );
            if let Err(cleanup) = std::fs::remove_file(&mtl_path) {
                message.push_str(&format!(
                    "\nFailed to remove {}: {cleanup}",
                    mtl_path.display()
                ));
            }
            return Err(message);
        }
        progress(0.95, "obj:write");
        return Ok(mtl_name);
    }
    Err("No name available for the OBJ material library".into())
}

fn stage(directory: &Path, parts: &[&[u8]]) -> Result<tempfile::NamedTempFile, String> {
    let mut file = tempfile::Builder::new()
        .prefix(".cym-obj-")
        .tempfile_in(directory)
        .map_err(|error| format!("Could not stage OBJ export: {error}"))?;
    for part in parts {
        file.write_all(part).map_err(|error| error.to_string())?;
    }
    file.as_file()
        .sync_all()
        .map_err(|error| error.to_string())?;
    Ok(file)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mesh::model::MeshModel;

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

    #[test]
    fn writes_obj_and_mtl_with_distinct_colour_groups() {
        let mesh = two_face_mesh(vec![[255, 0, 0, 255], [0, 0, 255, 255]]);
        let dir = std::env::temp_dir().join("cym-obj-test");
        std::fs::create_dir_all(&dir).unwrap();
        let obj = dir.join("sample.obj");
        let mtl = dir.join("sample.mtl");

        export_obj(&mesh, &obj, &|_, _| {}).expect("obj export must succeed");

        let obj_text = std::fs::read_to_string(&obj).unwrap();
        let mtl_text = std::fs::read_to_string(&mtl).unwrap();

        // Header + mtllib reference.
        assert!(obj_text.contains("mtllib sample.mtl"));
        // Two material definitions, two usemtl groups.
        assert_eq!(mtl_text.matches("newmtl ").count(), 2);
        assert_eq!(obj_text.matches("usemtl m").count(), 2);
        // Both faces present, 1-based indices.
        assert!(obj_text.contains("f 1 2 3"));
        assert!(obj_text.contains("f 2 4 3"));
        // Kd carries normalised RGB (red -> 1.0 0.0 0.0).
        assert!(mtl_text.contains("Kd 1.000000 0.000000 0.000000"));

        let _ = std::fs::remove_file(&obj);
        let _ = std::fs::remove_file(&mtl);
    }

    #[test]
    fn unpainted_mesh_exports_a_single_neutral_material() {
        let mesh = two_face_mesh(vec![]);
        let dir = tempfile::tempdir().unwrap();
        let obj = dir.path().join("neutral.obj");

        export_obj(&mesh, &obj, &|_, _| {}).expect("neutral obj export must succeed");
        let obj_text = std::fs::read_to_string(&obj).unwrap();
        assert_eq!(obj_text.matches("usemtl m").count(), 1);
    }

    #[test]
    fn exact_small_palette_is_preserved_without_quantisation() {
        // 3 exact colours -> 3 materials, no merging.
        let mesh = two_face_mesh(vec![
            [10, 20, 30, 255],
            [40, 50, 60, 255],
            [10, 20, 30, 255],
            [70, 80, 90, 255],
        ]);
        // faces.len() must match face_colors.len(); pad the mesh.
        let mut mesh = mesh;
        mesh.faces = vec![[0, 1, 2], [1, 3, 2], [0, 1, 2], [1, 3, 2]];
        let (palette, face_material) = build_palette(&mesh);
        assert_eq!(palette.len(), 3);
        assert_eq!(face_material.len(), 4);
    }

    #[test]
    fn quantized_obj_can_reference_the_256th_material_without_wrapping() {
        let colors: Vec<_> = (0..=256u32)
            .map(|i| [(i % 256) as u8, (i / 256) as u8, 0, 255])
            .collect();
        let mut mesh = two_face_mesh(colors);
        mesh.faces = vec![[0, 1, 2]; 257];
        let (palette, materials) = build_palette(&mesh);
        assert_eq!(palette.len(), 256);
        assert!(
            materials.contains(&255),
            "the 256th palette entry must be addressable"
        );
        for (rgba, &material) in mesh.face_colors.iter().zip(&materials) {
            let distance = |rgb: &[u8; 4]| -> i32 {
                (0..3)
                    .map(|i| (i32::from(rgb[i]) - i32::from(rgba[i])).pow(2))
                    .sum()
            };
            let actual_distance = distance(&palette[material]);
            assert!(
                palette
                    .iter()
                    .all(|entry| actual_distance <= distance(entry)),
                "face {:?} was assigned to a non-nearest material {}",
                rgba,
                material
            );
        }
    }

    #[test]
    fn companion_material_path_replaces_the_final_extension_regardless_of_case() {
        let mesh = two_face_mesh(vec![[255, 0, 0, 255], [0, 0, 255, 255]]);
        let dir = std::env::temp_dir().join(format!("cym-obj-case-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        for (name, material) in [
            ("sample.OBJ", "sample.mtl"),
            ("sample.obj.obj", "sample.obj.mtl"),
            ("Sample Name.obj", "Sample Name.mtl"),
        ] {
            let obj = dir.join(name);
            export_obj(&mesh, &obj, &|_, _| {}).unwrap();
            assert!(
                dir.join(material).exists(),
                "expected companion {}",
                material
            );
            let text = std::fs::read_to_string(&obj).unwrap();
            assert!(text
                .lines()
                .any(|line| line == format!("mtllib {}", material)));
        }
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn material_destination_conflict_is_rejected_without_overwriting_the_file() {
        let mesh = two_face_mesh(vec![[255, 0, 0, 255], [0, 0, 255, 255]]);
        let dir = std::env::temp_dir().join(format!("cym-obj-conflict-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        for name in ["project.mtl", "project.MTL"] {
            let target = dir.join(name);
            std::fs::write(&target, "original material library").unwrap();
            assert!(export_obj(&mesh, &target, &|_, _| {}).is_err());
            assert_eq!(
                std::fs::read_to_string(&target).unwrap(),
                "original material library"
            );
        }
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn obj_export_cannot_overwrite_an_existing_3mf() {
        let mesh = two_face_mesh(vec![[255, 0, 0, 255], [0, 0, 255, 255]]);
        let dir = tempfile::tempdir().unwrap();
        let project = dir.path().join("project.3mf");
        std::fs::write(
            &project,
            include_bytes!("../../../examples/3plate_sample.3mf"),
        )
        .unwrap();
        let original = std::fs::read(&project).unwrap();

        assert!(export_obj(&mesh, &project, &|_, _| {}).is_err());
        assert_eq!(std::fs::read(&project).unwrap(), original);
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
    }

    #[test]
    fn obj_export_keeps_existing_companions_and_references_the_new_materials() {
        let mesh = two_face_mesh(vec![[255, 0, 0, 255], [0, 0, 255, 255]]);
        let dir = tempfile::tempdir().unwrap();
        let obj = dir.path().join("project.OBJ");
        let first = dir.path().join("project.mtl");
        let second = dir.path().join("project.1.mtl");
        std::fs::write(&first, "library from another model").unwrap();
        std::fs::write(&second, "another library").unwrap();

        let new_library = export_obj(&mesh, &obj, &|_, _| {}).unwrap();
        assert_eq!(new_library, "project.2.mtl");
        assert_eq!(
            std::fs::read_to_string(first).unwrap(),
            "library from another model"
        );
        assert_eq!(std::fs::read_to_string(second).unwrap(), "another library");
        let data = std::fs::read_to_string(&obj).unwrap();
        let library = data
            .lines()
            .find_map(|line| line.strip_prefix("mtllib "))
            .unwrap();
        assert_eq!(library, new_library);
        let actual = std::fs::read_to_string(dir.path().join(library)).unwrap();
        assert!(actual.contains("newmtl m1"));
    }

    #[test]
    fn geometry_install_failure_preserves_existing_files_and_removes_the_new_library() {
        let mesh = two_face_mesh(vec![[255, 0, 0, 255], [0, 0, 255, 255]]);
        let dir = std::env::temp_dir().join(format!("cym-obj-failure-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("project.obj")).unwrap();
        let mtl = dir.join("project.mtl");
        std::fs::write(&mtl, "original material library").unwrap();
        assert!(export_obj(&mesh, &dir.join("project.obj"), &|_, _| {}).is_err());
        assert_eq!(
            std::fs::read_to_string(mtl).unwrap(),
            "original material library"
        );
        assert!(dir.join("project.obj").is_dir());
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 2);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn replacing_confirmed_obj_keeps_its_previous_library_available() {
        let dir = tempfile::tempdir().unwrap();
        let obj = dir.path().join("painted.obj");
        let mut mesh = two_face_mesh(vec![[10, 20, 30, 255], [255, 0, 0, 255]]);
        let first = export_obj(&mesh, &obj, &|_, _| {}).unwrap();
        let former_materials = std::fs::read(dir.path().join(&first)).unwrap();

        mesh.face_colors[0] = [1, 2, 3, 255];
        let second = export_obj(&mesh, &obj, &|_, _| {}).unwrap();
        assert_ne!(first, second);
        assert_eq!(
            std::fs::read(dir.path().join(first)).unwrap(),
            former_materials
        );
        let model = std::fs::read_to_string(&obj).unwrap();
        assert!(model.lines().any(|line| line == format!("mtllib {second}")));
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 3);
    }
}
