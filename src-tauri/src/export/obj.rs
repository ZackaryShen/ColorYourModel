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
use crate::mesh::model::MeshModel;
use std::collections::BTreeMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

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

    if palette.len() > MAX_OBJ_COLORS {
        let mut counts = BTreeMap::new();
        for rgba in &mesh.face_colors {
            *counts.entry([rgba[0], rgba[1], rgba[2]]).or_insert(0usize) += 1;
        }
        let histogram: Vec<_> = counts.into_iter().collect();
        palette = quantize_histogram(&histogram, MAX_OBJ_COLORS);
        // OBJ material 255 is valid; passing through a 1-based u8 slicer slot
        // would wrap its value 256 to zero and silently select material 0.
        for (rgb, material) in &mut lookup {
            *material = nearest_palette_index(&palette, *rgb);
        }
    }
    let face_material = mesh
        .face_colors
        .iter()
        .map(|rgba| lookup[&[rgba[0], rgba[1], rgba[2]]])
        .collect();
    (palette, face_material)
}

/// Export mesh with per-face colours to a `.obj` + sibling `.mtl`.
///
/// `output_path` is the exact OBJ destination confirmed by the user. The
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

    if output_path
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("mtl"))
    {
        return Err("OBJ output conflicts with its companion material library".into());
    }
    let obj_path = output_path.to_path_buf();
    let mtl_path = obj_path.with_extension("mtl");
    let mtl_name = mtl_path
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or("OBJ material filename is not valid UTF-8")?
        .to_string();

    // --- Material library (.mtl) -------------------------------------------
    let mtl_body = {
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
        body
    };
    progress(0.4, "obj:mtl");

    // --- Geometry (.obj) ---------------------------------------------------
    let obj_body = {
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
    let mut files = [
        StagedOutput::new(&mtl_path, mtl_body.as_bytes())?,
        StagedOutput::new(&obj_path, obj_body.as_bytes())?,
    ];
    commit_pair(&mut files, &|from, to| std::fs::rename(from, to))?;
    progress(0.95, "obj:write");

    Ok(())
}

static OUTPUT_SEQUENCE: AtomicU64 = AtomicU64::new(0);

struct StagedOutput {
    target: PathBuf,
    staged: PathBuf,
    backup: PathBuf,
    staged_owned: bool,
    backed_up: bool,
    installed: bool,
    committed: bool,
}

impl StagedOutput {
    fn new(target: &Path, contents: &[u8]) -> Result<Self, String> {
        match std::fs::symlink_metadata(target) {
            Ok(meta) if !meta.is_file() && !meta.file_type().is_symlink() => {
                return Err(format!(
                    "Export destination is not a file: {}",
                    target.display()
                ));
            }
            Err(error) if error.kind() != std::io::ErrorKind::NotFound => {
                return Err(error.to_string())
            }
            _ => {}
        }
        let id = OUTPUT_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|e| e.to_string())?
            .as_nanos();
        let prefix = format!(".cym-obj-{}-{}-{}", std::process::id(), nonce, id);
        let mut file = Self {
            target: target.to_path_buf(),
            staged: target.with_file_name(format!("{}.tmp", prefix)),
            backup: target.with_file_name(format!("{}.bak", prefix)),
            staged_owned: false,
            backed_up: false,
            installed: false,
            committed: false,
        };
        let mut output = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&file.staged)
            .map_err(|e| e.to_string())?;
        file.staged_owned = true;
        output
            .write_all(contents)
            .and_then(|_| output.sync_all())
            .map_err(|e| e.to_string())?;
        Ok(file)
    }

    fn rollback(&mut self) -> Result<(), String> {
        if self.installed {
            std::fs::remove_file(&self.target).map_err(|e| e.to_string())?;
            self.installed = false;
        }
        if self.backed_up {
            std::fs::rename(&self.backup, &self.target).map_err(|e| {
                format!(
                    "Cannot restore {}; old file retained at {}: {}",
                    self.target.display(),
                    self.backup.display(),
                    e
                )
            })?;
            self.backed_up = false;
        }
        Ok(())
    }
}

impl Drop for StagedOutput {
    fn drop(&mut self) {
        if !self.committed {
            if let Err(error) = self.rollback() {
                log::error!("OBJ rollback: {}", error);
            }
        }
        if self.staged_owned {
            let _ = std::fs::remove_file(&self.staged);
        }
        if self.committed {
            let _ = std::fs::remove_file(&self.backup);
        }
    }
}

// Stage both files before replacing either. This is failure-safe, not a
// cross-file atomic transaction against process or machine crashes.
fn commit_pair(
    files: &mut [StagedOutput; 2],
    rename: &dyn Fn(&Path, &Path) -> std::io::Result<()>,
) -> Result<(), String> {
    let result = (|| {
        for file in files.iter_mut() {
            match std::fs::symlink_metadata(&file.target) {
                Ok(meta) if !meta.is_file() && !meta.file_type().is_symlink() => {
                    return Err(format!(
                        "Export destination is not a file: {}",
                        file.target.display()
                    ))
                }
                Ok(_) => {
                    std::fs::rename(&file.target, &file.backup).map_err(|e| e.to_string())?;
                    file.backed_up = true;
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(error.to_string()),
            }
        }
        for file in files.iter_mut() {
            rename(&file.staged, &file.target)
                .map_err(|e| format!("Failed to install {}: {}", file.target.display(), e))?;
            file.installed = true;
        }
        Ok(())
    })();
    if let Err(mut error) = result {
        for file in files.iter_mut().rev() {
            if let Err(restore) = file.rollback() {
                error.push_str(&format!("; {}", restore));
            }
        }
        return Err(error);
    }
    for file in files {
        file.committed = true;
    }
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
            let distance = |rgb: &[u8; 3]| -> i32 {
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
            ("extensionless", "extensionless.mtl"),
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
    fn geometry_write_failure_preserves_the_existing_material_library() {
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
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_second_install_failure_restores_both_old_files_and_removes_temporary_files() {
        let dir = std::env::temp_dir().join(format!("cym-obj-rollback-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let mtl = dir.join("project.mtl");
        let obj = dir.join("project.obj");
        std::fs::write(&mtl, "old materials").unwrap();
        std::fs::write(&obj, "old geometry").unwrap();
        let mut files = [
            StagedOutput::new(&mtl, b"new materials").unwrap(),
            StagedOutput::new(&obj, b"new geometry").unwrap(),
        ];
        let result = commit_pair(&mut files, &|from, to| {
            if to == obj {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::PermissionDenied,
                    "injected geometry install failure",
                ));
            }
            std::fs::rename(from, to)
        });
        assert!(result.is_err());
        assert_eq!(std::fs::read_to_string(&mtl).unwrap(), "old materials");
        assert_eq!(std::fs::read_to_string(&obj).unwrap(), "old geometry");
        drop(files);
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 2);
        std::fs::remove_dir_all(dir).unwrap();
    }
}
