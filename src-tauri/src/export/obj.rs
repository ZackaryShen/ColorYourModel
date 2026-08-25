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

use crate::export::quantize::quantize_face_colors;
use crate::mesh::model::MeshModel;
use std::collections::BTreeMap;
use std::io::Write;
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
fn build_palette(mesh: &MeshModel) -> (Vec<[u8; 3]>, Vec<usize>) {
    if mesh.face_colors.is_empty() {
        // An unpainted mesh still deserves a file: a single neutral material
        // for every face.
        return (vec![[200, 200, 200]], vec![0; mesh.faces.len()]);
    }

    // Exact unique colours first so small palettes survive untouched.
    let mut palette: Vec<[u8; 3]> = Vec::new();
    let mut lookup: BTreeMap<[u8; 3], usize> = BTreeMap::new();
    for rgba in &mesh.face_colors {
        let rgb = [rgba[0], rgba[1], rgba[2]];
        if !lookup.contains_key(&rgb) {
            lookup.insert(rgb, palette.len());
            palette.push(rgb);
        }
    }

    if palette.len() <= MAX_OBJ_COLORS {
        let face_material = mesh
            .face_colors
            .iter()
            .map(|rgba| {
                let rgb = [rgba[0], rgba[1], rgba[2]];
                *lookup.get(&rgb).expect("colour was inserted above")
            })
            .collect();
        (palette, face_material)
    } else {
        // Too many distinct colours — fold to the cap. `quantize_face_colors`
        // returns 1-based slots; convert back to 0-based material indices.
        let q = quantize_face_colors(&mesh.face_colors, MAX_OBJ_COLORS);
        let face_material = q
            .face_slots
            .iter()
            .map(|s| (*s as usize).saturating_sub(1))
            .collect();
        (q.palette, face_material)
    }
}

/// Export mesh with per-face colours to a `.obj` + sibling `.mtl`.
///
/// `output_path` is the `.obj` location (the caller forces the extension). The
/// material library is written alongside as `<stem>.mtl` and referenced via
/// `mtllib`.
pub fn export_obj(
    mesh: &MeshModel,
    output_path: &Path,
    progress: &dyn Fn(f32, &str),
) -> Result<(), String> {
    let (palette, face_material) = build_palette(mesh);
    if face_material.len() != mesh.faces.len() {
        return Err(format!(
            "colour mapping produced {} materials for {} faces",
            face_material.len(),
            mesh.faces.len()
        ));
    }
    progress(0.2, "obj:colors");

    let obj_path = output_path.to_path_buf();
    let mtl_name = obj_path
        .file_name()
        .and_then(|n| n.to_str())
        .map(|n| {
            let stem = n.strip_suffix(".obj").unwrap_or(n);
            format!("{}.mtl", stem)
        })
        .unwrap_or_else(|| "model.mtl".to_string());
    let mtl_path = obj_path.with_file_name(&mtl_name);

    // --- Material library (.mtl) -------------------------------------------
    {
        let mut body = String::new();
        body.push_str("# ColorYourModel material library\n");
        body.push_str("# One material per exported face colour.\n\n");
        for (i, rgb) in palette.iter().enumerate() {
            body.push_str(&format!(
                "newmtl m{}\nKd {:.6} {:.6} {:.6}\n\n",
                i,
                rgb[0] as f32 / 255.0,
                rgb[1] as f32 / 255.0,
                rgb[2] as f32 / 255.0,
            ));
        }
        let mut f = std::fs::File::create(&mtl_path)
            .map_err(|e| format!("Failed to create {}: {}", mtl_path.display(), e))?;
        f.write_all(body.as_bytes())
            .map_err(|e| format!("Failed to write {}: {}", mtl_path.display(), e))?;
    }
    progress(0.4, "obj:mtl");

    // --- Geometry (.obj) ---------------------------------------------------
    {
        let mut body = String::new();
        body.push_str("# ColorYourModel OBJ export\n");
        body.push_str("# Per-face colours are carried via usemtl groups + the sibling .mtl.\n");
        body.push_str(&format!("mtllib {}\n", mtl_name));
        body.push('\n');

        // Vertices. OBJ indices are 1-based; the mesh's vertices are reused
        // verbatim across colour groups.
        for v in &mesh.vertices {
            body.push_str(&format!("v {:.6} {:.6} {:.6}\n", v[0], v[1], v[2]));
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
                body.push_str(&format!("f {} {} {}\n", face[0] + 1, face[1] + 1, face[2] + 1));
            }
        }
        progress(0.9, "obj:faces");

        let mut f = std::fs::File::create(&obj_path)
            .map_err(|e| format!("Failed to create {}: {}", obj_path.display(), e))?;
        f.write_all(body.as_bytes())
            .map_err(|e| format!("Failed to write {}: {}", obj_path.display(), e))?;
    }
    progress(0.95, "obj:write");

    Ok(())
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
        let dir = std::env::temp_dir().join("cym-obj-test");
        std::fs::create_dir_all(&dir).unwrap();
        let obj = dir.join("neutral.obj");

        export_obj(&mesh, &obj, &|_, _| {}).expect("neutral obj export must succeed");
        let obj_text = std::fs::read_to_string(&obj).unwrap();
        assert_eq!(obj_text.matches("usemtl m").count(), 1);

        let _ = std::fs::remove_file(&obj);
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
}
