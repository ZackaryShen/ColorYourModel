use super::loader::{build_mesh_indices, load_stl, ProgressFn};
use super::model::MeshModel;
use super::{exchange_blender, exchange_gltf, exchange_obj, exchange_ply};
use nalgebra::Vector3;
use std::fs::{self, OpenOptions};
use std::io::BufWriter;
use std::path::Path;

pub(crate) const BLENDER_FORMATS: &[&str] = &["fbx", "usd", "usda", "usdc", "usdz"];

pub(crate) fn read_model(
    path: &Path,
    format: &str,
    progress: &ProgressFn,
) -> Result<(MeshModel, bool), String> {
    if format == "stl" {
        return load_stl(path, progress).map(|mesh| (mesh, false));
    }
    progress(0.05, "Reading mesh file...");
    let (vertices, faces, face_colors) = match format {
        "obj" => exchange_obj::read(path)?,
        "ply" => exchange_ply::read(path)?,
        "gltf" | "glb" => exchange_gltf::read(path)?,
        format if BLENDER_FORMATS.contains(&format) => exchange_blender::import(path)?,
        _ => return Err(format!("Unsupported file format: {format}")),
    };
    validate_geometry(&vertices, &faces)?;
    if faces.len() != face_colors.len() {
        return Err("Face colours do not match the imported geometry".into());
    }
    let skipped_texture =
        matches!(format, "gltf" | "glb") && exchange_gltf::has_base_color_texture(path)?;
    progress(0.50, "Initializing mesh...");
    let mut mesh = MeshModel::new();
    mesh.vertices = vertices;
    mesh.faces = faces;
    mesh.face_colors = face_colors;
    mesh.segment_labels.resize(mesh.faces.len(), 0);
    mesh.compute_normals();
    mesh.compute_bbox();
    build_mesh_indices(&mut mesh, progress);
    progress(0.95, "Finalizing...");
    Ok((mesh, skipped_texture))
}

pub(crate) fn write_model(
    mesh: &MeshModel,
    destination: &Path,
    format: &str,
) -> Result<(), String> {
    validate_geometry(&mesh.vertices, &mesh.faces)?;
    if format != "stl" && mesh.face_colors.len() != mesh.faces.len() {
        return Err("Face colours do not match the exported geometry".into());
    }
    if !matches!(format, "stl" | "ply" | "gltf" | "glb") && !BLENDER_FORMATS.contains(&format) {
        return Err(format!("Unsupported export format: {format}"));
    }
    if !destination
        .extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case(format))
    {
        return Err(format!("Choose a .{format} filename for this export"));
    }
    let parent = destination
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let staged = tempfile::Builder::new()
        .prefix(".cym-mesh-")
        .suffix(&format!(".{format}"))
        .tempfile_in(parent)
        .map_err(|error| format!("Could not stage export: {error}"))?
        .into_temp_path();
    if BLENDER_FORMATS.contains(&format) {
        exchange_blender::export(mesh, &staged, format)?;
    } else {
        let mut file =
            BufWriter::new(fs::File::create(&staged).map_err(|error| error.to_string())?);
        match format {
            "stl" => {
                if mesh.normals.len() != mesh.faces.len() {
                    return Err("Face normals do not match the exported geometry".into());
                }
                let triangles =
                    mesh.faces
                        .iter()
                        .zip(&mesh.normals)
                        .map(|(face, normal)| stl_io::Triangle {
                            normal: stl_io::Normal::new(*normal),
                            vertices: face.map(|i| stl_io::Vertex::new(mesh.vertices[i as usize])),
                        });
                stl_io::write_stl(&mut file, triangles).map_err(|error| error.to_string())?;
            }
            "ply" => exchange_ply::write(mesh, &mut file)?,
            "gltf" | "glb" => exchange_gltf::write(mesh, format == "glb", &mut file)?,
            _ => unreachable!("format checked before staging"),
        }
        use std::io::Write;
        file.flush().map_err(|error| error.to_string())?;
    }
    let file = OpenOptions::new()
        .write(true)
        .open(&staged)
        .map_err(|error| format!("Could not inspect staged export: {error}"))?;
    if file.metadata().map_err(|error| error.to_string())?.len() == 0 {
        return Err("Mesh export produced an empty file".into());
    }
    file.sync_all().map_err(|error| error.to_string())?;
    drop(file);
    staged
        .persist(destination)
        .map_err(|error| format!("Could not install exported file: {}", error.error))
}

fn validate_geometry(vertices: &[[f32; 3]], faces: &[[u32; 3]]) -> Result<(), String> {
    if faces.is_empty() {
        return Err("The model contains no mesh faces".into());
    }
    if vertices.len() > u32::MAX as usize || faces.len() > u32::MAX as usize {
        return Err("The model exceeds the mesh index limits".into());
    }
    // Keep f32 cross products and squared norms finite in mesh initialization.
    if vertices.iter().flatten().any(|axis| !(axis.abs() <= 1e9)) {
        return Err("Vertex coordinates are non-finite or exceed mesh precision limits".into());
    }
    if faces
        .iter()
        .flatten()
        .any(|i| *i as usize >= vertices.len())
    {
        return Err("Mesh face index is out of range".into());
    }
    Ok(())
}

/// Project a polygon onto its most stable axis plane and preserve source winding.
pub(crate) fn triangulate_polygon(
    indices: &[u32],
    vertices: &[[f32; 3]],
) -> Result<Vec<[u32; 3]>, String> {
    let ring = if indices.len() > 3 && indices.first() == indices.last() {
        &indices[..indices.len() - 1]
    } else {
        indices
    };
    if !(3..=16_384).contains(&ring.len()) {
        return Err("A face must have between 3 and 16384 vertices".into());
    }
    if ring.iter().any(|i| *i as usize >= vertices.len()) {
        return Err("Face index is out of range".into());
    }
    if let [a, b, c] = *ring {
        return Ok(vec![[a, b, c]]);
    }
    let origin = Vector3::from(vertices[ring[0] as usize].map(f64::from));
    let points: Vec<_> = ring
        .iter()
        .map(|&i| Vector3::from(vertices[i as usize].map(f64::from)) - origin)
        .collect();
    let normal = points
        .iter()
        .zip(points.iter().cycle().skip(1))
        .fold(Vector3::zeros(), |sum, (a, b)| sum + a.cross(b));
    let axis = normal.iamax();
    let area = normal[axis];
    if !area.is_finite() || area == 0.0 {
        return Err("Polygon has no stable interior".into());
    }
    let uv = [(axis + 1) % 3, (axis + 2) % 3];
    let projected: Vec<_> = points.iter().flat_map(|p| uv.map(|axis| p[axis])).collect();
    let triangles = earcutr::earcut(&projected, &[], 2).map_err(|error| error.to_string())?;
    let mut actual_area = 0.0;
    let mut faces = Vec::with_capacity(triangles.len() / 3);
    for triangle in triangles.chunks_exact(3) {
        let (a, b, c) = (triangle[0], triangle[1], triangle[2]);
        let signed = (points[b] - points[a]).cross(&(points[c] - points[a]))[axis];
        actual_area += signed.abs();
        if signed.signum() == area.signum() {
            faces.push([ring[a], ring[b], ring[c]]);
        } else {
            faces.push([ring[a], ring[c], ring[b]]);
        }
    }
    if faces.is_empty() || (actual_area - area.abs()).abs() > area.abs() * 1e-8 {
        return Err("Could not triangulate polygon without changing its area".into());
    }
    Ok(faces)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mesh::loader::load_stl;

    fn sample() -> MeshModel {
        let mut mesh = MeshModel::new();
        mesh.vertices = vec![
            [3.0, 5.0, 7.0],
            [23.0, 5.0, 7.0],
            [3.0, 16.0, 7.0],
            [3.0, 5.0, 11.0],
        ];
        mesh.faces = vec![[0, 1, 2], [0, 3, 1]];
        mesh.face_colors = vec![[29, 81, 173, 255], [207, 48, 61, 255]];
        mesh.compute_normals();
        mesh
    }

    #[test]
    fn stl_binary_has_correct_length_normals_and_exact_geometry_after_reimport() {
        let mesh = sample();
        let directory = tempfile::tempdir().unwrap();
        let destination = directory.path().join("fixture.STL");
        write_model(&mesh, &destination, "stl").unwrap();
        let bytes = std::fs::read(&destination).unwrap();
        assert_eq!(bytes.len(), 84 + 50 * mesh.faces.len());
        assert_eq!(u32::from_le_bytes(bytes[80..84].try_into().unwrap()), 2);
        assert_eq!(f32::from_le_bytes(bytes[92..96].try_into().unwrap()), 1.0);
        assert_eq!(f32::from_le_bytes(bytes[138..142].try_into().unwrap()), 1.0);
        assert_eq!(&bytes[132..134], &[0, 0]);
        assert_eq!(&bytes[182..184], &[0, 0]);
        let reopened = load_stl(&destination, &|_, _| {}).unwrap();
        assert_eq!(reopened.faces, mesh.faces);
        assert_eq!(reopened.vertices, mesh.vertices);
        assert_eq!(reopened.normals, mesh.normals);
        assert_eq!(
            reopened.face_colors,
            vec![super::super::model::DEFAULT_FACE_COLOR; 2]
        );
    }

    #[test]
    fn invalid_output_leaves_the_existing_file_and_directory_contents_intact() {
        let directory = tempfile::tempdir().unwrap();
        let destination = directory.path().join("fixture.stl");
        std::fs::write(&destination, b"prior file").unwrap();
        let mut mesh = sample();
        mesh.faces[0][0] = 41;
        assert!(write_model(&mesh, &destination, "stl").is_err());
        assert_eq!(std::fs::read(&destination).unwrap(), b"prior file");
        assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 1);
    }

    #[test]
    fn mismatched_extension_does_not_replace_a_project() {
        let directory = tempfile::tempdir().unwrap();
        let destination = directory.path().join("active.3mf");
        std::fs::write(&destination, b"existing project").unwrap();
        assert!(write_model(&sample(), &destination, "stl").is_err());
        assert_eq!(std::fs::read(&destination).unwrap(), b"existing project");
    }

    #[test]
    fn obj_import_has_paint_ready_topology_and_ply_export_uses_the_edited_color() {
        let directory = tempfile::tempdir().unwrap();
        let source = directory.path().join("incoming.obj");
        std::fs::write(
            &source,
            "v 3 5 7\nv 23 5 7\nv 3 16 7\nv 3 5 11\nf 1 2 3\nf 1 4 2\n",
        )
        .unwrap();
        let (mut model, skipped_texture) = read_model(&source, "obj", &|_, _| {}).unwrap();
        assert!(!skipped_texture);
        assert_eq!(model.segment_labels, vec![0, 0]);
        assert_eq!(model.face_adjacency.edge_count(), 1);
        assert_eq!(model.bbox.min, [3.0, 5.0, 7.0]);
        assert_eq!(model.bbox.max, [23.0, 16.0, 11.0]);
        model.apply_paint(None, &[(1, [19, 105, 212, 255])]);
        let output = directory.path().join("painted.ply");
        write_model(&model, &output, "ply").unwrap();
        let (painted, _) = read_model(&output, "ply", &|_, _| {}).unwrap();
        assert_eq!(painted.vertices, model.vertices);
        assert_eq!(painted.faces, model.faces);
        assert_eq!(painted.face_adjacency.edge_count(), 1);
        assert_eq!(painted.face_colors, model.face_colors);
    }

    #[test]
    #[ignore = "requires the Blender executable on PATH or in CYM_BLENDER"]
    fn blender_independently_reads_and_writes_stl_and_obj() {
        let directory = tempfile::tempdir().unwrap();
        let mesh = sample();
        let path = directory.path();
        write_model(&mesh, &path.join("cym.stl"), "stl").unwrap();
        crate::export::obj::export_obj(&mesh, &path.join("cym.obj"), &|_, _| {}).unwrap();
        let blender = std::env::var_os("CYM_BLENDER").unwrap_or_else(|| "blender".into());
        let script = r#"
from pathlib import Path
import bpy, math, sys
root = Path(sys.argv[-1])
vertices = [(3, 5, 7), (23, 5, 7), (3, 16, 7), (3, 5, 11)]
faces = [(0, 1, 2), (0, 3, 1)]
colors = [(29, 81, 173), (207, 48, 61)]
def linear(byte):
    value = byte / 255
    return value / 12.92 if value <= .04045 else ((value + .055) / 1.055) ** 2.4
def reset():
    bpy.ops.wm.read_factory_settings(use_empty=True)
def face_key(corners):
    return min(tuple(corners[i:] + corners[:i]) for i in range(len(corners)))
expected = {face_key([vertices[i] for i in face]): color for face, color in zip(faces, colors)}
for form in ["stl", "obj"]:
    reset()
    if form == "stl":
        bpy.ops.wm.stl_import(filepath=str(root / "cym.stl"))
    else:
        bpy.ops.wm.obj_import(filepath=str(root / "cym.obj"), forward_axis="Y", up_axis="Z")
    assert len(bpy.context.scene.objects) == 1
    obj = next(iter(bpy.context.scene.objects))
    assert len(obj.data.polygons) == len(faces)
    for poly in obj.data.polygons:
        coords = [tuple(obj.matrix_world @ obj.data.vertices[i].co) for i in poly.vertices]
        color = expected[face_key(coords)]
        if form == "obj":
            mat = obj.material_slots[poly.material_index].material
            actual = mat.node_tree.nodes.get("Principled BSDF").inputs["Base Color"].default_value
            assert all(math.isclose(actual[i], linear(color[i]), abs_tol=1e-6) for i in range(3))
reset()
mesh = bpy.data.meshes.new("reference")
mesh.from_pydata(vertices, [], faces)
obj = bpy.data.objects.new("reference", mesh)
bpy.context.scene.collection.objects.link(obj)
for i, color in enumerate(colors):
    material = bpy.data.materials.new(f"Material {i}")
    material.use_nodes = True
    material.node_tree.nodes.get("Principled BSDF").inputs["Base Color"].default_value = (*map(linear, color), 1)
    mesh.materials.append(material)
    mesh.polygons[i].material_index = i
bpy.ops.wm.stl_export(filepath=str(root / "blender.stl"), ascii_format=True)
bpy.ops.wm.obj_export(filepath=str(root / "blender.obj"), forward_axis="Y", up_axis="Z")
"#;
        let output = std::process::Command::new(blender)
            .args([
                "--background",
                "--factory-startup",
                "--python-exit-code",
                "1",
                "--python-expr",
                script,
                "--",
            ])
            .arg(path)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr),
        );
        for format in ["stl", "obj"] {
            let (reference, _) =
                read_model(&path.join(format!("blender.{format}")), format, &|_, _| {}).unwrap();
            assert_eq!(reference.bbox.min, [3.0, 5.0, 7.0]);
            assert_eq!(reference.bbox.max, [23.0, 16.0, 11.0]);
            let directed_faces = |model: &MeshModel| {
                model
                    .faces
                    .iter()
                    .map(|face| {
                        let [a, b, c] = face.map(|i| model.vertices[i as usize].map(|x| x as i32));
                        [[a, b, c], [b, c, a], [c, a, b]].into_iter().min().unwrap()
                    })
                    .collect::<std::collections::BTreeSet<_>>()
            };
            assert_eq!(
                directed_faces(&reference),
                directed_faces(&mesh),
                "{format}"
            );
            if format == "obj" {
                assert_eq!(reference.face_colors, mesh.face_colors);
            }
        }
    }
}
