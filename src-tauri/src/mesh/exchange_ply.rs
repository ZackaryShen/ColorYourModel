use super::exchange::triangulate_polygon;
use super::model::{MeshModel, DEFAULT_FACE_COLOR};
use std::collections::HashMap;
use std::fs::File;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::Path;
use std::str::SplitAsciiWhitespace;

type PlyMesh = (Vec<[f32; 3]>, Vec<[u32; 3]>, Vec<[u8; 4]>);

// Keep limits high enough for detailed print meshes while bounding malicious
// counts, ASCII records, and the quadratic triangulation of a single polygon.
const MAX_FILE_BYTES: u64 = 512 * 1024 * 1024;
const MAX_HEADER_BYTES: usize = 256 * 1024;
const MAX_RECORD_BYTES: usize = 1024 * 1024;
const MAX_VERTICES: usize = 10_000_000;
const MAX_TRIANGLES: usize = 5_000_000;
const MAX_POLYGON_CORNERS: usize = 4_096;
const MAX_LIST_ITEMS: usize = 1_000_000;
const SOURCE_INDEX_COMMENT: &str = "cym_source_vertex identifies original vertices at color seams";

#[derive(Clone, Copy)]
enum Encoding {
    Ascii,
    LittleEndian,
    BigEndian,
}

#[derive(Clone, Copy)]
enum Scalar {
    I8,
    U8,
    I16,
    U16,
    I32,
    U32,
    F32,
    F64,
}

impl Scalar {
    fn parse(name: &str) -> Result<Self, String> {
        match name {
            "char" | "int8" => Ok(Self::I8),
            "uchar" | "uint8" => Ok(Self::U8),
            "short" | "int16" => Ok(Self::I16),
            "ushort" | "uint16" => Ok(Self::U16),
            "int" | "int32" => Ok(Self::I32),
            "uint" | "uint32" => Ok(Self::U32),
            "float" | "float32" => Ok(Self::F32),
            "double" | "float64" => Ok(Self::F64),
            _ => Err(format!("Unsupported PLY property type {name}")),
        }
    }

    fn is_integer(self) -> bool {
        !matches!(self, Self::F32 | Self::F64)
    }

    fn ascii(self, token: &str) -> Result<f64, String> {
        let value = match self {
            Self::I8 => token.parse::<i8>().ok().map(f64::from),
            Self::U8 => token.parse::<u8>().ok().map(f64::from),
            Self::I16 => token.parse::<i16>().ok().map(f64::from),
            Self::U16 => token.parse::<u16>().ok().map(f64::from),
            Self::I32 => token.parse::<i32>().ok().map(f64::from),
            Self::U32 => token.parse::<u32>().ok().map(f64::from),
            Self::F32 => token.parse::<f32>().ok().map(f64::from),
            Self::F64 => token.parse::<f64>().ok(),
        };
        value.ok_or_else(|| "Invalid PLY property value or list count".into())
    }

    fn binary(self, input: &mut dyn Read, little: bool) -> Result<f64, String> {
        fn bytes<const N: usize>(input: &mut dyn Read, little: bool) -> Result<[u8; N], String> {
            let mut bytes = [0; N];
            input.read_exact(&mut bytes).map_err(ply_io_error)?;
            if !little {
                bytes.reverse();
            }
            Ok(bytes)
        }

        Ok(match self {
            Self::I8 => f64::from(i8::from_le_bytes(bytes(input, little)?)),
            Self::U8 => f64::from(u8::from_le_bytes(bytes(input, little)?)),
            Self::I16 => f64::from(i16::from_le_bytes(bytes(input, little)?)),
            Self::U16 => f64::from(u16::from_le_bytes(bytes(input, little)?)),
            Self::I32 => f64::from(i32::from_le_bytes(bytes(input, little)?)),
            Self::U32 => f64::from(u32::from_le_bytes(bytes(input, little)?)),
            Self::F32 => f64::from(f32::from_le_bytes(bytes(input, little)?)),
            Self::F64 => f64::from_le_bytes(bytes(input, little)?),
        })
    }

    fn color(self, value: f64) -> Result<u8, String> {
        let byte = if self.is_integer() {
            value
        } else {
            value * 255.0
        };
        if !(0.0..=255.0).contains(&byte) {
            return Err("PLY color is outside its supported range".into());
        }
        Ok(byte.round() as u8)
    }
}

#[derive(Clone, Copy)]
enum Property {
    Scalar(Scalar),
    List { count: Scalar, item: Scalar },
}

#[derive(Clone, Copy)]
enum Field {
    Position(usize),
    Color(usize),
    SourceIndex,
    FaceIndices,
    Other,
}

#[derive(PartialEq, Eq, Clone, Copy)]
enum ElementKind {
    Vertex,
    Face,
    Other,
}

struct Element {
    kind: ElementKind,
    name: String,
    count: usize,
    properties: Vec<(String, Property, Field)>,
    has_rgb: bool,
}

struct Header {
    encoding: Encoding,
    elements: Vec<Element>,
    has_source_indices: bool,
}

fn ply_io_error(error: std::io::Error) -> String {
    format!("PLY file I/O failed: {error}")
}

fn bounded_line(
    input: &mut impl BufRead,
    buffer: &mut Vec<u8>,
    limit: usize,
) -> Result<(), String> {
    buffer.clear();
    let count = input
        .take(limit as u64 + 1)
        .read_until(b'\n', buffer)
        .map_err(ply_io_error)?;
    if count == 0 {
        return Err("Unexpected end of PLY file".into());
    }
    if count > limit {
        return Err("PLY line exceeds the size limit".into());
    }
    Ok(())
}

fn read_header(input: &mut impl BufRead) -> Result<Header, String> {
    let mut line = Vec::new();
    let mut header_bytes = 0;
    let mut elements: Vec<Element> = Vec::new();
    let mut has_source_indices = false;
    let mut format = None;
    loop {
        bounded_line(input, &mut line, MAX_HEADER_BYTES - header_bytes)?;
        let text = std::str::from_utf8(&line).map_err(|_| "PLY header must be text")?;
        if header_bytes == 0 {
            if text.trim_end_matches(['\r', '\n']) != "ply" {
                return Err("Invalid PLY signature".into());
            }
            header_bytes += line.len();
            continue;
        }
        header_bytes += line.len();
        let fields: Vec<_> = text.split_ascii_whitespace().collect();
        match fields.as_slice() {
            ["comment", ..] => {
                if text.trim().strip_prefix("comment ") == Some(SOURCE_INDEX_COMMENT) {
                    has_source_indices = true;
                }
            }
            ["obj_info", ..] => {}
            ["format", encoding, "1.0"] if format.is_none() => {
                format = Some(match *encoding {
                    "ascii" => Encoding::Ascii,
                    "binary_little_endian" => Encoding::LittleEndian,
                    "binary_big_endian" => Encoding::BigEndian,
                    _ => return Err(format!("Unsupported PLY encoding {encoding}")),
                });
            }
            ["element", name, count] if format.is_some() => {
                let kind = match *name {
                    "vertex" => ElementKind::Vertex,
                    "face" => ElementKind::Face,
                    _ => ElementKind::Other,
                };
                let limit = if kind == ElementKind::Face {
                    MAX_TRIANGLES
                } else {
                    MAX_VERTICES
                };
                let count = count
                    .parse::<usize>()
                    .map_err(|_| "Invalid PLY element count")?;
                if count > limit {
                    return Err(format!("PLY {name} count exceeds the {limit} limit"));
                }
                if elements.iter().any(|element| element.name == *name) {
                    return Err(format!("Duplicate PLY element {name}"));
                }
                elements.push(Element {
                    kind,
                    name: (*name).into(),
                    count,
                    properties: Vec::new(),
                    has_rgb: false,
                });
            }
            ["property", ..] => {
                let element = elements
                    .last_mut()
                    .ok_or("PLY property precedes its element")?;
                let (name, property) = match fields.as_slice() {
                    ["property", ty, name] => (*name, Property::Scalar(Scalar::parse(ty)?)),
                    ["property", "list", count, item, name] => {
                        let count = Scalar::parse(count)?;
                        if !count.is_integer() {
                            return Err("PLY list count must use an integer property type".into());
                        }
                        (
                            *name,
                            Property::List {
                                count,
                                item: Scalar::parse(item)?,
                            },
                        )
                    }
                    _ => return Err("Invalid PLY property declaration".into()),
                };
                if element
                    .properties
                    .iter()
                    .any(|(property, _, _)| property == name)
                {
                    return Err(format!("Duplicate PLY property {name}"));
                }
                element
                    .properties
                    .push((name.into(), property, Field::Other));
            }
            ["end_header"] => break,
            _ => return Err("Invalid or unsupported PLY header declaration".into()),
        }
    }

    let encoding = format.ok_or("PLY is missing a format declaration")?;
    let mut mesh_elements = (false, false);
    for element in &mut elements {
        element.set_fields(has_source_indices)?;
        match element.kind {
            ElementKind::Vertex => mesh_elements.0 = element.count > 0,
            ElementKind::Face => mesh_elements.1 = element.count > 0,
            ElementKind::Other => {}
        }
    }
    if mesh_elements != (true, true) {
        return Err(
            "PLY must contain vertices and polygon faces; point clouds are unsupported".into(),
        );
    }
    Ok(Header {
        encoding,
        elements,
        has_source_indices,
    })
}

impl Element {
    fn set_fields(&mut self, has_source_indices: bool) -> Result<(), String> {
        if self.properties.is_empty() && self.count > 0 {
            return Err(format!("PLY {} element has no properties", self.name));
        }
        let mut xyz_mask = 0u8;
        let mut color_mask = 0u8;
        let mut has_faces = false;
        let mut has_index = false;
        for (name, property, field) in &mut self.properties {
            *field = match (self.kind, name.as_str(), property) {
                (ElementKind::Vertex, "x" | "y" | "z", Property::Scalar(_)) => {
                    let axis = usize::from(name.as_bytes()[0] - b'x');
                    xyz_mask |= 1 << axis;
                    Field::Position(axis)
                }
                (ElementKind::Vertex, "cym_source_vertex", Property::Scalar(ty))
                    if has_source_indices && ty.is_integer() =>
                {
                    has_index = true;
                    Field::SourceIndex
                }
                (
                    ElementKind::Face,
                    "vertex_index" | "vertex_indices",
                    Property::List { item, .. },
                ) if item.is_integer() => {
                    if has_faces {
                        return Err("PLY face has two vertex index lists".into());
                    }
                    has_faces = true;
                    Field::FaceIndices
                }
                (ElementKind::Vertex | ElementKind::Face, name, Property::Scalar(_)) => {
                    let channel = match name {
                        "red" => 0,
                        "green" => 1,
                        "blue" => 2,
                        "alpha" => 3,
                        _ => continue,
                    };
                    color_mask |= 1 << channel;
                    Field::Color(channel)
                }
                _ => Field::Other,
            }
        }
        self.has_rgb = color_mask & 0b111 == 0b111;
        match self.kind {
            ElementKind::Vertex if xyz_mask != 0b111 => {
                return Err("PLY vertex requires scalar x, y and z properties".into())
            }
            ElementKind::Vertex if has_source_indices && !has_index => {
                return Err(
                    "PLY CYM color seam metadata is missing the original vertex index".into(),
                )
            }
            ElementKind::Face if !has_faces => {
                return Err("PLY face requires an integer vertex index list".into())
            }
            _ => {}
        }
        Ok(())
    }
}

enum Values<'a> {
    Ascii(SplitAsciiWhitespace<'a>),
    Binary {
        input: &'a mut dyn Read,
        little: bool,
    },
}

impl Values<'_> {
    fn read(&mut self, scalar: Scalar) -> Result<f64, String> {
        match self {
            Self::Ascii(tokens) => scalar.ascii(tokens.next().ok_or("Missing PLY property value")?),
            Self::Binary { input, little } => scalar.binary(*input, *little),
        }
    }
}

struct Record {
    position: [f32; 3],
    color: [u8; 4],
    index: u32,
    polygon: Vec<u32>,
}

impl Default for Record {
    fn default() -> Self {
        Self {
            position: [0.0; 3],
            color: [0, 0, 0, 255],
            index: 0,
            polygon: Vec::new(),
        }
    }
}

impl Record {
    fn read(&mut self, element: &Element, mut values: Values<'_>) -> Result<(), String> {
        self.polygon.clear();
        for (_, property, field) in &element.properties {
            match *property {
                Property::Scalar(ty) => {
                    let value = values.read(ty)?;
                    match *field {
                        Field::Position(axis) => {
                            let value = value as f32;
                            if !value.is_finite() {
                                return Err("PLY vertex coordinate is not a finite float".into());
                            }
                            self.position[axis] = value;
                        }
                        Field::Color(channel) if element.has_rgb => {
                            self.color[channel] = ty.color(value)?
                        }
                        Field::SourceIndex => self.index = vertex_index(value)?,
                        _ => {}
                    }
                }
                Property::List { count, item } => {
                    let face_indices = matches!(field, Field::FaceIndices);
                    let limit = if face_indices {
                        MAX_POLYGON_CORNERS
                    } else {
                        MAX_LIST_ITEMS
                    };
                    let len = values.read(count)?;
                    if !(0.0..=limit as f64).contains(&len) {
                        return Err(format!(
                            "PLY list count is outside the supported limit of {limit}"
                        ));
                    }
                    let len = len as usize;
                    if face_indices {
                        if len < 3 {
                            return Err("PLY face has fewer than three vertex indices".into());
                        }
                        self.polygon.reserve(len);
                        for _ in 0..len {
                            self.polygon.push(vertex_index(values.read(item)?)?);
                        }
                    } else {
                        for _ in 0..len {
                            values.read(item)?;
                        }
                    }
                }
            }
        }
        if let Values::Ascii(mut tokens) = values {
            if tokens.next().is_some() {
                return Err("Unexpected extra PLY property values".into());
            }
        }
        Ok(())
    }
}

fn vertex_index(value: f64) -> Result<u32, String> {
    if !(0.0..=f64::from(u32::MAX)).contains(&value) {
        return Err("Invalid PLY vertex index".into());
    }
    Ok(value as u32)
}

struct MeshBuilder {
    vertices: Vec<[f32; 3]>,
    vertex_colors: Vec<[u8; 4]>,
    source_indices: Vec<u32>,
    faces: Vec<[u32; 3]>,
    face_colors: Vec<[u8; 4]>,
}

impl MeshBuilder {
    fn add_face(&mut self, polygon: &[u32], color: Option<[u8; 4]>) -> Result<(), String> {
        if polygon
            .iter()
            .any(|&index| index as usize >= self.vertices.len())
        {
            return Err("PLY face index is outside the vertex table".into());
        }
        if polygon.len() == 3 {
            self.add_triangle([polygon[0], polygon[1], polygon[2]], color)?;
        } else {
            for triangle in triangulate_polygon(polygon, &self.vertices)? {
                self.add_triangle(triangle, color)?;
            }
        }
        Ok(())
    }

    fn add_triangle(
        &mut self,
        triangle: [u32; 3],
        face_color: Option<[u8; 4]>,
    ) -> Result<(), String> {
        if self.faces.len() >= MAX_TRIANGLES {
            return Err("PLY triangle count exceeds the supported limit".into());
        }
        let color = face_color.unwrap_or_else(|| {
            if self.vertex_colors.is_empty() {
                return DEFAULT_FACE_COLOR;
            }
            std::array::from_fn(|channel| {
                let sum: u16 = triangle
                    .iter()
                    .map(|&index| u16::from(self.vertex_colors[index as usize][channel]))
                    .sum();
                // Round to the nearest byte.
                ((sum + 1) / 3) as u8
            })
        });
        self.faces.push(triangle);
        self.face_colors.push(color);
        Ok(())
    }

    fn finish(mut self) -> Result<PlyMesh, String> {
        if !self.source_indices.is_empty() {
            // Rejoin the color splits written by CYM. Unmarked PLY inputs
            // retain their original topology, including coincident vertices.
            let mut original_to_merged = HashMap::new();
            let mut vertices = Vec::new();
            let mut remap = Vec::with_capacity(self.vertices.len());
            for (position, source_index) in self.vertices.into_iter().zip(self.source_indices) {
                let index = *original_to_merged.entry(source_index).or_insert_with(|| {
                    let next = vertices.len();
                    vertices.push(position);
                    next
                });
                if vertices[index] != position {
                    return Err("PLY color seam metadata joins different vertex positions".into());
                }
                remap.push(index as u32);
            }
            for index in self.faces.iter_mut().flatten() {
                *index = remap[*index as usize];
            }
            self.vertices = vertices;
        }
        Ok((self.vertices, self.faces, self.face_colors))
    }
}

pub(crate) fn read(path: &Path) -> Result<PlyMesh, String> {
    let file = File::open(path).map_err(ply_io_error)?;
    let metadata = file.metadata().map_err(ply_io_error)?;
    if !metadata.is_file() || metadata.len() > MAX_FILE_BYTES {
        return Err("PLY source must be a file no larger than 512 MiB".into());
    }
    let mut input = BufReader::new(file.take(MAX_FILE_BYTES));
    let header = read_header(&mut input)?;
    let mut mesh = MeshBuilder {
        vertices: Vec::new(),
        vertex_colors: Vec::new(),
        source_indices: Vec::new(),
        faces: Vec::new(),
        face_colors: Vec::new(),
    };
    let mut line = Vec::new();
    let mut pending = Vec::new();
    for element in header.elements {
        let mut record = Record::default();
        for _ in 0..element.count {
            let values = match header.encoding {
                Encoding::Ascii => {
                    bounded_line(&mut input, &mut line, MAX_RECORD_BYTES)?;
                    let text =
                        std::str::from_utf8(&line).map_err(|_| "PLY ASCII data is not text")?;
                    Values::Ascii(text.split_ascii_whitespace())
                }
                Encoding::LittleEndian | Encoding::BigEndian => Values::Binary {
                    input: &mut input,
                    little: matches!(header.encoding, Encoding::LittleEndian),
                },
            };
            record.read(&element, values)?;
            match element.kind {
                ElementKind::Vertex => {
                    mesh.vertices.push(record.position);
                    if element.has_rgb {
                        mesh.vertex_colors.push(record.color);
                    }
                    if header.has_source_indices {
                        mesh.source_indices.push(record.index);
                    }
                }
                ElementKind::Face => {
                    let color = element.has_rgb.then_some(record.color);
                    if mesh.vertices.is_empty() {
                        pending.push((std::mem::take(&mut record.polygon), color));
                    } else {
                        mesh.add_face(&record.polygon, color)?;
                    }
                }
                ElementKind::Other => {}
            }
        }
    }
    for (polygon, color) in pending {
        mesh.add_face(&polygon, color)?;
    }
    mesh.finish()
}

struct ColoredVertex {
    source: u32,
    color: Option<[u8; 4]>,
}

pub(crate) fn write(mesh: &MeshModel, output: &mut dyn Write) -> Result<(), String> {
    if mesh.vertices.is_empty() || mesh.faces.is_empty() {
        return Err("PLY needs a polygon mesh".into());
    }
    if mesh.vertices.len() > MAX_VERTICES || mesh.faces.len() > MAX_TRIANGLES {
        return Err("PLY mesh exceeds the supported size limit".into());
    }
    if mesh.vertices.iter().flatten().any(|axis| !axis.is_finite()) {
        return Err("PLY vertex coordinate is not a finite float".into());
    }
    let source_vertex_count = mesh.vertices.len();
    let mut vertices: Vec<_> = (0..mesh.vertices.len() as u32)
        .map(|source| ColoredVertex {
            source,
            color: None,
        })
        .collect();
    let mut faces = Vec::with_capacity(mesh.faces.len());
    let mut split_vertices = HashMap::new();
    for (face_id, &face) in mesh.faces.iter().enumerate() {
        let color = mesh
            .face_colors
            .get(face_id)
            .copied()
            .unwrap_or(DEFAULT_FACE_COLOR);
        let mut split = [0; 3];
        for (slot, source) in face.into_iter().enumerate() {
            let vertex = vertices[..source_vertex_count]
                .get_mut(source as usize)
                .ok_or("PLY face index is outside the vertex table")?;
            let primary_color = *vertex.color.get_or_insert(color);
            split[slot] = if primary_color == color {
                source
            } else if let Some(&index) = split_vertices.get(&(source, color)) {
                index
            } else {
                let index = vertices.len();
                if index == MAX_VERTICES {
                    return Err("PLY color seams exceed the supported vertex limit".into());
                }
                vertices.push(ColoredVertex {
                    source,
                    color: Some(color),
                });
                split_vertices.insert((source, color), index as u32);
                index as u32
            };
        }
        faces.push(split);
    }
    write_binary(output, mesh, &vertices, &faces).map_err(ply_io_error)
}

fn write_binary(
    output: &mut dyn Write,
    mesh: &MeshModel,
    vertices: &[ColoredVertex],
    faces: &[[u32; 3]],
) -> std::io::Result<()> {
    writeln!(
        output,
        "ply\nformat binary_little_endian 1.0\ncomment {SOURCE_INDEX_COMMENT}"
    )?;
    writeln!(output, "element vertex {}", vertices.len())?;
    writeln!(
        output,
        "property float x\nproperty float y\nproperty float z"
    )?;
    writeln!(
        output,
        "property uchar red\nproperty uchar green\nproperty uchar blue\nproperty uchar alpha"
    )?;
    writeln!(output, "property uint cym_source_vertex")?;
    writeln!(output, "element face {}", faces.len())?;
    writeln!(output, "property list uchar int vertex_indices\nend_header")?;

    for vertex in vertices {
        let mut bytes = [0u8; 20];
        for (slot, axis) in bytes[..12]
            .chunks_exact_mut(4)
            .zip(mesh.vertices[vertex.source as usize])
        {
            slot.copy_from_slice(&axis.to_le_bytes());
        }
        bytes[12..16].copy_from_slice(&vertex.color.unwrap_or(DEFAULT_FACE_COLOR));
        bytes[16..].copy_from_slice(&vertex.source.to_le_bytes());
        output.write_all(&bytes)?;
    }
    for face in faces {
        let mut bytes = [3u8; 13];
        for (slot, index) in bytes[1..].chunks_exact_mut(4).zip(face) {
            slot.copy_from_slice(&index.to_le_bytes());
        }
        output.write_all(&bytes)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::super::model::DEFAULT_FACE_COLOR;
    use super::*;

    fn read_bytes(bytes: &[u8]) -> Result<(Vec<[f32; 3]>, Vec<[u32; 3]>, Vec<[u8; 4]>), String> {
        let mut file = tempfile::NamedTempFile::new().unwrap();
        file.write_all(bytes).unwrap();
        read(file.path())
    }

    const ASCII_HEADER: &str = "ply\nformat ascii 1.0\nelement vertex 3\nproperty float x\nproperty float y\nproperty float z\nelement face 1\nproperty list uchar int vertex_indices\nend_header\n";

    #[test]
    fn reads_ascii_reordered_attributes_and_prefers_face_rgba() {
        let bytes = b"ply\r\nformat ascii 1.0\r\ncomment Blender-style PLY\r\nelement face 1\r\nproperty uchar blue\r\nproperty list uchar int vertex_index\r\nproperty uchar red\r\nproperty uchar alpha\r\nproperty uchar green\r\nelement vendor 1\r\nproperty list uchar short ignored\r\nelement vertex 3\r\nproperty uchar blue\r\nproperty double z\r\nproperty uchar green\r\nproperty float y\r\nproperty uchar red\r\nproperty float x\r\nend_header\r\n18 3 0 1 2 92 128 151\r\n2 -4 3\r\n4 8 3 -2 200 1\r\n4 8 3 -2 200 7\r\n4 8 3 5 200 1\r\n";
        let (vertices, faces, colors) = read_bytes(bytes).unwrap();
        assert_eq!(
            vertices,
            [[1.0, -2.0, 8.0], [7.0, -2.0, 8.0], [1.0, 5.0, 8.0]]
        );
        assert_eq!(faces, [[0, 1, 2]]);
        assert_eq!(colors, [[92, 151, 18, 128]]);
    }

    #[test]
    fn reads_both_binary_byte_orders_and_averages_vertex_colors() {
        for little in [true, false] {
            let format = if little {
                "binary_little_endian"
            } else {
                "binary_big_endian"
            };
            let mut bytes = format!("ply\nformat {format} 1.0\nelement vertex 3\nproperty float y\nproperty uchar red\nproperty float z\nproperty float x\nproperty uchar green\nproperty uchar blue\nproperty uchar alpha\nelement face 1\nproperty list ushort uint vertex_indices\nend_header\n").into_bytes();
            for (x, y, z, [r, g, b, a]) in [
                (1.5_f32, 2.5_f32, 3.5_f32, [255, 0, 0, 0]),
                (3.0, 2.5, 3.5, [0, 0, 0, 255]),
                (1.5, 4.0, 3.5, [0, 0, 255, 255]),
            ] {
                let encode = |v: f32| {
                    if little {
                        v.to_le_bytes()
                    } else {
                        v.to_be_bytes()
                    }
                };
                bytes.extend(encode(y));
                bytes.push(r);
                bytes.extend(encode(z));
                bytes.extend(encode(x));
                bytes.extend([g, b, a]);
            }
            bytes.extend(if little {
                3_u16.to_le_bytes()
            } else {
                3_u16.to_be_bytes()
            });
            for index in 0..3_u32 {
                bytes.extend(if little {
                    index.to_le_bytes()
                } else {
                    index.to_be_bytes()
                });
            }
            let (vertices, faces, colors) = read_bytes(&bytes).unwrap();
            assert_eq!(
                vertices,
                [[1.5, 2.5, 3.5], [3.0, 2.5, 3.5], [1.5, 4.0, 3.5]]
            );
            assert_eq!(faces, [[0, 1, 2]]);
            assert_eq!(colors, [[85, 0, 85, 170]]);
        }
    }

    #[test]
    fn triangulates_concave_faces_and_uses_default_grey() {
        let bytes = b"ply\nformat ascii 1.0\nelement vertex 5\nproperty float x\nproperty float y\nproperty float z\nelement face 1\nproperty list uchar uint vertex_indices\nend_header\n0 0 0\n4 0 0\n4 4 0\n2 1 0\n0 4 0\n5 0 1 2 3 4\n";
        let (vertices, faces, colors) = read_bytes(bytes).unwrap();
        assert_eq!(faces.len(), 3);
        assert_eq!(colors, [DEFAULT_FACE_COLOR; 3]);
        let double_areas: Vec<_> = faces
            .iter()
            .copied()
            .map(|[a, b, c]| {
                let [a, b, c] = [a, b, c].map(|index| vertices[index as usize]);
                (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0])
            })
            .collect();
        assert!(double_areas.iter().all(|area| *area > 0.0));
        assert_eq!(double_areas.iter().sum::<f32>(), 20.0);
    }

    #[test]
    fn rejects_bad_counts_indices_and_nonfinite_coordinates() {
        for (face_line, expected) in [
            ("3 0 1 9", "outside"),
            ("3 0 1 -1", "index"),
            ("4 0 1 2", "property"),
            ("2 0 1", "fewer"),
            ("-1", "list count"),
        ] {
            let source = format!("{ASCII_HEADER}0 0 0\n1 0 0\n0 1 0\n{face_line}\n");
            let error = read_bytes(source.as_bytes()).unwrap_err();
            assert!(error.contains(expected), "{error}");
        }
        let oversized = ASCII_HEADER.replace("vertex 3", "vertex 999999999");
        assert!(read_bytes(oversized.as_bytes())
            .unwrap_err()
            .contains("limit"));
        let input = format!("{ASCII_HEADER}NaN 0 0\n1 0 0\n0 1 0\n3 0 1 2\n");
        assert!(read_bytes(input.as_bytes()).unwrap_err().contains("finite"));
    }

    #[test]
    fn reimports_own_vertex_color_seams_without_merging_distinct_coincident_vertices() {
        let mut mesh = MeshModel::new();
        mesh.vertices = vec![
            [0., 0., 0.],
            [2., 0., 0.],
            [0., 2., 0.],
            [2., 2., 0.],
            [0., 0., 0.],
        ];
        mesh.faces = vec![[0, 1, 2], [1, 3, 2], [4, 2, 3]];
        mesh.face_colors = vec![[255, 20, 40, 200], [30, 10, 255, 255], [10, 128, 1, 255]];
        let mut bytes = Vec::new();
        write(&mesh, &mut bytes).unwrap();
        assert!(bytes.starts_with(b"ply\nformat binary_little_endian 1.0\n"));
        let (vertices, faces, colors) = read_bytes(&bytes).unwrap();
        assert_eq!(vertices, mesh.vertices);
        assert_eq!(faces, mesh.faces);
        assert_eq!(colors, mesh.face_colors);
    }

    #[test]
    fn writer_rejects_an_invalid_index_even_after_creating_color_seams() {
        let mut mesh = MeshModel::new();
        mesh.vertices = vec![[0., 0., 0.], [2., 0., 0.], [0., 2., 0.], [2., 2., 0.]];
        mesh.faces = vec![[0, 1, 2], [1, 3, 2], [4, 2, 3]];
        mesh.face_colors = vec![[255, 20, 40, 255], [30, 10, 255, 255], [30, 10, 255, 255]];
        let mut bytes = Vec::new();
        assert!(write(&mesh, &mut bytes).is_err());
        assert!(bytes.is_empty());
    }
}
