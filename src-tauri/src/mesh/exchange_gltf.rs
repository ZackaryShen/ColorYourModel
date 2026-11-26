use crate::mesh::face_colors::{linear_to_srgb, srgb_to_linear};
use crate::mesh::model::{MeshModel, DEFAULT_FACE_COLOR};
use base64::{engine::general_purpose::STANDARD, Engine};
use gltf::accessor::{DataType, Dimensions};
use gltf::material::AlphaMode;
use gltf::mesh::Mode;
use gltf::{Accessor, Document, Node, Primitive, Semantic};
use nalgebra::{Matrix4, Vector4};
use serde_json::json;
use std::borrow::Cow;
use std::collections::{hash_map::Entry, HashMap, HashSet};
use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Component, Path};

const DATA_URI_PREFIX: &str = "data:application/octet-stream;base64,";
const SOURCE_VERTEX_ATTRIBUTE: &str = "_CYM_SOURCE_VERTEX";
// Bound expanded instances and sparse accessors before allocating. The file
// budget also accommodates an embedded export with all triangle corners split.
const MAX_TRIANGLES: usize = 2_000_000;
const MAX_VERTICES: usize = MAX_TRIANGLES * 3;
const MAX_INPUT_MIB: usize = 320;
const MAX_BYTES: usize = MAX_INPUT_MIB * 1024 * 1024;

type Geometry = (Vec<[f32; 3]>, Vec<[u32; 3]>, Vec<[u8; 4]>);

#[derive(PartialEq, Eq, Hash)]
enum VertexIdentity {
    Accessor(usize, usize),
    Original(u32),
}

struct ImportedVertex {
    index: u32,
    local_position: [f32; 3],
}

pub(crate) fn read(path: &Path) -> Result<Geometry, String> {
    let bytes = read_file(path)?;
    let (document, bin) = parse_document(&bytes)?;
    if document.animations().len() != 0 {
        return Err("glTF geometry import requires a static scene".into());
    }
    let buffers = load_buffers(&document, bin, path)?;
    let mut geometry = (Vec::new(), Vec::new(), Vec::new());
    let mut source_vertices = 0;
    visit_scene(&document, |node, transform, mirrored| {
        if node.skin().is_some()
            || node.weights().is_some()
            || node.extension_value("EXT_mesh_gpu_instancing").is_some()
        {
            return Err(
                "glTF geometry import does not support skins, morphs, or GPU instancing".into(),
            );
        }

        if let Some(mesh) = node.mesh() {
            // Accessors and original vertex IDs are local to one instantiated
            // mesh. Each separate scene node starts a fresh mapping.
            let mut vertex_map = HashMap::new();
            for primitive in mesh.primitives().filter(|item| is_triangular(item.mode())) {
                read_primitive(
                    primitive,
                    &buffers,
                    transform,
                    mirrored,
                    &mut source_vertices,
                    &mut vertex_map,
                    &mut geometry,
                )?;
            }
        }
        Ok(())
    })?;
    if geometry.1.is_empty() {
        return Err("glTF scene contains no triangles".into());
    }
    Ok(geometry)
}

pub(crate) fn has_base_color_texture(path: &Path) -> Result<bool, String> {
    let (mut file, length) = open_model(path)?;
    let mut header = [0; 20];
    file.read_exact(&mut header[..4])
        .map_err(|error| format!("Cannot read glTF header: {error}"))?;
    let bytes = if &header[..4] == b"glTF" {
        file.read_exact(&mut header[4..])
            .map_err(|error| format!("Cannot read GLB header: {error}"))?;
        let mut json = vec![0; glb_json_length(&header, length)?];
        file.read_exact(&mut json)
            .map_err(|error| format!("Cannot read GLB metadata: {error}"))?;
        json
    } else {
        read_file_bytes(file, length, &header[..4])?
    };
    let document = parse_json(&bytes)?;
    let mut textured = false;
    visit_scene(&document, |node, _, _| {
        if let Some(mesh) = node.mesh() {
            textured |= mesh.primitives().any(|primitive| {
                is_triangular(primitive.mode())
                    && primitive
                        .material()
                        .pbr_metallic_roughness()
                        .base_color_texture()
                        .is_some()
            });
        }
        Ok(())
    })?;
    Ok(textured)
}

fn read_file(path: &Path) -> Result<Vec<u8>, String> {
    let (file, length) = open_model(path)?;
    read_file_bytes(file, length, &[])
}

fn open_model(path: &Path) -> Result<(File, usize), String> {
    let metadata = fs::metadata(path).map_err(|error| format!("Cannot inspect glTF: {error}"))?;
    if !metadata.is_file() || metadata.len() > MAX_BYTES as u64 {
        return Err(format!(
            "Select a glTF file no larger than {MAX_INPUT_MIB} MiB"
        ));
    }
    let file = File::open(path).map_err(|error| format!("Cannot open glTF: {error}"))?;
    Ok((file, metadata.len() as usize))
}

fn read_file_bytes(file: File, length: usize, prefix: &[u8]) -> Result<Vec<u8>, String> {
    let mut bytes = Vec::with_capacity(length);
    bytes.extend_from_slice(prefix);
    file.take((MAX_BYTES - prefix.len() + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|error| format!("Cannot read glTF: {error}"))?;
    if bytes.len() > MAX_BYTES {
        return Err(format!("glTF file exceeds {MAX_INPUT_MIB} MiB"));
    }
    Ok(bytes)
}

fn parse_document(bytes: &[u8]) -> Result<(Document, Option<&[u8]>), String> {
    let (json, bin) = if bytes.starts_with(b"glTF") {
        split_glb(bytes)?
    } else {
        (bytes, None)
    };
    Ok((parse_json(json)?, bin))
}

fn parse_json(json: &[u8]) -> Result<Document, String> {
    let mut root = gltf::json::Root::from_slice(json)
        .map_err(|error| format!("Invalid glTF metadata: {error}"))?;
    if root.asset.version != "2.0"
        || root
            .asset
            .min_version
            .as_deref()
            .is_some_and(|min| min != "2.0")
    {
        return Err("Geometry import requires glTF 2.0".into());
    }
    // Unlit changes the shading model while preserving base color and geometry.
    root.extensions_required
        .retain(|name| name != "KHR_materials_unlit");
    if let Some(extension) = root.extensions_required.first() {
        return Err(format!("Unsupported required glTF extension: {extension}"));
    }
    // gltf 1.4.1 looks up POSITION unchecked during metadata validation.
    if root
        .meshes
        .iter()
        .flat_map(|mesh| &mesh.primitives)
        .any(|primitive| {
            use gltf::json::validation::Checked::Valid;
            primitive
                .attributes
                .get(&Valid(Semantic::Positions))
                .is_some_and(|index| index.value() >= root.accessors.len())
        })
    {
        return Err("glTF POSITION accessor index is out of range".into());
    }
    Document::from_json(root).map_err(|error| format!("Invalid glTF: {error}"))
}

fn u32_at(bytes: &[u8], offset: usize) -> Option<u32> {
    bytes
        .get(offset..offset + 4)?
        .try_into()
        .ok()
        .map(u32::from_le_bytes)
}

fn glb_json_length(header: &[u8], file_length: usize) -> Result<usize, String> {
    if u32_at(header, 4) != Some(2)
        || u32_at(header, 8).map(|length| length as usize) != Some(file_length)
    {
        return Err("Invalid GLB version or file length".into());
    }
    if header.get(16..20) != Some(b"JSON") {
        return Err("Missing initial GLB JSON chunk".into());
    }
    u32_at(header, 12)
        .map(|length| length as usize)
        .filter(|&length| length % 4 == 0 && length <= file_length.saturating_sub(20))
        .ok_or_else(|| "Invalid GLB JSON chunk length or alignment".into())
}

fn split_glb(bytes: &[u8]) -> Result<(&[u8], Option<&[u8]>), String> {
    let json_end = 20 + glb_json_length(bytes, bytes.len())?;
    let json = &bytes[20..json_end];
    let mut cursor = json_end;
    let mut bin = None;
    while cursor < bytes.len() {
        let length = u32_at(bytes, cursor).ok_or("Truncated GLB chunk header")? as usize;
        let kind = bytes
            .get(cursor + 4..cursor + 8)
            .ok_or("Truncated GLB chunk header")?;
        let start = cursor + 8;
        cursor = start
            .checked_add(length)
            .ok_or("Invalid GLB chunk length")?;
        let chunk = bytes
            .get(start..cursor)
            .filter(|_| length % 4 == 0)
            .ok_or("Invalid GLB chunk length or alignment")?;
        match kind {
            b"BIN\0" if start == json_end + 8 => bin = Some(chunk),
            b"JSON" | b"BIN\0" => return Err("Invalid GLB chunk order".into()),
            _ => {} // Forward-compatible extension chunks follow JSON and BIN.
        }
    }
    Ok((json, bin))
}

fn load_buffers<'a>(
    document: &Document,
    bin: Option<&'a [u8]>,
    path: &Path,
) -> Result<Vec<Cow<'a, [u8]>>, String> {
    let mut total_bytes = 0;
    document
        .buffers()
        .map(|buffer| {
            let length = buffer.length();
            total_bytes = bounded_sum(
                total_bytes,
                length,
                MAX_BYTES,
                "glTF buffer data exceeds the file size budget",
            )?;
            match buffer.source() {
                gltf::buffer::Source::Bin if buffer.index() == 0 => {
                    let embedded = bin
                        .filter(|bin| (length..=length + 3).contains(&bin.len()))
                        .ok_or("GLB BIN chunk does not match declared buffer length")?;
                    Ok(Cow::Borrowed(&embedded[..length]))
                }
                gltf::buffer::Source::Bin => {
                    Err("Only the first glTF buffer may use the GLB BIN chunk".into())
                }
                gltf::buffer::Source::Uri(uri) if uri.starts_with("data:") => {
                    let (media_type, data) =
                        uri.split_once(',').ok_or("Invalid glTF buffer data URI")?;
                    if !matches!(
                        media_type,
                        "data:application/octet-stream;base64"
                            | "data:application/gltf-buffer;base64"
                    ) {
                        return Err("Unsupported glTF buffer data URI type".into());
                    }
                    let mut bytes = STANDARD
                        .decode(data)
                        .map_err(|_| "Invalid base64 glTF buffer")?;
                    if bytes.len() < length {
                        return Err(
                            "Embedded glTF buffer is shorter than its declared length".into()
                        );
                    }
                    bytes.truncate(length);
                    Ok(Cow::Owned(bytes))
                }
                gltf::buffer::Source::Uri(uri) => {
                    load_local_buffer(path, uri, length).map(Cow::Owned)
                }
            }
        })
        .collect()
}

fn load_local_buffer(path: &Path, uri: &str, length: usize) -> Result<Vec<u8>, String> {
    fn hex(byte: u8) -> Option<u8> {
        (byte as char).to_digit(16).map(|digit| digit as u8)
    }
    if uri.contains(['?', '#']) {
        return Err("glTF buffers must reference local files".into());
    }
    let mut decoded = Vec::with_capacity(uri.len());
    let mut bytes = uri.bytes();
    while let Some(byte) = bytes.next() {
        if byte == b'%' {
            let hi = bytes
                .next()
                .and_then(hex)
                .ok_or("Invalid glTF buffer URI encoding")?;
            let lo = bytes
                .next()
                .and_then(hex)
                .ok_or("Invalid glTF buffer URI encoding")?;
            decoded.push(hi * 16 + lo);
        } else {
            decoded.push(byte);
        }
    }
    let relative = std::str::from_utf8(&decoded).map_err(|_| "Invalid UTF-8 glTF buffer path")?;
    let buffer_path = Path::new(relative);
    if relative.is_empty()
        || relative.contains([':', '\\', '\0'])
        || buffer_path
            .components()
            .any(|part| !matches!(part, Component::Normal(_) | Component::CurDir))
    {
        return Err("glTF buffers must reference files inside the model folder".into());
    }
    let folder = path
        .parent()
        .filter(|dir| !dir.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."))
        .canonicalize()
        .map_err(|error| format!("Cannot resolve glTF folder: {error}"))?;
    let buffer_path = folder
        .join(buffer_path)
        .canonicalize()
        .map_err(|error| format!("Cannot resolve glTF buffer: {error}"))?;
    if !buffer_path.starts_with(&folder) {
        return Err("glTF buffer resolves outside the model folder".into());
    }
    let metadata = fs::metadata(&buffer_path)
        .map_err(|error| format!("Cannot inspect glTF buffer: {error}"))?;
    if !metadata.is_file() || metadata.len() < length as u64 {
        return Err("External glTF buffer is not a file of the declared length".into());
    }
    let mut data = vec![0; length];
    File::open(buffer_path)
        .and_then(|mut input| input.read_exact(&mut data))
        .map_err(|error| format!("Cannot read glTF buffer: {error}"))?;
    Ok(data)
}

fn visit_scene(
    document: &Document,
    mut visit: impl FnMut(Node<'_>, &Matrix4<f64>, bool) -> Result<(), String>,
) -> Result<(), String> {
    let roots: Vec<_> = if let Some(scene) = document
        .default_scene()
        .or_else(|| document.scenes().next())
    {
        scene.nodes().collect()
    } else {
        let children: HashSet<_> = document
            .nodes()
            .flat_map(|node| {
                node.children()
                    .map(|child| child.index())
                    .collect::<Vec<_>>()
            })
            .collect();
        document
            .nodes()
            .filter(|node| !children.contains(&node.index()))
            .collect()
    };
    let mut stack: Vec<_> = roots
        .into_iter()
        .rev()
        .map(|node| (node, Matrix4::<f64>::identity()))
        .collect();
    let mut visited = HashSet::new();
    while let Some((node, parent)) = stack.pop() {
        if !visited.insert(node.index()) {
            return Err("glTF scene has a cycle or a node with multiple parents".into());
        }
        let local = node.transform().matrix();
        if [local[0][3], local[1][3], local[2][3], local[3][3]] != [0.0, 0.0, 0.0, 1.0] {
            return Err("glTF node transform must be affine".into());
        }
        let transform = parent * Matrix4::from_iterator(local.into_iter().flatten().map(f64::from));
        let determinant = transform.fixed_view::<3, 3>(0, 0).determinant();
        if !transform.iter().all(|value| value.is_finite()) || !determinant.is_finite() {
            return Err("Non-finite glTF node transform".into());
        }
        let child_start = stack.len();
        stack.extend(node.children().map(|child| (child, transform)));
        stack[child_start..].reverse();
        visit(node, &transform, determinant < 0.0)?;
    }
    Ok(())
}

fn is_triangular(mode: Mode) -> bool {
    matches!(
        mode,
        Mode::Triangles | Mode::TriangleStrip | Mode::TriangleFan
    )
}

fn bounded_sum(
    current: usize,
    increase: usize,
    limit: usize,
    error: &str,
) -> Result<usize, String> {
    current
        .checked_add(increase)
        .filter(|&total| total <= limit)
        .ok_or_else(|| error.into())
}

fn view_bytes<'a>(
    view: &gltf::buffer::View<'_>,
    buffers: &'a [Cow<'_, [u8]>],
) -> Result<&'a [u8], String> {
    if view.extension_value("EXT_meshopt_compression").is_some() {
        return Err("Unsupported glTF meshopt compression".into());
    }
    let end = view
        .offset()
        .checked_add(view.length())
        .ok_or("Invalid glTF buffer view range")?;
    buffers
        .get(view.buffer().index())
        .and_then(|buffer| buffer.get(view.offset()..end))
        .ok_or_else(|| "glTF buffer view range is out of bounds".into())
}

fn accessor_range(
    bytes: &[u8],
    offset: usize,
    count: usize,
    stride: usize,
    item_size: usize,
) -> Result<&[u8], String> {
    count
        .checked_sub(1)
        .and_then(|last| last.checked_mul(stride))
        .and_then(|span| span.checked_add(item_size))
        .and_then(|length| offset.checked_add(length))
        .filter(|_| stride >= item_size)
        .and_then(|end| bytes.get(offset..end))
        .ok_or_else(|| "Invalid glTF accessor byte range or stride".into())
}

fn validate_accessor(
    accessor: &Accessor<'_>,
    buffers: &[Cow<'_, [u8]>],
    indices: bool,
) -> Result<(), String> {
    let item_size = accessor.size();
    let alignment = accessor.data_type().size();
    let count = accessor.count();
    if count == 0 || count > MAX_VERTICES {
        return Err("Invalid glTF accessor element count".into());
    }
    if let Some(view) = accessor.view() {
        let stride = view.stride().unwrap_or(item_size);
        if (indices && view.stride().is_some())
            || accessor.offset() % alignment != 0
            || view.offset() % alignment != 0
            || stride % alignment != 0
        {
            return Err("Invalid glTF accessor alignment".into());
        }
        accessor_range(
            view_bytes(&view, buffers)?,
            accessor.offset(),
            count,
            stride,
            item_size,
        )?;
    } else if accessor.sparse().is_none() {
        return Err("glTF geometry accessor has no buffer data".into());
    }
    if let Some(sparse) = accessor.sparse() {
        let positions = sparse.indices();
        let values = sparse.values();
        let position_view = positions.view();
        let value_view = values.view();
        let position_size = positions.index_type().size();
        let sparse_count = sparse.count();
        if sparse_count > count
            || position_view.stride().is_some()
            || value_view.stride().is_some()
            || positions.offset() % position_size != 0
            || position_view.offset() % position_size != 0
            || values.offset() % alignment != 0
            || value_view.offset() % alignment != 0
        {
            return Err("Invalid glTF sparse accessor count or alignment".into());
        }
        accessor_range(
            view_bytes(&value_view, buffers)?,
            values.offset(),
            sparse_count,
            item_size,
            item_size,
        )?;
        let position_bytes = accessor_range(
            view_bytes(&position_view, buffers)?,
            positions.offset(),
            sparse_count,
            position_size,
            position_size,
        )?;
        let mut next_min = 0;
        for position in position_bytes.chunks_exact(position_size) {
            let index = position
                .iter()
                .rev()
                .fold(0usize, |value, &byte| value * 256 + byte as usize);
            if !(next_min..count).contains(&index) {
                return Err("glTF sparse indices must be increasing and in range".into());
            }
            next_min = index + 1;
        }
    }
    Ok(())
}

fn read_primitive(
    primitive: Primitive<'_>,
    buffers: &[Cow<'_, [u8]>],
    transform: &Matrix4<f64>,
    mirrored: bool,
    source_vertices: &mut usize,
    vertex_map: &mut HashMap<VertexIdentity, ImportedVertex>,
    (vertices, faces, face_colors): &mut Geometry,
) -> Result<(), String> {
    if primitive.morph_targets().len() != 0
        || primitive
            .extension_value("KHR_draco_mesh_compression")
            .is_some()
    {
        return Err(
            "glTF geometry import does not support morph targets or Draco compression".into(),
        );
    }
    let position = primitive
        .get(&Semantic::Positions)
        .ok_or("Missing glTF POSITION")?;
    if position.dimensions() != Dimensions::Vec3
        || position.data_type() != DataType::F32
        || position.normalized()
    {
        return Err("glTF POSITION must contain floating-point VEC3 values".into());
    }
    let position_count = position.count();
    let position_accessor = position.index();
    *source_vertices = bounded_sum(
        *source_vertices,
        position_count,
        MAX_VERTICES,
        "glTF scene exceeds 6 million source vertices",
    )?;
    validate_accessor(&position, buffers, false)?;
    let color_accessor = primitive.get(&Semantic::Colors(0));
    if let Some(color) = &color_accessor {
        let valid_type = match color.data_type() {
            DataType::F32 => !color.normalized(),
            DataType::U8 | DataType::U16 => color.normalized(),
            _ => false,
        };
        if !valid_type
            || !matches!(color.dimensions(), Dimensions::Vec3 | Dimensions::Vec4)
            || color.count() != position_count
        {
            return Err("glTF COLOR_0 must have one valid RGB or RGBA value per position".into());
        }
        validate_accessor(color, buffers, false)?;
    }
    let mut index_count = position_count;
    let mut restart = u32::MAX;
    if let Some(indices) = primitive.indices() {
        restart = match indices.data_type() {
            DataType::U8 => u8::MAX.into(),
            DataType::U16 => u16::MAX.into(),
            DataType::U32 => u32::MAX,
            _ => return Err("glTF indices must be unsigned integers".into()),
        };
        if indices.dimensions() != Dimensions::Scalar || indices.normalized() {
            return Err("glTF indices must be non-normalized scalars".into());
        }
        index_count = indices.count();
        validate_accessor(&indices, buffers, true)?;
    }
    let mode = primitive.mode();
    let triangle_count = match mode {
        Mode::Triangles if index_count % 3 == 0 && index_count >= 3 => index_count / 3,
        Mode::TriangleStrip | Mode::TriangleFan if index_count >= 3 => index_count - 2,
        _ => return Err("Invalid glTF triangle index count".into()),
    };
    bounded_sum(
        faces.len(),
        triangle_count,
        MAX_TRIANGLES,
        "glTF scene exceeds 2 million triangles",
    )?;

    let buffer_data =
        |buffer: gltf::Buffer<'_>| buffers.get(buffer.index()).map(|bytes| bytes.as_ref());
    let source_ids = primitive
        .get(&Semantic::Extras(SOURCE_VERTEX_ATTRIBUTE[1..].into()))
        .map(|accessor| {
            if accessor.dimensions() != Dimensions::Vec2
                || accessor.data_type() != DataType::U16
                || accessor.normalized()
                || accessor.count() != position_count
            {
                return Err("Invalid glTF source vertex IDs".into());
            }
            validate_accessor(&accessor, buffers, false)?;
            let values: Vec<_> = gltf::accessor::Iter::<[u16; 2]>::new(accessor, buffer_data)
                .ok_or("Cannot read glTF source vertex IDs")?
                .map(|[low, high]| u32::from(low) | (u32::from(high) << 16))
                .collect();
            if values.len() != position_count {
                return Err("glTF source vertex IDs do not match the positions".into());
            }
            Ok::<_, String>(values)
        })
        .transpose()?;
    let reader = primitive.reader(buffer_data);
    let positions: Vec<_> = reader
        .read_positions()
        .ok_or("Cannot read glTF POSITION")?
        .collect();
    let colors = if color_accessor.is_some() {
        let colors: Vec<_> = reader
            .read_colors(0)
            .ok_or("Cannot read glTF COLOR_0")?
            .into_rgba_f32()
            .collect();
        if colors.iter().flatten().any(|channel| !channel.is_finite()) {
            return Err("Non-finite glTF vertex color".into());
        }
        Some(colors)
    } else {
        None
    };
    let indices: Vec<_> = if primitive.indices().is_some() {
        reader
            .read_indices()
            .ok_or("Cannot read glTF triangle indices")?
            .into_u32()
            .collect()
    } else {
        (0..position_count as u32).collect()
    };
    if positions.len() != position_count
        || colors
            .as_ref()
            .is_some_and(|values| values.len() != position_count)
        || indices.len() != index_count
        || indices
            .iter()
            .any(|&index| index as usize >= position_count || index == restart)
    {
        return Err("glTF geometry accessor count or triangle index is out of range".into());
    }

    let material = primitive.material();
    let factor = material.pbr_metallic_roughness().base_color_factor();
    if factor.iter().any(|channel| !(0.0..=1.0).contains(channel)) {
        return Err("Invalid glTF material base color factor".into());
    }
    let alpha_mode = material.alpha_mode();
    let alpha_cutoff = material.alpha_cutoff().unwrap_or(0.5);
    if alpha_mode == AlphaMode::Mask && (!alpha_cutoff.is_finite() || alpha_cutoff < 0.0) {
        return Err("Invalid glTF material alpha cutoff".into());
    }
    let rgba = |color| linear_rgba(color, alpha_mode, alpha_cutoff);
    let solid_color = if material.index().is_some() {
        rgba(factor)
    } else {
        DEFAULT_FACE_COLOR
    };
    let mut remapped = vec![None; position_count];
    for n in 0..triangle_count {
        let winding = match mode {
            Mode::Triangles => [3 * n, 3 * n + 1, 3 * n + 2],
            Mode::TriangleStrip if n % 2 == 1 => [n, n + 2, n + 1],
            Mode::TriangleStrip => [n, n + 1, n + 2],
            Mode::TriangleFan => [0, n + 1, n + 2],
            _ => return Err("Unsupported glTF primitive topology".into()),
        };
        let corners = winding.map(|index| indices[index] as usize);
        let mut face = [0; 3];
        for (dest, source) in face.iter_mut().zip(corners) {
            *dest = if let Some(id) = remapped[source] {
                id
            } else {
                let local = positions[source];
                let key = match &source_ids {
                    Some(ids) => VertexIdentity::Original(ids[source]),
                    None => VertexIdentity::Accessor(position_accessor, source),
                };
                let id = match vertex_map.entry(key) {
                    Entry::Occupied(known) => {
                        let known = known.get();
                        if known.local_position != local {
                            return Err("Conflicting positions for a glTF source vertex ID".into());
                        }
                        known.index
                    }
                    Entry::Vacant(new) => {
                        let [x, y, z] = local.map(f64::from);
                        let point = transform * Vector4::new(x, y, z, 1.0);
                        // glTF has Y up in metres. CYM has Z up in millimetres.
                        let position =
                            [point.x, -point.z, point.y].map(|value| (1000.0 * value) as f32);
                        if !position.iter().all(|channel| channel.is_finite()) {
                            return Err("Non-finite glTF vertex position".into());
                        }
                        let id = vertices.len() as u32;
                        vertices.push(position);
                        new.insert(ImportedVertex {
                            index: id,
                            local_position: local,
                        });
                        id
                    }
                };
                remapped[source] = Some(id);
                id
            };
        }
        if mirrored {
            face.swap(1, 2);
        }
        faces.push(face);
        face_colors.push(colors.as_ref().map_or(solid_color, |colors| {
            rgba(std::array::from_fn(|channel| {
                let sum: f32 = corners
                    .iter()
                    .map(|&corner| colors[corner][channel].clamp(0.0, 1.0))
                    .sum();
                sum / 3.0 * factor[channel]
            }))
        }));
    }
    Ok(())
}

fn linear_rgba([r, g, b, a]: [f32; 4], alpha_mode: AlphaMode, alpha_cutoff: f32) -> [u8; 4] {
    let srgb = |value| (linear_to_srgb(value) * 255.0).round() as u8;
    let alpha = match alpha_mode {
        AlphaMode::Opaque => 255,
        AlphaMode::Blend => (a * 255.0).round() as u8,
        // CYM has one alpha per face. For partial coverage, the centroid's
        // interpolated alpha determines whether the entire face is masked.
        AlphaMode::Mask => {
            if a >= alpha_cutoff {
                255
            } else {
                0
            }
        }
    };
    [srgb(r), srgb(g), srgb(b), alpha]
}

pub(crate) fn write(mesh: &MeshModel, binary: bool, output: &mut dyn Write) -> Result<(), String> {
    if mesh.faces.is_empty() || mesh.faces.len() > MAX_TRIANGLES {
        return Err("glTF export requires between 1 and 2 million triangles".into());
    }
    let linear: [f32; 256] = std::array::from_fn(|value| srgb_to_linear(value as f32 / 255.0));
    let mut vertex_map = HashMap::new();
    let mut bin = Vec::new();
    let mut indices = Vec::with_capacity(mesh.faces.len() * 3);
    let mut min = [f32::INFINITY; 3];
    let mut max = [f32::NEG_INFINITY; 3];
    let mut transparent = false;
    for (face_id, face) in mesh.faces.iter().enumerate() {
        let color = mesh
            .face_colors
            .get(face_id)
            .copied()
            .unwrap_or(DEFAULT_FACE_COLOR);
        transparent |= color[3] != 255;
        for &vertex in face {
            let vertex_count = vertex_map.len() as u32;
            let index = match vertex_map.entry((vertex, color)) {
                Entry::Occupied(existing) => *existing.get(),
                Entry::Vacant(new) => {
                    let &[x, y, z] = mesh
                        .vertices
                        .get(vertex as usize)
                        .ok_or("glTF export face index is out of range")?;
                    let position = [x / 1000.0, z / 1000.0, -y / 1000.0];
                    if !position.iter().all(|value| value.is_finite()) {
                        return Err("glTF export has a non-finite vertex".into());
                    }
                    for (axis, value) in position.into_iter().enumerate() {
                        min[axis] = min[axis].min(value);
                        max[axis] = max[axis].max(value);
                        bin.extend_from_slice(&value.to_le_bytes());
                    }
                    for value in &color[..3] {
                        bin.extend_from_slice(&linear[*value as usize].to_le_bytes());
                    }
                    bin.extend_from_slice(&(color[3] as f32 / 255.0).to_le_bytes());
                    // VEC2 of two shorts carries a u32 ID without using an
                    // UNSIGNED_INT component, which glTF reserves for indices.
                    bin.extend_from_slice(&vertex.to_le_bytes());
                    *new.insert(vertex_count)
                }
            };
            indices.push(index);
        }
    }
    let vertex_count = vertex_map.len();
    drop(vertex_map);
    let attribute_bytes = bin.len();
    let index_count = indices.len();
    for index in indices {
        bin.extend_from_slice(&index.to_le_bytes());
    }

    // All vertices of each face carry its color in standard linear FLOAT
    // COLOR_0. Reuse corners only when position and face color both match.
    let mut doc = json!({
        "asset":{"version":"2.0","generator":"ColorYourModel"},
        "scene":0,"scenes":[{"nodes":[0]}],"nodes":[{"mesh":0}],
        "meshes":[{"primitives":[{"attributes":{"POSITION":0,"COLOR_0":1,(SOURCE_VERTEX_ATTRIBUTE):3},"indices":2,"material":0}]}],
        "materials":[{"pbrMetallicRoughness":{"metallicFactor":0,"roughnessFactor":1},
                      "alphaMode": if transparent { "BLEND" } else { "OPAQUE" }}],
        "accessors":[
            {"bufferView":0,"componentType":5126,"count":vertex_count,"type":"VEC3","min":min,"max":max},
            {"bufferView":0,"byteOffset":12,"componentType":5126,"count":vertex_count,"type":"VEC4"},
            {"bufferView":1,"componentType":5125,"count":index_count,"type":"SCALAR"},
            {"bufferView":0,"byteOffset":28,"componentType":5123,"count":vertex_count,"type":"VEC2"}
        ],
        "bufferViews":[
            {"buffer":0,"byteLength":attribute_bytes,"byteStride":32,"target":34962},
            {"buffer":0,"byteOffset":attribute_bytes,"byteLength":index_count * 4,"target":34963}
        ],
        "buffers":[{"byteLength":bin.len()}]
    });
    if binary {
        use gltf::binary::{Glb, Header};
        let json =
            serde_json::to_vec(&doc).map_err(|error| format!("Cannot encode glTF: {error}"))?;
        Glb {
            header: Header {
                magic: *b"glTF",
                version: 2,
                length: 0,
            },
            json: Cow::Owned(json),
            bin: Some(Cow::Owned(bin)),
        }
        .to_writer(output)
        .map_err(|error| format!("Cannot write GLB: {error}"))
    } else {
        let mut uri = DATA_URI_PREFIX.to_string();
        STANDARD.encode_string(bin, &mut uri);
        doc["buffers"][0]["uri"] = uri.into();
        serde_json::to_writer(output, &doc).map_err(|error| format!("Cannot write glTF: {error}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mesh::model::DEFAULT_FACE_COLOR;
    use base64::{engine::general_purpose::STANDARD, Engine};
    use serde_json::{json, Value};
    use std::fs;

    fn fixture() -> (Value, Vec<u8>) {
        let vertices: [[f32; 3]; 6] = [
            [0.0, 0.0, 0.0],
            [1.0, 0.0, 0.0],
            [1.0, 1.0, 0.0],
            [-0.0, 0.0, -0.0],
            [1.0, 1.0, 0.0],
            [0.0, 1.0, 0.0],
        ];
        let mut bin: Vec<_> = vertices
            .into_iter()
            .flatten()
            .flat_map(f32::to_le_bytes)
            .collect();
        bin.extend_from_slice(&[
            255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255, 0, 0, 255, 128, 0, 0, 255, 128, 0, 0,
            255, 128,
        ]);
        let doc = json!({
            "asset": {"version": "2.0"},
            "scene": 0, "scenes": [{"nodes":[0,2]}],
            "nodes": [{"translation":[2.0,0.0,0.5],"children":[1]},
                      {"mesh":0,"scale":[-1.0,1.0,1.0]}, {"mesh":0}],
            "meshes":[{"primitives":[{
                "attributes":{"POSITION":0,"COLOR_0":1},"material":0
            }]}],
            "materials":[{"pbrMetallicRoughness":{"baseColorFactor":[0.5,1.0,0.25,0.5]},
                          "alphaMode":"BLEND"}],
            "accessors":[
                {"bufferView":0,"componentType":5126,"count":6,"type":"VEC3",
                 "min":[0,0,0],"max":[1,1,0]},
                {"bufferView":1,"componentType":5121,"count":6,"type":"VEC4","normalized":true}
            ],
            "bufferViews":[{"buffer":0,"byteLength":72},
                           {"buffer":0,"byteOffset":72,"byteLength":24}],
            "buffers":[{"byteLength":bin.len()}]
        });
        (doc, bin)
    }

    fn save_json(path: &Path, doc: &Value) {
        fs::write(path, serde_json::to_vec(doc).unwrap()).unwrap();
    }

    #[test]
    fn external_buffer_scene_instances_keep_mirrored_winding_and_coincident_indices() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("scene.gltf");
        let (mut doc, bin) = fixture();
        doc["buffers"][0]["uri"] = json!("mesh%20data.bin");
        fs::write(temp.path().join("mesh data.bin"), bin).unwrap();
        save_json(&path, &doc);

        let (vertices, faces, colors) = read(&path).unwrap();
        assert_eq!(
            vertices,
            [
                [2000.0, -500.0, 0.0],
                [1000.0, -500.0, 0.0],
                [1000.0, -500.0, 1000.0],
                [2000.0, -500.0, 0.0],
                [1000.0, -500.0, 1000.0],
                [2000.0, -500.0, 1000.0],
                [0.0, 0.0, 0.0],
                [1000.0, 0.0, 0.0],
                [1000.0, 0.0, 1000.0],
                [0.0, 0.0, 0.0],
                [1000.0, 0.0, 1000.0],
                [0.0, 0.0, 1000.0],
            ]
        );
        assert_eq!(faces, [[0, 2, 1], [3, 5, 4], [6, 7, 8], [9, 10, 11]]);
        assert_eq!(
            colors,
            [
                [113, 156, 82, 128],
                [0, 0, 137, 64],
                [113, 156, 82, 128],
                [0, 0, 137, 64]
            ]
        );
    }

    #[test]
    fn embedded_buffer_without_colors_uses_mesh_default() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("unpainted.gltf");
        let (mut doc, bin) = fixture();
        doc["meshes"][0]["primitives"][0] = json!({"attributes":{"POSITION":0}});
        doc["buffers"][0]["uri"] = json!(format!(
            "data:application/octet-stream;base64,{}",
            STANDARD.encode(bin)
        ));
        save_json(&path, &doc);
        let (_, _, colors) = read(&path).unwrap();
        assert_eq!(colors, [DEFAULT_FACE_COLOR; 4]);
    }

    #[test]
    fn import_respects_material_alpha_modes_at_the_triangle_average() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("alpha.gltf");
        let (mut doc, mut bin) = fixture();
        // First face straddles a 0.4 cutoff. At its centroid, alpha is about 0.417.
        bin[72 + 7] = 128;
        doc["buffers"][0]["uri"] = json!(format!("{DATA_URI_PREFIX}{}", STANDARD.encode(bin)));
        for (mode, cutoff, alpha) in [
            (None, None, [255, 255]),
            (Some("OPAQUE"), Some(0.9), [255, 255]),
            (Some("BLEND"), None, [106, 64]),
            (Some("MASK"), Some(0.4), [255, 0]),
            (Some("MASK"), None, [0, 0]),
            (Some("MASK"), Some(1.2), [0, 0]),
        ] {
            let material = doc["materials"][0].as_object_mut().unwrap();
            material.remove("alphaMode");
            material.remove("alphaCutoff");
            if let Some(mode) = mode {
                material.insert("alphaMode".into(), json!(mode));
            }
            if let Some(cutoff) = cutoff {
                material.insert("alphaCutoff".into(), json!(cutoff));
            }
            save_json(&path, &doc);
            let (_, _, colors) = read(&path).unwrap();
            assert_eq!(
                colors.iter().map(|rgba| rgba[3]).collect::<Vec<_>>(),
                [alpha[0], alpha[1], alpha[0], alpha[1]],
                "{mode:?} {cutoff:?}"
            );
        }

        // With no COLOR_0, the base factor exactly equals the default cutoff.
        doc["meshes"][0]["primitives"][0]["attributes"] = json!({"POSITION":0});
        doc["materials"][0]
            .as_object_mut()
            .unwrap()
            .remove("alphaCutoff");
        save_json(&path, &doc);
        let (_, _, colors) = read(&path).unwrap();
        assert!(colors.iter().all(|rgba| rgba[3] == 255));
    }

    #[test]
    fn static_exports_preserve_flat_face_colors_and_standard_coordinates() {
        let mut mesh = MeshModel::new();
        mesh.vertices = vec![
            [0.0, 0.0, 0.0],
            [250.0, 0.0, 0.0],
            [250.0, 125.0, 500.0],
            [0.0, 125.0, 500.0],
        ];
        mesh.faces = vec![[0, 1, 2], [0, 2, 3]];
        mesh.face_colors = vec![[1, 128, 241, 255], [2, 7, 24, 65]];
        let temp = tempfile::tempdir().unwrap();
        for (binary, name) in [(false, "mesh.gltf"), (true, "mesh.glb")] {
            let mut bytes = Vec::new();
            write(&mesh, binary, &mut bytes).unwrap();
            let path = temp.path().join(name);
            fs::write(&path, &bytes).unwrap();
            let gltf = gltf::Gltf::from_slice(&bytes).unwrap();
            let primitive = gltf.meshes().next().unwrap().primitives().next().unwrap();
            assert_eq!(
                primitive.get(&gltf::Semantic::Positions).unwrap().max(),
                Some(json!([0.25, 0.5, 0.0]))
            );
            assert_eq!(
                primitive.get(&gltf::Semantic::Positions).unwrap().min(),
                Some(json!([0.0, 0.0, -0.125]))
            );
            assert_eq!(
                read(&path).unwrap(),
                (
                    mesh.vertices.clone(),
                    mesh.faces.clone(),
                    mesh.face_colors.clone()
                )
            );
        }
    }

    #[test]
    fn native_roundtrip_preserves_distinct_indices_at_the_same_position() {
        let mut mesh = MeshModel::new();
        let triangle = [[0.0, 0.0, 0.0], [250.0, 0.0, 0.0], [0.0, 0.0, 250.0]];
        mesh.vertices.extend(triangle);
        mesh.vertices.extend(triangle);
        mesh.faces = vec![[0, 1, 2], [3, 4, 5]];
        mesh.face_colors = vec![[255, 0, 0, 255], [0, 255, 0, 255]];
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("coincident.glb");
        let mut bytes = Vec::new();
        write(&mesh, true, &mut bytes).unwrap();
        fs::write(&path, bytes).unwrap();
        assert_eq!(
            read(&path).unwrap(),
            (mesh.vertices, mesh.faces, mesh.face_colors)
        );
    }

    #[test]
    fn source_vertex_id_cannot_merge_different_positions() {
        let mut mesh = MeshModel::new();
        mesh.vertices = vec![[0., 0., 0.], [250., 0., 0.], [0., 0., 250.]];
        mesh.faces = vec![[0, 1, 2]];
        let mut bytes = Vec::new();
        write(&mesh, false, &mut bytes).unwrap();
        let mut doc: Value = serde_json::from_slice(&bytes).unwrap();
        let attribute = doc["meshes"][0]["primitives"][0]["attributes"][SOURCE_VERTEX_ATTRIBUTE]
            .as_u64()
            .unwrap() as usize;
        let accessor = &doc["accessors"][attribute];
        let offset = accessor["byteOffset"].as_u64().unwrap() as usize;
        let view = accessor["bufferView"].as_u64().unwrap() as usize;
        let stride = doc["bufferViews"][view]["byteStride"].as_u64().unwrap() as usize;
        let uri = &mut doc["buffers"][0]["uri"];
        let mut bin = STANDARD
            .decode(uri.as_str().unwrap().strip_prefix(DATA_URI_PREFIX).unwrap())
            .unwrap();
        bin.copy_within(offset..offset + 4, offset + stride);
        *uri = json!(format!("{DATA_URI_PREFIX}{}", STANDARD.encode(bin)));

        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("bad-provenance.gltf");
        save_json(&path, &doc);
        assert!(read(&path).unwrap_err().contains("Conflicting positions"));
    }

    #[test]
    fn malformed_accessor_index_and_binary_range_return_errors() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("invalid.gltf");
        let (mut doc, bin) = fixture();
        doc["buffers"][0]["uri"] = json!(format!(
            "data:application/octet-stream;base64,{}",
            STANDARD.encode(bin)
        ));

        doc["meshes"][0]["primitives"][0]["attributes"]["POSITION"] = json!(7);
        save_json(&path, &doc);
        assert!(read(&path).unwrap_err().contains("POSITION"));

        doc["meshes"][0]["primitives"][0]["attributes"]["POSITION"] = json!(0);
        doc["accessors"][0]["byteOffset"] = json!(4);
        save_json(&path, &doc);
        assert!(read(&path).unwrap_err().contains("range"));
    }

    #[test]
    fn indexed_strips_read_sparse_positions_and_reject_repeated_sparse_indices() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("strip.gltf");
        let mut bin = vec![1, 2, 3, 0];
        let sparse_positions = [[1.0_f32, 0.0, 0.0], [0.0, 1.0, 0.0], [1.0, 1.0, 0.0]];
        bin.extend(
            sparse_positions
                .into_iter()
                .flatten()
                .flat_map(f32::to_le_bytes),
        );
        bin.extend_from_slice(&[0, 1, 2, 3]);
        let mut doc = json!({
            "asset":{"version":"2.0"},
            "scenes":[{"nodes":[0]}],"nodes":[{"mesh":0}],
            "meshes":[{"primitives":[{"attributes":{"POSITION":0},"indices":1,"mode":5,"material":0}]}],
            "materials":[{"pbrMetallicRoughness":{"baseColorFactor":[1.0,0.0,0.25,1.0]}}],
            "accessors":[
                {"componentType":5126,"count":4,"type":"VEC3","min":[0,0,0],"max":[1,1,0],
                 "sparse":{"count":3,"indices":{"bufferView":0,"componentType":5121},"values":{"bufferView":1}}},
                {"bufferView":2,"componentType":5121,"count":4,"type":"SCALAR"}
            ],
            "bufferViews":[{"buffer":0,"byteLength":3},
                           {"buffer":0,"byteOffset":4,"byteLength":36},
                           {"buffer":0,"byteOffset":40,"byteLength":4}],
            "buffers":[{"byteLength":bin.len()}]
        });
        doc["buffers"][0]["uri"] = json!(format!("{DATA_URI_PREFIX}{}", STANDARD.encode(&bin)));
        save_json(&path, &doc);
        let (vertices, faces, colors) = read(&path).unwrap();
        assert_eq!(
            vertices,
            [
                [0., 0., 0.],
                [1000., 0., 0.],
                [0., 0., 1000.],
                [1000., 0., 1000.]
            ]
        );
        assert_eq!(faces, [[0, 1, 2], [1, 3, 2]]);
        assert_eq!(colors, [[255, 0, 137, 255]; 2]);

        bin[0] = 2;
        doc["buffers"][0]["uri"] = json!(format!("{DATA_URI_PREFIX}{}", STANDARD.encode(&bin)));
        save_json(&path, &doc);
        assert!(read(&path).unwrap_err().contains("sparse indices"));
    }

    #[test]
    fn glb_allows_unknown_chunks_after_bin_and_rejects_invalid_chunk_bounds() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("extensions.glb");
        let (doc, bin) = fixture();
        use gltf::binary::{Glb, Header};
        let mut bytes = Glb {
            header: Header {
                magic: *b"glTF",
                version: 2,
                length: 0,
            },
            json: Cow::Owned(serde_json::to_vec(&doc).unwrap()),
            bin: Some(Cow::Owned(bin)),
        }
        .to_vec()
        .unwrap();
        let extra_chunk = bytes.len();
        bytes.extend_from_slice(b"\x04\0\0\0FOUR\0\0\0\0");
        let length = bytes.len() as u32;
        bytes[8..12].copy_from_slice(&length.to_le_bytes());
        fs::write(&path, &bytes).unwrap();
        assert!(!has_base_color_texture(&path).unwrap());
        assert_eq!(read(&path).unwrap().1.len(), 4);

        bytes[extra_chunk] = 8;
        fs::write(&path, &bytes).unwrap();
        assert!(!has_base_color_texture(&path).unwrap());
        assert!(read(&path).unwrap_err().contains("chunk length"));
    }

    #[test]
    fn texture_routing_uses_active_scene_materials() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("routing.gltf");
        let (mut doc, _) = fixture();
        doc["images"] = json!([{"uri":"missing.png"}]);
        doc["textures"] = json!([{"source":0}]);
        doc["materials"]
            .as_array_mut()
            .unwrap()
            .push(json!({"pbrMetallicRoughness":{"baseColorTexture":{"index":0}}}));
        doc["meshes"]
            .as_array_mut()
            .unwrap()
            .push(json!({"primitives":[{"attributes":{"POSITION":0},"material":1}]}));
        doc["nodes"].as_array_mut().unwrap().push(json!({"mesh":1}));
        doc["scenes"]
            .as_array_mut()
            .unwrap()
            .push(json!({"nodes":[3]}));
        save_json(&path, &doc);
        assert!(!has_base_color_texture(&path).unwrap());

        doc["scene"] = json!(1);
        save_json(&path, &doc);
        assert!(has_base_color_texture(&path).unwrap());
    }
}
