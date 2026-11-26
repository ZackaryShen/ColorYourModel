use super::exchange::triangulate_polygon;
use super::face_colors::linear_to_srgb;
use super::model::DEFAULT_FACE_COLOR;
use std::collections::HashMap;
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};

const MAX_OBJ_BYTES: u64 = 512 * 1024 * 1024;
const MAX_MTL_BYTES: u64 = 16 * 1024 * 1024;
const MAX_MESH_ELEMENTS: usize = 5_000_000;

pub(crate) fn read(path: &Path) -> Result<(Vec<[f32; 3]>, Vec<[u32; 3]>, Vec<[u8; 4]>), String> {
    let mut vertices = Vec::new();
    let mut vertex_colors = Vec::new();
    let mut faces = Vec::new();
    let mut face_materials = Vec::new();
    let mut materials = HashMap::new();
    let mut material_ids = HashMap::new();
    let mut active_material = None;
    let mut logical_line = String::new();
    let mut ring = Vec::new();
    for (line_no, line) in open_text(path, MAX_OBJ_BYTES)?.lines().enumerate() {
        let line = line.map_err(|error| format!("Could not read OBJ: {error}"))?;
        let (key, rest) = statement(&line);
        let line = if key == "mtllib" && material_path(path, rest).is_some() {
            // An existing companion's filename can include a literal '#'.
            line.trim()
        } else {
            line.split('#').next().unwrap_or("").trim()
        };
        if let Some(prefix) = line.strip_suffix('\\') {
            logical_line.push_str(prefix);
            logical_line.push(' ');
            continue;
        }
        logical_line.push_str(line);
        let (key, rest) = statement(&logical_line);
        let parsed: Result<(), String> = (|| {
            match key {
                "v" => {
                    if vertices.len() >= MAX_MESH_ELEMENTS {
                        return Err("OBJ exceeds the vertex limit".into());
                    }
                    let mut fields = rest.split_whitespace();
                    vertices.push(triple(&mut fields)?);
                    // Some exporters extend a polygon vertex with RGB or RGBA.
                    // One trailing value is the standard homogeneous weight.
                    let extra = std::array::from_fn(|_| fields.next());
                    let color = match extra {
                        [Some(r), Some(g), Some(b), alpha, None] => {
                            let alpha = alpha.map(number).transpose()?.unwrap_or(1.0);
                            Some([
                                unit_byte(number(r)?),
                                unit_byte(number(g)?),
                                unit_byte(number(b)?),
                                unit_byte(alpha),
                            ])
                        }
                        _ => None,
                    };
                    vertex_colors.push(color);
                }
                "f" => {
                    ring.clear();
                    for corner in rest.split_whitespace().take(16_386) {
                        ring.push(vertex_index(corner, vertices.len())?);
                    }
                    let triangle = match ring.as_slice() {
                        [a, b, c] => Some([*a, *b, *c]),
                        _ => None,
                    };
                    let batch;
                    let triangles = if let Some(ref face) = triangle {
                        std::slice::from_ref(face)
                    } else {
                        batch = triangulate_polygon(&ring, &vertices)?;
                        &batch
                    };
                    if faces.len() + triangles.len() > MAX_MESH_ELEMENTS {
                        return Err("OBJ exceeds the triangulated face limit".into());
                    }
                    faces.extend_from_slice(triangles);
                    face_materials.resize(faces.len(), active_material);
                }
                "mtllib" => {
                    for material_path in material_paths(path, rest) {
                        read_mtl(&material_path, &mut materials)?;
                    }
                }
                "usemtl" if !rest.is_empty() => {
                    let next = material_ids.len();
                    active_material = Some(*material_ids.entry(rest.to_owned()).or_insert(next));
                }
                _ => {}
            }
            Ok(())
        })();
        parsed.map_err(|error| format!("OBJ line {}: {error}", line_no + 1))?;
        logical_line.clear();
    }
    if !logical_line.is_empty() {
        return Err("OBJ ends with an unfinished continuation line".into());
    }
    if faces.is_empty() {
        return Err("OBJ has no mesh faces".into());
    }
    let mut palette = vec![None; material_ids.len()];
    for (name, id) in material_ids {
        palette[id] = materials.get(&name).copied();
    }
    let colors = faces
        .iter()
        .zip(face_materials)
        .map(|(face, material)| {
            material.and_then(|id| palette[id]).unwrap_or_else(|| {
                let source = face.map(|i| vertex_colors[i as usize].unwrap_or(DEFAULT_FACE_COLOR));
                std::array::from_fn(|channel| {
                    let sum: u16 = source.iter().map(|color| u16::from(color[channel])).sum();
                    ((sum + 1) / 3) as u8
                })
            })
        })
        .collect();
    Ok((vertices, faces, colors))
}

fn open_text(path: &Path, max_bytes: u64) -> Result<BufReader<File>, String> {
    let meta = path
        .metadata()
        .map_err(|error| format!("Cannot open {}: {error}", path.display()))?;
    if !meta.is_file() || meta.len() > max_bytes {
        return Err(format!(
            "{} is not a regular mesh file within the size limit",
            path.display()
        ));
    }
    File::open(path)
        .map(BufReader::new)
        .map_err(|error| error.to_string())
}

fn statement(line: &str) -> (&str, &str) {
    let trimmed = line.trim().trim_start_matches('\u{feff}');
    let (keyword, rest) = trimmed
        .split_once(char::is_whitespace)
        .unwrap_or((trimmed, ""));
    (keyword, rest.trim())
}

fn number(field: &str) -> Result<f32, String> {
    field
        .parse::<f32>()
        .ok()
        .filter(|value| value.is_finite())
        .ok_or_else(|| format!("Invalid number {field:?}"))
}

fn triple<'a>(fields: &mut impl Iterator<Item = &'a str>) -> Result<[f32; 3], String> {
    let mut values = [0.0; 3];
    for value in &mut values {
        *value = number(
            fields
                .next()
                .ok_or("Expected three coordinates or colour channels")?,
        )?;
    }
    Ok(values)
}

fn vertex_index(corner: &str, vertex_count: usize) -> Result<u32, String> {
    let index = corner
        .split('/')
        .next()
        .unwrap_or("")
        .parse::<i64>()
        .map_err(|_| format!("Invalid OBJ vertex index {corner:?}"))?;
    let index = match index {
        1.. => index - 1,
        ..=-1 => vertex_count as i64 + index,
        0 => return Err("OBJ vertex index zero is invalid".into()),
    };
    if index < 0 || index >= vertex_count as i64 {
        return Err("OBJ vertex index is out of range".into());
    }
    Ok(index as u32)
}

fn material_paths(obj: &Path, names: &str) -> Vec<PathBuf> {
    if let Some(path) = material_path(obj, names) {
        return vec![path];
    }
    names
        .split_whitespace()
        .filter_map(|name| material_path(obj, name))
        .collect()
}

fn material_path(obj: &Path, name: &str) -> Option<PathBuf> {
    let parent = obj.parent().unwrap_or_else(|| Path::new("."));
    let path = Path::new(name.trim_matches('"'));
    (path.is_relative()
        && path
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("mtl")))
    .then(|| parent.join(path))
    .filter(|path| path.is_file())
}

fn read_mtl(path: &Path, output: &mut HashMap<String, [u8; 4]>) -> Result<(), String> {
    let mut name = String::new();
    let mut rgb = None;
    let mut alpha = 1.0;
    let mut legacy_cym = false;
    let mut put = |name: &str, rgb: Option<[f32; 3]>, alpha, legacy_cym| {
        if let Some(rgb) = rgb.filter(|_| !name.is_empty()) {
            let rgb = rgb.map(|c| {
                unit_byte(if legacy_cym {
                    c
                } else {
                    linear_to_srgb(c.clamp(0.0, 1.0))
                })
            });
            output.insert(name.into(), [rgb[0], rgb[1], rgb[2], unit_byte(alpha)]);
        }
    };
    for line in open_text(path, MAX_MTL_BYTES)?.lines() {
        let line = line.map_err(|error| format!("Could not read material library: {error}"))?;
        match line.trim() {
            "# ColorYourModel material library" => legacy_cym = true,
            "# Kd uses linear sRGB" => legacy_cym = false,
            _ => {}
        }
        let (key, value) = statement(line.split('#').next().unwrap_or(""));
        match key {
            "newmtl" => {
                put(&name, rgb.take(), alpha, legacy_cym);
                name = value.to_owned();
                alpha = 1.0;
            }
            "Kd" => rgb = triple(&mut value.split_whitespace()).ok(),
            "d" => {
                if let Some(value) = value
                    .split_whitespace()
                    .last()
                    .and_then(|field| number(field).ok())
                {
                    alpha = value;
                }
            }
            "Tr" => {
                if let Ok(value) = number(value) {
                    alpha = 1.0 - value;
                }
            }
            _ => {}
        }
    }
    put(&name, rgb, alpha, legacy_cym);
    Ok(())
}

fn unit_byte(value: f32) -> u8 {
    (value.clamp(0.0, 1.0) * 255.0).round() as u8
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::export::obj::export_obj;
    use crate::mesh::model::MeshModel;
    use std::fs;

    #[test]
    fn concave_obj_triangulates_in_source_winding_with_negative_indices_and_material() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("模型.obj");
        fs::write(
            dir.path().join("surface colors.mtl"),
            "newmtl Coral red\nKd 0.5 0.25 0.1\n",
        )
        .unwrap();
        fs::write(
            &path,
            "mtllib surface colors.mtl\n\
            v 10 20 3\nv 14 20 3\nv 14 24 3\nv 12 21 3\nv 10 24 3\n\
            vt 0 0\nvn 0 0 1\nusemtl Coral red\n\
            f -5/1/1 -4/1/1 -3/1/1 \\\n -2/1/1 -1/1/1 # comment\n",
        )
        .unwrap();
        let (vertices, faces, colors) = read(&path).unwrap();
        assert_eq!(vertices[0], [10.0, 20.0, 3.0]);
        assert_eq!(faces.len(), 3);
        assert_eq!(colors, vec![[188, 137, 89, 255]; 3]);
        let twice_area: f32 = faces
            .iter()
            .map(|face| {
                let [a, b, c] = face.map(|i| vertices[i as usize]);
                let cross = (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0]);
                assert!(cross > 0.0);
                cross
            })
            .sum();
        assert_eq!(twice_area, 20.0);
    }

    #[test]
    fn material_and_geometry_roundtrip_with_non_primary_colors_and_small_coordinates() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("Painted #part.obj");
        let mut mesh = MeshModel::new();
        mesh.vertices = vec![[0.00000026, 0.0, 0.0], [12.0, 0.0, 0.0], [0.0, 7.0, 0.0]];
        mesh.faces = vec![[0, 1, 2]];
        mesh.face_colors = vec![[95, 139, 212, 255]];
        export_obj(&mesh, &path, &|_, _| {}).unwrap();
        let (vertices, faces, colors) = read(&path).unwrap();
        assert_eq!(vertices, mesh.vertices);
        assert_eq!(faces, mesh.faces);
        assert_eq!(colors, mesh.face_colors);
        assert!(fs::read_to_string(path.with_extension("mtl"))
            .unwrap()
            .contains("Kd 0.114435"));
    }

    #[test]
    fn legacy_cym_materials_keep_their_written_rgb_bytes() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("part.obj");
        fs::write(
            dir.path().join("old.mtl"),
            "# ColorYourModel material library\nnewmtl m0\nKd 0.5 0.25 0.1\n",
        )
        .unwrap();
        fs::write(
            &path,
            "mtllib old.mtl\nv 0 0 0\nv 1 0 0\nv 0 1 0\nusemtl m0\nf 1 2 3\n",
        )
        .unwrap();
        assert_eq!(read(&path).unwrap().2, vec![[128, 64, 26, 255]]);
    }

    #[test]
    fn obj_roundtrip_keeps_materials_with_the_same_rgb_and_different_opacity() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("Opacity.obj");
        let mut mesh = MeshModel::new();
        mesh.vertices = vec![
            [0.0, 0.0, 0.0],
            [5.0, 0.0, 0.0],
            [0.0, 5.0, 0.0],
            [5.0, 5.0, 0.0],
        ];
        mesh.faces = vec![[0, 1, 2], [1, 3, 2]];
        mesh.face_colors = vec![[60, 140, 210, 64], [60, 140, 210, 192]];
        export_obj(&mesh, &path, &|_, _| {}).unwrap();
        assert_eq!(read(&path).unwrap().2, mesh.face_colors);
        let materials = fs::read_to_string(path.with_extension("mtl")).unwrap();
        assert_eq!(materials.matches("newmtl ").count(), 2);
        assert_eq!(materials.matches("\nd ").count(), 2);
    }

    #[test]
    fn rejects_out_of_range_indices_and_point_only_obj() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("broken.obj");
        fs::write(&path, "v 0 0 0\nv 1 0 0\nv 0 1 0\nf 1 2 4\n").unwrap();
        assert!(read(&path).unwrap_err().contains("index"));
        fs::write(&path, "v 0 0 0\np 1\n").unwrap();
        assert!(read(&path).unwrap_err().contains("faces"));
    }
}
