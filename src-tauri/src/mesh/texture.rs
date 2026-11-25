use crate::export::{paint_color::MAX_EXTRUDER_SLOT, quantize::quantize_face_colors};
use base64::{engine::general_purpose::STANDARD, Engine};
use gltf::accessor::{DataType, Dimensions};
use gltf::texture::{MagFilter, MinFilter, WrappingMode};
use gltf::{Accessor, Document, Primitive, Semantic};
use image::{DynamicImage, ImageDecoder, RgbImage};
use nalgebra::{Matrix4, Vector3, Vector4};
use serde::Serialize;
use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::io::{Cursor, Read};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};

// Both meshes cross Tauri's JSON boundary and coexist with the current mesh.
// Keep triangle indices and nine floats of per-corner preview data per face bounded.
pub(crate) const MAX_FACES: usize = 200_000;
const MAX_FILE_BYTES: usize = 48 * 1024 * 1024;
const MAX_JSON_BYTES: usize = 4 * 1024 * 1024;
const MAX_IMAGE_BYTES: usize = 8 * 1024 * 1024;
const MAX_IMAGE_EDGE: u32 = 4096;
const MAX_IMAGE_PIXELS: u64 = 8 * 1024 * 1024;
const MAX_SUBDIVISION_LEVELS: i32 = 6;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OriginalTexture {
    pub vertices: Vec<f32>,
    pub faces: Vec<u32>,
    pub corner_uvs: Vec<f32>,
    pub image_url: String,
    pub wrap_s: String,
    pub wrap_t: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mag_filter: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub min_filter: Option<u32>,
    pub base_color_factor: [f32; 4],
}

#[derive(Debug)]
pub struct BakedTexture {
    pub original: OriginalTexture,
    pub vertices: Vec<[f32; 3]>,
    pub faces: Vec<[u32; 3]>,
    pub face_colors: Vec<[u8; 4]>,
    pub palette: Vec<[u8; 3]>,
    pub warning: Option<&'static str>,
}

pub fn load_texture(
    path: &Path,
    color_limit: u8,
    cancelled: &AtomicBool,
) -> Result<BakedTexture, String> {
    check_cancelled(cancelled)?;
    if !(1..=MAX_EXTRUDER_SLOT).contains(&color_limit) {
        return Err(format!(
            "Texture palette must contain 1–{MAX_EXTRUDER_SLOT} colors"
        ));
    }

    let mut file = File::open(path).map_err(|error| format!("Cannot open GLB: {error}"))?;
    let metadata = file
        .metadata()
        .map_err(|error| format!("Cannot inspect GLB: {error}"))?;
    if !metadata.is_file() {
        return Err("Select a GLB file".into());
    }
    if metadata.len() > MAX_FILE_BYTES as u64 {
        return Err("GLB exceeds the 48 MiB file budget".into());
    }
    let mut bytes = Vec::with_capacity(metadata.len() as usize);
    (&mut file)
        .take(MAX_FILE_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| format!("Cannot read GLB: {error}"))?;
    if bytes.len() > MAX_FILE_BYTES {
        return Err("GLB exceeds the 48 MiB file budget".into());
    }
    check_cancelled(cancelled)?;

    let (document, bin) = parse_glb(&bytes)?;
    let (sources, texture) = validate_scene(&document, bin, cancelled)?;
    let mut vertices = Vec::new();
    let mut triangles = read_geometry(
        &sources,
        bin,
        texture.info.tex_coord(),
        &mut vertices,
        cancelled,
    )?;
    let image = decode_texture(texture.image, texture.mime_type)?;
    check_cancelled(cancelled)?;
    let (wrap_s, wrap_t, factor) = (texture.wrap_s, texture.wrap_t, texture.factor);
    let (mag_filter, min_filter) = (texture.mag_filter, texture.min_filter);
    let mut image_url = format!("data:{};base64,", texture.mime_type);
    STANDARD.encode_string(texture.image, &mut image_url);

    let original = OriginalTexture {
        vertices: vertices.iter().flatten().copied().collect(),
        faces: triangles
            .iter()
            .flat_map(|triangle| triangle.indices)
            .collect(),
        corner_uvs: triangles
            .iter()
            .flat_map(|triangle| triangle.uvs.into_iter().flatten())
            .collect(),
        image_url,
        wrap_s: wrap_name(wrap_s).into(),
        wrap_t: wrap_name(wrap_t).into(),
        mag_filter: mag_filter.map(|filter| filter.as_gl_enum()),
        min_filter: min_filter.map(|filter| filter.as_gl_enum()),
        base_color_factor: factor,
    };
    drop(sources);
    drop(document);
    drop(bytes);

    let texel_extent = longest_texture_edge(&triangles, image.dimensions());
    let mut depth = 0;
    let mut warning = None;
    // Split at least once, then aim for edges no longer than one texel.
    while depth == 0 || texel_extent > 2f64.powi(depth) {
        check_cancelled(cancelled)?;
        if triangles.len() * 4 > MAX_FACES {
            warning = Some("budget");
            break;
        }
        if depth == MAX_SUBDIVISION_LEVELS {
            warning = Some("level-limit");
            break;
        }
        let vertex_count = vertices.len();
        let Some(finer) = subdivide(&triangles, &mut vertices, cancelled)? else {
            // Keep the whole previous level so every shared edge still matches.
            vertices.truncate(vertex_count);
            warning = Some("precision");
            break;
        };
        triangles = finer;
        depth += 1;
    }

    // Baking averages samples from the base image, independent of viewport zoom.
    // If only the minification sampler was defined, use its base-level kernel.
    let nearest = match mag_filter {
        Some(filter) => filter == MagFilter::Nearest,
        None => matches!(
            min_filter,
            Some(
                MinFilter::Nearest
                    | MinFilter::NearestMipmapNearest
                    | MinFilter::NearestMipmapLinear
            )
        ),
    };
    let sampler = TextureSampler {
        image: &image,
        wrap_s,
        wrap_t,
        nearest,
        factor,
        srgb_to_linear: std::array::from_fn(|value| srgb_to_linear(value as f32 / 255.0)),
    };
    let mut face_colors = Vec::with_capacity(triangles.len());
    let mut faces = Vec::with_capacity(triangles.len());
    for triangle in triangles {
        check_cancelled(cancelled)?;
        face_colors.push(sampler.face_color(triangle.uvs));
        faces.push(triangle.indices);
    }
    let quantized = quantize_face_colors(&face_colors, color_limit as usize);
    check_cancelled(cancelled)?;
    for (color, slot) in face_colors.iter_mut().zip(quantized.face_slots) {
        let [r, g, b] = quantized.palette[usize::from(slot) - 1];
        *color = [r, g, b, 255];
    }

    Ok(BakedTexture {
        original,
        vertices,
        faces,
        face_colors,
        palette: quantized.palette,
        warning,
    })
}

fn check_cancelled(cancelled: &AtomicBool) -> Result<(), String> {
    if cancelled.load(Ordering::Relaxed) {
        return Err("Texture conversion was cancelled".into());
    }
    Ok(())
}

fn parse_glb(bytes: &[u8]) -> Result<(Document, &[u8]), String> {
    let size_at = |offset: usize| {
        bytes
            .get(offset..offset + 4)
            .and_then(|raw| raw.try_into().ok())
            .map(|raw| u32::from_le_bytes(raw) as usize)
    };
    if bytes.len() < 20 || size_at(8) != Some(bytes.len()) {
        return Err("Invalid GLB file length".into());
    }
    let json_length = size_at(12).unwrap_or_default();
    if json_length > MAX_JSON_BYTES {
        return Err("GLB exceeds the 4 MiB metadata budget".into());
    }
    let bin_offset = 20 + json_length;
    if json_length % 4 != 0
        || size_at(bin_offset).is_none_or(|length| {
            length % 4 != 0 || bytes.len().checked_sub(bin_offset + 8) != Some(length)
        })
    {
        return Err("Invalid GLB embedded buffer length or alignment".into());
    }
    let glb =
        gltf::binary::Glb::from_slice(bytes).map_err(|error| format!("Invalid GLB: {error}"))?;
    let root = gltf::json::Root::from_slice(&glb.json)
        .map_err(|error| format!("Invalid GLB metadata: {error}"))?;
    if root.asset.version != "2.0"
        || root
            .asset
            .min_version
            .as_deref()
            .is_some_and(|min| min != "2.0")
    {
        return Err("Texture import requires glTF 2.0".into());
    }
    if let Some(extension) = root.extensions_required.first() {
        return Err(format!("Unsupported required GLB extension: {extension}"));
    }
    // gltf 1.4.1's primitive validation looks up POSITION without checking its index first.
    use gltf::json::validation::Checked::Valid;
    if root
        .meshes
        .iter()
        .flat_map(|mesh| &mesh.primitives)
        .any(|primitive| {
            primitive
                .attributes
                .get(&Valid(Semantic::Positions))
                .is_some_and(|index| index.value() >= root.accessors.len())
        })
    {
        return Err("GLB POSITION accessor index is out of range".into());
    }
    let document =
        Document::from_json(root).map_err(|error| format!("Invalid GLB metadata: {error}"))?;
    let Some(std::borrow::Cow::Borrowed(bin)) = glb.bin else {
        return Err("GLB must contain an embedded binary buffer".into());
    };
    let buffers: Vec<_> = document.buffers().collect();
    let [buffer] = buffers.as_slice() else {
        return Err("GLB must contain one embedded binary buffer".into());
    };
    if !matches!(buffer.source(), gltf::buffer::Source::Bin) {
        return Err("GLB buffers must be embedded in the binary chunk".into());
    }
    if !(buffer.length()..=buffer.length().saturating_add(3)).contains(&bin.len()) {
        return Err("GLB binary buffer range does not match its declared size".into());
    }
    let bin = &bin[..buffer.length()];
    for view in document.views() {
        view_slice(&view, bin)?;
    }
    Ok((document, bin))
}

struct TextureInput<'a> {
    info: gltf::texture::Info<'a>,
    image: &'a [u8],
    mime_type: &'a str,
    wrap_s: WrappingMode,
    wrap_t: WrappingMode,
    mag_filter: Option<MagFilter>,
    min_filter: Option<MinFilter>,
    factor: [f32; 4],
}

struct GeometrySource<'a> {
    primitives: Vec<Primitive<'a>>,
    transform: Matrix4<f64>,
    mirrored: bool,
}

fn validate_scene<'a>(
    document: &'a Document,
    bin: &'a [u8],
    cancelled: &AtomicBool,
) -> Result<(Vec<GeometrySource<'a>>, TextureInput<'a>), String> {
    if document.animations().len() != 0 || document.skins().len() != 0 {
        return Err("Texture import requires a static GLB mesh".into());
    }
    let scene = document
        .default_scene()
        .or_else(|| {
            let mut scenes = document.scenes();
            (scenes.len() == 1).then(|| scenes.next()).flatten()
        })
        .ok_or("GLB must have a default or single scene")?;
    let mut stack: Vec<_> = scene
        .nodes()
        .map(|node| (node, Matrix4::<f64>::identity()))
        .collect();
    let mut visited = HashSet::new();
    let mut sources = Vec::new();
    let mut material = None;
    let mut vertex_count = 0;
    let mut face_count = 0;
    while let Some((node, parent)) = stack.pop() {
        check_cancelled(cancelled)?;
        if !visited.insert(node.index()) {
            return Err("GLB scene hierarchy must be a tree without repeated nodes".into());
        }
        if node.extension_value("EXT_mesh_gpu_instancing").is_some() || node.weights().is_some() {
            return Err("Texture import requires a static GLB mesh without instancing extensions or morph targets".into());
        }

        let local = node.transform().matrix();
        if local[0][3] != 0.0 || local[1][3] != 0.0 || local[2][3] != 0.0 || local[3][3] != 1.0 {
            return Err("GLB node transform must be affine".into());
        }
        let transform = parent * Matrix4::from_iterator(local.into_iter().flatten().map(f64::from));
        let determinant = transform.fixed_view::<3, 3>(0, 0).determinant();
        if !transform.iter().all(|x| x.is_finite())
            || !determinant.is_finite()
            || determinant == 0.0
        {
            return Err("GLB node transform is invalid or singular".into());
        }
        stack.extend(node.children().map(|child| (child, transform)));

        let Some(mesh) = node.mesh() else { continue };
        let mut primitives = Vec::new();
        for primitive in mesh.primitives() {
            if primitive.mode() != gltf::mesh::Mode::Triangles {
                return Err("Texture import requires triangle primitives".into());
            }
            if primitive.morph_targets().len() != 0 {
                return Err(
                    "Texture import requires a static GLB mesh without morph targets".into(),
                );
            }
            if primitive
                .extension_value("KHR_draco_mesh_compression")
                .is_some()
                || primitive
                    .extension_value("KHR_materials_variants")
                    .is_some()
                || primitive.get(&Semantic::Colors(0)).is_some()
            {
                return Err(
                    "Unsupported GLB primitive compression, material variants, or vertex colors"
                        .into(),
                );
            }
            let selected = primitive.material();
            let id = selected
                .index()
                .ok_or("GLB triangles require an explicit base-color material")?;
            let (material_id, texture) = match &mut material {
                Some(material) => material,
                slot @ None => slot.insert((id, validate_material(document, &selected, bin)?)),
            };
            if id != *material_id {
                return Err("Texture import supports one GLB material".into());
            }
            let position = primitive
                .get(&Semantic::Positions)
                .ok_or("GLB primitive is missing positions")?;
            let uv = primitive
                .get(&Semantic::TexCoords(texture.info.tex_coord()))
                .ok_or("GLB base-color texture is missing its UV coordinates")?;
            let indices = primitive.indices();
            let index_count = indices
                .as_ref()
                .map_or(position.count(), |accessor| accessor.count());
            vertex_count = bounded_sum(vertex_count, position.count(), MAX_FACES * 3)?;
            face_count = bounded_sum(face_count, index_count / 3, MAX_FACES)?;
            if index_count < 3 || index_count % 3 != 0 {
                return Err("GLB triangle index count must be a positive multiple of three".into());
            }
            if position.dimensions() != Dimensions::Vec3
                || position.data_type() != DataType::F32
                || position.normalized()
            {
                return Err("GLB vertex positions must be floating-point VEC3 coordinates".into());
            }
            if uv.dimensions() != Dimensions::Vec2
                || uv.count() != position.count()
                || !matches!(
                    (uv.data_type(), uv.normalized()),
                    (DataType::F32, false) | (DataType::U8 | DataType::U16, true)
                )
            {
                return Err("Invalid GLB UV accessor type or count".into());
            }
            validate_accessor(document, position, bin)?;
            validate_accessor(document, uv, bin)?;
            if let Some(indices) = indices {
                if indices.dimensions() != Dimensions::Scalar
                    || indices.normalized()
                    || !matches!(
                        indices.data_type(),
                        DataType::U8 | DataType::U16 | DataType::U32
                    )
                    || indices.view().is_some_and(|view| view.stride().is_some())
                {
                    return Err("Invalid GLB triangle index accessor".into());
                }
                validate_accessor(document, indices, bin)?;
            }
            primitives.push(primitive);
        }
        if !primitives.is_empty() {
            sources.push(GeometrySource {
                primitives,
                transform,
                mirrored: determinant < 0.0,
            });
        }
    }
    let (_, texture) = material.ok_or("GLB scene contains no textured triangles")?;
    Ok((sources, texture))
}

fn bounded_sum(total: usize, next: usize, maximum: usize) -> Result<usize, String> {
    total
        .checked_add(next)
        .filter(|&sum| sum <= maximum)
        .ok_or_else(|| format!("GLB exceeds the {MAX_FACES} triangle or associated vertex budget"))
}

fn validate_material<'a>(
    document: &'a Document,
    material: &gltf::Material<'a>,
    bin: &'a [u8],
) -> Result<TextureInput<'a>, String> {
    let pbr = material.pbr_metallic_roughness();
    let factor = pbr.base_color_factor();
    if material.alpha_mode() != gltf::material::AlphaMode::Opaque || factor[3] != 1.0 {
        return Err("GLB base-color material must be opaque".into());
    }
    if !factor.into_iter().all(|x| (0.0..=1.0).contains(&x))
        || material
            .extensions()
            .is_some_and(|ext| !ext.keys().all(|key| key == "KHR_materials_unlit"))
        || pbr.extensions().is_some_and(|ext| !ext.is_empty())
        || pbr.metallic_roughness_texture().is_some()
        || material.normal_texture().is_some()
        || material.occlusion_texture().is_some()
        || material.emissive_texture().is_some()
    {
        return Err("Unsupported GLB material. Use a single base-color texture".into());
    }
    let info = pbr
        .base_color_texture()
        .ok_or("GLB material has no base-color texture")?;
    if info.extensions().is_some_and(|ext| !ext.is_empty()) {
        return Err("GLB base-color texture extensions are unsupported".into());
    }
    let texture = info.texture();
    if texture.extensions().is_some_and(|ext| !ext.is_empty()) {
        return Err("GLB base-color texture extensions are unsupported".into());
    }
    let source = texture.source();
    // gltf::Image::source assumes both bufferView and mimeType were provided.
    let image_metadata = &document.as_json().images[source.index()];
    if image_metadata.uri.is_some() || image_metadata.buffer_view.is_none() {
        return Err("GLB base-color image must be embedded in a binary buffer view".into());
    }
    if image_metadata.mime_type.is_none() {
        return Err("GLB base-color image is missing a PNG or JPEG media type".into());
    }
    let image = texture_document_image(&source, bin)?;
    let sampler = texture.sampler();

    Ok(TextureInput {
        info,
        image: image.0,
        mime_type: image.1,
        wrap_s: sampler.wrap_s(),
        wrap_t: sampler.wrap_t(),
        mag_filter: sampler.mag_filter(),
        min_filter: sampler.min_filter(),
        factor,
    })
}

fn texture_document_image<'a>(
    source: &gltf::Image<'a>,
    bin: &'a [u8],
) -> Result<(&'a [u8], &'a str), String> {
    let gltf::image::Source::View { view, mime_type } = source.source() else {
        return Err("GLB base-color image must be embedded in a binary buffer view".into());
    };
    if !matches!(mime_type, "image/png" | "image/jpeg") {
        return Err("GLB base-color image must be an embedded PNG or JPEG".into());
    }
    if view.length() > MAX_IMAGE_BYTES {
        return Err("GLB base-color image exceeds the 8 MiB encoded image budget".into());
    }
    Ok((view_slice(&view, bin)?, mime_type))
}

fn view_slice<'a>(view: &gltf::buffer::View<'_>, bin: &'a [u8]) -> Result<&'a [u8], String> {
    bin.get(view.offset()..)
        .and_then(|bytes| bytes.get(..view.length()))
        .ok_or_else(|| "GLB buffer view is out of range".into())
}

fn validate_accessor(
    document: &Document,
    accessor: Accessor<'_>,
    bin: &[u8],
) -> Result<(), String> {
    let count = accessor.count();
    if let Some(view) = accessor.view() {
        check_accessor_range(
            &view,
            accessor.offset(),
            count,
            accessor.size(),
            accessor.data_type().size(),
        )?;
    } else if accessor.sparse().is_none() {
        return Err("GLB geometry accessor is missing its buffer view".into());
    }
    if let Some(sparse) = accessor.sparse() {
        use gltf::json::validation::Checked::Valid;
        let sparse_metadata = document.as_json().accessors[accessor.index()]
            .sparse
            .as_ref()
            .ok_or("GLB sparse accessor is missing metadata")?;
        let size = match sparse_metadata.indices.component_type {
            Valid(gltf::json::accessor::IndexComponentType(DataType::U8)) => 1,
            Valid(gltf::json::accessor::IndexComponentType(DataType::U16)) => 2,
            Valid(gltf::json::accessor::IndexComponentType(DataType::U32)) => 4,
            _ => return Err("Invalid GLB sparse accessor index type".into()),
        };
        let indices = sparse.indices();
        let values = sparse.values();
        let index_view = indices.view();
        let value_view = values.view();
        if sparse.count() > count || index_view.stride().is_some() || value_view.stride().is_some()
        {
            return Err("Invalid GLB sparse accessor count or layout".into());
        }
        check_accessor_range(&index_view, indices.offset(), sparse.count(), size, size)?;
        check_accessor_range(
            &value_view,
            values.offset(),
            sparse.count(),
            accessor.size(),
            accessor.data_type().size(),
        )?;
        let bytes = view_slice(&index_view, bin)?;
        let mut previous = None;
        for raw in bytes[indices.offset()..][..sparse.count() * size].chunks_exact(size) {
            let mut aligned = [0; 4];
            aligned[..size].copy_from_slice(raw);
            let index = u32::from_le_bytes(aligned) as usize;
            if index >= count || previous.is_some_and(|last| index <= last) {
                return Err("GLB sparse accessor indices must be in range and increasing".into());
            }
            previous = Some(index);
        }
    }
    Ok(())
}

fn check_accessor_range(
    view: &gltf::buffer::View<'_>,
    offset: usize,
    count: usize,
    size: usize,
    alignment: usize,
) -> Result<(), String> {
    let stride = view.stride().unwrap_or(size);
    let end = count
        .checked_sub(1)
        .and_then(|steps| steps.checked_mul(stride))
        .and_then(|length| length.checked_add(size))
        .and_then(|length| length.checked_add(offset));
    if stride < size
        || stride % alignment != 0
        || offset % alignment != 0
        || view.offset() % alignment != 0
        || end.is_none_or(|end| end > view.length())
    {
        return Err("GLB accessor range or alignment is invalid".into());
    }
    Ok(())
}

#[derive(Clone, Copy)]
struct Triangle {
    indices: [u32; 3],
    uvs: [[f32; 2]; 3],
}

impl Triangle {
    fn normal(&self, vertices: &[[f32; 3]]) -> Vector3<f32> {
        let [a, b, c] = self
            .indices
            .map(|index| Vector3::from(vertices[index as usize]));
        (b - a).cross(&(c - a))
    }
}

fn read_geometry(
    sources: &[GeometrySource<'_>],
    bin: &[u8],
    uv_set: u32,
    vertices: &mut Vec<[f32; 3]>,
    cancelled: &AtomicBool,
) -> Result<Vec<Triangle>, String> {
    let mut triangles = Vec::new();
    for source in sources {
        // A single node may duplicate positions for seams and hard normals.
        // Keep the positions shared while leaving UVs attached to face corners.
        let mut shared_positions = HashMap::new();
        for primitive in &source.primitives {
            let reader = primitive.reader(|_| Some(bin));
            let positions: Vec<_> = reader
                .read_positions()
                .ok_or("Invalid GLB positions")?
                .collect();
            let uvs: Vec<_> = reader
                .read_tex_coords(uv_set)
                .ok_or("Invalid GLB UVs")?
                .into_f32()
                .collect();
            let indices: Vec<_> = match reader.read_indices() {
                Some(indices) => indices.into_u32().collect(),
                None => (0..positions.len() as u32).collect(),
            };
            let mut indices_by_vertex = vec![None; positions.len()];
            for indices in indices.chunks_exact(3) {
                check_cancelled(cancelled)?;
                let mut triangle = Triangle {
                    indices: [0; 3],
                    uvs: [[0.0; 2]; 3],
                };
                for (corner, &index) in indices.iter().enumerate() {
                    let index = index as usize;
                    let position = *positions
                        .get(index)
                        .ok_or("GLB triangle index is out of range")?;
                    let uv = uvs[index];
                    if !uv.into_iter().all(f32::is_finite) {
                        return Err("GLB UVs must be finite".into());
                    }
                    let vertex = match indices_by_vertex[index] {
                        Some(vertex) => vertex,
                        None => {
                            let p = source.transform
                                * Vector4::new(
                                    position[0] as f64,
                                    position[1] as f64,
                                    position[2] as f64,
                                    1.0,
                                );
                            // glTF uses meters and Y up. CYM uses millimeters and Z up.
                            let point = [
                                (p.x * 1000.0) as f32,
                                (-p.z * 1000.0) as f32,
                                (p.y * 1000.0) as f32,
                            ];
                            // This bound also keeps f32 squared cross products finite in mesh initialization.
                            if !point.into_iter().all(|axis| axis.abs() <= 1e9) {
                                return Err("GLB vertex coordinates are non-finite or exceed mesh precision limits".into());
                            }
                            let key =
                                point.map(|axis| if axis == 0.0 { 0 } else { axis.to_bits() });
                            let vertex = *shared_positions.entry(key).or_insert_with(|| {
                                let vertex = vertices.len() as u32;
                                vertices.push(point);
                                vertex
                            });
                            indices_by_vertex[index] = Some(vertex);
                            vertex
                        }
                    };
                    triangle.indices[corner] = vertex;
                    triangle.uvs[corner] = uv;
                }
                if source.mirrored {
                    triangle.indices.swap(1, 2);
                    triangle.uvs.swap(1, 2);
                }
                if triangle.normal(vertices).norm_squared() == 0.0 {
                    return Err("GLB contains a degenerate triangle".into());
                }
                triangles.push(triangle);
            }
        }
    }
    Ok(triangles)
}

fn decode_texture(bytes: &[u8], mime: &str) -> Result<RgbImage, String> {
    let invalid = |error| format!("Invalid GLB base-color image: {error}");
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(MAX_IMAGE_EDGE);
    limits.max_image_height = Some(MAX_IMAGE_EDGE);
    limits.max_alloc = Some(MAX_IMAGE_PIXELS * 8);
    let decoded = if mime == "image/png" {
        let decoder = image::codecs::png::PngDecoder::with_limits(Cursor::new(bytes), limits)
            .map_err(invalid)?;
        if decoder.is_apng().map_err(invalid)? {
            return Err("GLB base-color image must be a static PNG or JPEG".into());
        }
        decode_bounded_image(decoder)?
    } else {
        let mut decoder =
            image::codecs::jpeg::JpegDecoder::new(Cursor::new(bytes)).map_err(invalid)?;
        decoder.set_limits(limits).map_err(invalid)?;
        decode_bounded_image(decoder)?
    };

    use DynamicImage::*;
    let translucent = match &decoded {
        ImageLumaA8(image) => image.pixels().any(|pixel| pixel[1] != u8::MAX),
        ImageRgba8(image) => image.pixels().any(|pixel| pixel[3] != u8::MAX),
        ImageLumaA16(image) => image.pixels().any(|pixel| pixel[1] != u16::MAX),
        ImageRgba16(image) => image.pixels().any(|pixel| pixel[3] != u16::MAX),
        _ => false,
    };
    if translucent {
        return Err("GLB base-color image must be opaque".into());
    }
    Ok(decoded.into_rgb8())
}

fn decode_bounded_image(decoder: impl ImageDecoder) -> Result<DynamicImage, String> {
    let (width, height) = decoder.dimensions();
    if u64::from(width) * u64::from(height) > MAX_IMAGE_PIXELS {
        return Err("GLB base-color image exceeds the decoded pixel budget".into());
    }
    DynamicImage::from_decoder(decoder)
        .map_err(|error| format!("Invalid GLB base-color image: {error}"))
}

fn longest_texture_edge(triangles: &[Triangle], (width, height): (u32, u32)) -> f64 {
    triangles
        .iter()
        .flat_map(|triangle| {
            let [a, b, c] = triangle.uvs;
            [(a, b), (b, c), (c, a)]
        })
        .map(|(a, b)| {
            let u = (f64::from(a[0]) - f64::from(b[0])).abs() * f64::from(width);
            let v = (f64::from(a[1]) - f64::from(b[1])).abs() * f64::from(height);
            u.max(v)
        })
        .fold(0.0, f64::max)
}

fn subdivide(
    triangles: &[Triangle],
    vertices: &mut Vec<[f32; 3]>,
    cancelled: &AtomicBool,
) -> Result<Option<Vec<Triangle>>, String> {
    let mut edge_midpoints = HashMap::new();
    let mut next = Vec::with_capacity(triangles.len() * 4);
    let mut midpoint = |a: u32, b: u32, vertices: &mut Vec<[f32; 3]>| {
        let key = if a < b { (a, b) } else { (b, a) };
        *edge_midpoints.entry(key).or_insert_with(|| {
            let index = vertices.len() as u32;
            let position = std::array::from_fn(|axis| {
                vertices[a as usize][axis] * 0.5 + vertices[b as usize][axis] * 0.5
            });
            vertices.push(position);
            index
        })
    };
    for &triangle in triangles {
        check_cancelled(cancelled)?;
        let Triangle {
            indices: [a, b, c],
            uvs: [u, v, w],
        } = triangle;
        let ab = midpoint(a, b, vertices);
        let bc = midpoint(b, c, vertices);
        let ca = midpoint(c, a, vertices);
        let uv_mid =
            |a: [f32; 2], b: [f32; 2]| std::array::from_fn(|axis| a[axis] * 0.5 + b[axis] * 0.5);
        let uv = uv_mid(u, v);
        let vw = uv_mid(v, w);
        let wu = uv_mid(w, u);
        let children = [
            Triangle {
                indices: [a, ab, ca],
                uvs: [u, uv, wu],
            },
            Triangle {
                indices: [ab, b, bc],
                uvs: [uv, v, vw],
            },
            Triangle {
                indices: [ca, bc, c],
                uvs: [wu, vw, w],
            },
            Triangle {
                indices: [ab, bc, ca],
                uvs: [uv, vw, wu],
            },
        ];
        // At f32 precision a midpoint can collapse or flip a child triangle.
        let normal = triangle.normal(vertices);
        if children
            .iter()
            .any(|child| child.normal(vertices).dot(&normal) <= 0.0)
        {
            return Ok(None);
        }
        next.extend(children);
    }
    Ok(Some(next))
}

fn wrap_name(wrap: WrappingMode) -> &'static str {
    match wrap {
        WrappingMode::ClampToEdge => "clamp",
        WrappingMode::MirroredRepeat => "mirror",
        WrappingMode::Repeat => "repeat",
    }
}

fn texel_axis(uv: f64, extent: u32, wrap: WrappingMode) -> (u32, u32, f64) {
    let size = i64::from(extent);
    let normalized = match wrap {
        WrappingMode::ClampToEdge => uv.clamp(0.0, 1.0),
        WrappingMode::Repeat => uv.rem_euclid(1.0),
        WrappingMode::MirroredRepeat => uv.rem_euclid(2.0),
    };
    let texel = normalized * f64::from(extent) - 0.5;
    let floor = texel.floor() as i64;
    let wrap_index = |index: i64| -> u32 {
        match wrap {
            WrappingMode::ClampToEdge => index.clamp(0, size - 1) as u32,
            WrappingMode::Repeat => index.rem_euclid(size) as u32,
            WrappingMode::MirroredRepeat => {
                let mirrored = index.rem_euclid(size * 2);
                mirrored.min(size * 2 - 1 - mirrored) as u32
            }
        }
    };
    (
        wrap_index(floor),
        wrap_index(floor + 1),
        texel - floor as f64,
    )
}

struct TextureSampler<'a> {
    image: &'a RgbImage,
    wrap_s: WrappingMode,
    wrap_t: WrappingMode,
    nearest: bool,
    factor: [f32; 4],
    srgb_to_linear: [f32; 256],
}

impl TextureSampler<'_> {
    fn face_color(&self, uvs: [[f32; 2]; 3]) -> [u8; 4] {
        // Keep the edge and corner samples inside each face's UV island.
        const WEIGHTS: [[f64; 3]; 7] = [
            [3.0 / 4.0, 1.0 / 8.0, 1.0 / 8.0],
            [1.0 / 8.0, 3.0 / 4.0, 1.0 / 8.0],
            [1.0 / 8.0, 1.0 / 8.0, 3.0 / 4.0],
            [1.0 / 8.0, 7.0 / 16.0, 7.0 / 16.0],
            [7.0 / 16.0, 1.0 / 8.0, 7.0 / 16.0],
            [7.0 / 16.0, 7.0 / 16.0, 1.0 / 8.0],
            [1.0 / 3.0; 3],
        ];
        let mut total = [0.0; 3];
        for weights in WEIGHTS {
            let [u, v] = std::array::from_fn(|axis| {
                uvs.into_iter()
                    .zip(weights)
                    .map(|(uv, weight)| f64::from(uv[axis]) * weight)
                    .sum()
            });
            let sample = self.sample(u, v);
            for (channel, value) in total.iter_mut().zip(sample) {
                *channel += value;
            }
        }
        let channels = std::array::from_fn::<_, 3, _>(|channel| {
            let average = total[channel] / WEIGHTS.len() as f32 * self.factor[channel];
            (linear_to_srgb(average) * 255.0).round() as u8
        });
        [channels[0], channels[1], channels[2], 255]
    }

    fn sample(&self, u: f64, v: f64) -> [f32; 3] {
        let (x0, x1, x) = texel_axis(u, self.image.width(), self.wrap_s);
        // The glTF coordinate origin is the upper-left texel.
        let (y0, y1, y) = texel_axis(v, self.image.height(), self.wrap_t);
        let linear = |value: u8| self.srgb_to_linear[value as usize];
        if self.nearest {
            let ix = if x < 0.5 { x0 } else { x1 };
            let iy = if y < 0.5 { y0 } else { y1 };
            return self.image.get_pixel(ix, iy).0.map(linear);
        }
        let (x, y) = (x as f32, y as f32);
        let top_left = self.image.get_pixel(x0, y0);
        let top_right = self.image.get_pixel(x1, y0);
        let bottom_left = self.image.get_pixel(x0, y1);
        let bottom_right = self.image.get_pixel(x1, y1);
        std::array::from_fn(|channel| {
            let top = linear(top_left[channel]) * (1.0 - x) + linear(top_right[channel]) * x;
            let bottom =
                linear(bottom_left[channel]) * (1.0 - x) + linear(bottom_right[channel]) * x;
            top * (1.0 - y) + bottom * y
        })
    }
}

fn srgb_to_linear(value: f32) -> f32 {
    if value <= 0.04045 {
        value / 12.92
    } else {
        ((value + 0.055) / 1.055).powf(2.4)
    }
}

fn linear_to_srgb(value: f32) -> f32 {
    if value <= 0.0031308 {
        value * 12.92
    } else {
        1.055 * value.powf(1.0 / 2.4) - 0.055
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::ImageEncoder;
    use serde_json::{json, Value};

    const POSITIONS: [[f32; 3]; 6] = [
        [0.0, 0.0, 0.0],
        [0.02, 0.0, 0.0],
        [0.02, 0.01, 0.0],
        [0.0, 0.01, 0.0],
        [0.0, 0.0, 0.0],
        [0.02, 0.01, 0.0],
    ];

    fn png(width: u32, height: u32, pixels: &[u8]) -> Vec<u8> {
        let mut image = Vec::new();
        image::codecs::png::PngEncoder::new(&mut image)
            .write_image(pixels, width, height, image::ExtendedColorType::Rgba8)
            .unwrap();
        image
    }

    fn fixture(
        positions: &[[f32; 3]],
        uvs: &[[f32; 2]],
        indices: &[u16],
        image: &[u8],
        change_json: impl FnOnce(&mut Value),
    ) -> Vec<u8> {
        let mut bin = Vec::new();
        bin.extend(positions.iter().flatten().flat_map(|x| x.to_le_bytes()));
        let uv_offset = bin.len();
        bin.extend(uvs.iter().flatten().flat_map(|x| x.to_le_bytes()));
        let indices_offset = bin.len();
        bin.extend(indices.iter().flat_map(|x| x.to_le_bytes()));
        let index_length = bin.len() - indices_offset;
        bin.resize(bin.len().next_multiple_of(4), 0);
        let image_offset = bin.len();
        bin.extend_from_slice(image);
        let mut document = json!({
            "asset": {"version": "2.0"},
            "scene": 0,
            "scenes": [{"nodes": [0]}],
            "nodes": [{"translation":[0.01, 0.02, 0.03], "children":[1]}, {"mesh":0, "scale":[-1.0, 1.0, 1.0]}],
            "meshes":[{"primitives":[{"attributes":{"POSITION":0,"TEXCOORD_0":1},"indices":2,"material":0}]}],
            "materials":[{"pbrMetallicRoughness":{"baseColorTexture":{"index":0}}}],
            "textures":[{"source":0, "sampler":0}],
            "samplers":[{"wrapS":33071,"wrapT":33071}],
            "images":[{"bufferView":3,"mimeType":"image/png"}],
            "accessors":[
                {"bufferView":0,"componentType":5126,"count":positions.len(),"type":"VEC3",
                 "min":[0.0,0.0,0.0],"max":[0.02,0.01,0.0]},
                {"bufferView":1,"componentType":5126,"count":uvs.len(),"type":"VEC2"},
                {"bufferView":2,"componentType":5123,"count":indices.len(),"type":"SCALAR"}
            ],
            "bufferViews":[
                {"buffer":0,"byteLength":uv_offset,"target":34962},
                {"buffer":0,"byteOffset":uv_offset,"byteLength":indices_offset-uv_offset,"target":34962},
                {"buffer":0,"byteOffset":indices_offset,"byteLength":index_length,"target":34963},
                {"buffer":0,"byteOffset":image_offset,"byteLength":image.len()}
            ],
            "buffers":[{"byteLength":bin.len()}]
        });
        change_json(&mut document);
        let mut document = serde_json::to_vec(&document).unwrap();
        document.resize(document.len().next_multiple_of(4), b' ');
        bin.resize(bin.len().next_multiple_of(4), 0);
        let mut bytes = b"glTF".to_vec();
        bytes.extend(2u32.to_le_bytes());
        bytes.extend(((28 + document.len() + bin.len()) as u32).to_le_bytes());
        bytes.extend((document.len() as u32).to_le_bytes());
        bytes.extend(b"JSON");
        bytes.extend(document);
        bytes.extend((bin.len() as u32).to_le_bytes());
        bytes.extend(b"BIN\0");
        bytes.extend(bin);
        bytes
    }

    fn seam_fixture(change_json: impl FnOnce(&mut Value)) -> Vec<u8> {
        fixture(
            &POSITIONS,
            &[
                [0.5, 0.25],
                [0.5, 0.25],
                [0.5, 0.25],
                [0.5, 0.75],
                [0.5, 0.75],
                [0.5, 0.75],
            ],
            &[0, 1, 2, 4, 5, 3],
            &png(1, 2, &[255, 0, 0, 255, 0, 0, 255, 255]),
            change_json,
        )
    }

    #[test]
    fn inconsistent_binary_chunk_size_is_rejected_before_parsing() {
        let mut bytes = seam_fixture(|_| {});
        let json_length = u32::from_le_bytes(bytes[12..16].try_into().unwrap()) as usize;
        let offset = 20 + json_length;
        bytes[offset..offset + 4].copy_from_slice(&(u32::MAX - 3).to_le_bytes());
        assert!(parse_glb(&bytes).err().unwrap().contains("buffer length"));
    }

    fn import(bytes: &[u8], color_limit: u8, cancelled: bool) -> Result<BakedTexture, String> {
        static SEQUENCE: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let sequence = SEQUENCE.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let path =
            std::env::temp_dir().join(format!("cym-texture-{}-{sequence}.glb", std::process::id()));
        std::fs::write(&path, bytes).unwrap();
        let result = load_texture(&path, color_limit, &AtomicBool::new(cancelled));
        std::fs::remove_file(path).unwrap();
        result
    }

    #[test]
    fn indexes_transformed_geometry_and_reverses_uv_winding_with_a_mirrored_node() {
        let result = import(
            &seam_fixture(|doc| {
                doc["accessors"][1]["byteOffset"] = json!(0);
            }),
            16,
            false,
        )
        .unwrap();

        assert_eq!(result.original.vertices.len(), 12);
        assert_eq!(result.original.faces, [0, 2, 1, 0, 3, 2]);
        let expected = [
            10.0, -30.0, 20.0, -10.0, -30.0, 20.0, -10.0, -30.0, 30.0, 10.0, -30.0, 30.0,
        ];
        assert!(result
            .original
            .vertices
            .iter()
            .zip(expected)
            .all(|(a, b)| (a - b).abs() < 1e-4));
        assert_eq!(
            result.original.corner_uvs,
            [0.5, 0.25, 0.5, 0.25, 0.5, 0.25, 0.5, 0.75, 0.5, 0.75, 0.5, 0.75]
        );
        let mirrored = fixture(
            &POSITIONS[..3],
            &[[0.25, 0.25], [0.5, 0.25], [0.5, 0.75]],
            &[0, 1, 2],
            &png(1, 1, &[255, 0, 0, 255]),
            |_| {},
        );
        assert_eq!(
            import(&mirrored, 16, false).unwrap().original.corner_uvs,
            [0.25, 0.25, 0.5, 0.75, 0.5, 0.25]
        );
    }

    #[test]
    fn subdivisions_share_geometric_midpoints_and_preserve_seam_texels() {
        let result = import(&seam_fixture(|_| {}), 16, false).unwrap();
        assert_eq!(result.faces.len(), 8);
        assert_eq!(result.vertices.len(), 9);
        assert_eq!(result.face_colors[..4], [[255, 0, 0, 255]; 4]);
        assert_eq!(result.face_colors[4..], [[0, 0, 255, 255]; 4]);
        assert!(result
            .original
            .image_url
            .starts_with("data:image/png;base64,"));
    }

    #[test]
    fn subdivision_keeps_the_last_complete_mesh_when_midpoints_lose_precision() {
        let bytes = fixture(
            &POSITIONS[..3],
            &[[0.0, 0.5], [1.0, 0.5], [1.0, 0.5]],
            &[0, 1, 2],
            &png(64, 1, &[255; 256]),
            |doc| {
                doc["nodes"][0]["translation"] = json!([0.2, 0.2, 0.2]);
                doc["nodes"][1]["scale"] = json!([0.00003, 0.00003, 0.00003]);
            },
        );
        let result = import(&bytes, 16, false).unwrap();
        assert_eq!(result.warning, Some("precision"));
        assert!(result.faces.len() > 1 && result.faces.len() < 4096);
        assert!(result.faces.iter().all(|face| {
            let [a, b, c] = face.map(|index| Vector3::from(result.vertices[index as usize]));
            (b - a).cross(&(c - a)).norm_squared() > 0.0
        }));
        let referenced: HashSet<_> = result.faces.iter().flatten().copied().collect();
        assert_eq!(referenced.len(), result.vertices.len());
    }

    #[test]
    fn reports_the_face_limit_while_retaining_a_valid_source_mesh() {
        let image = png(2, 1, &[255; 8]);
        let make_mesh = |face_count| {
            let indices = [0, 1, 2].repeat(face_count);
            fixture(
                &POSITIONS[..3],
                &[[0.0, 0.5], [1.0, 0.5], [1.0, 0.5]],
                &indices,
                &image,
                |_| {},
            )
        };
        let result = import(&make_mesh(50_001), 16, false).unwrap();
        assert_eq!(result.faces.len(), 50_001);
        assert_eq!(result.warning, Some("budget"));
        assert!(import(&make_mesh(200_001), 16, false)
            .unwrap_err()
            .contains("triangle or associated vertex budget"));
    }

    #[test]
    fn reports_the_maximum_subdivision_level_for_a_large_texture() {
        let bytes = fixture(
            &POSITIONS[..3],
            &[[0.0, 0.5], [1.0, 0.5], [1.0, 0.5]],
            &[0, 1, 2],
            &png(128, 1, &[255; 512]),
            |_| {},
        );
        let result = import(&bytes, 16, false).unwrap();
        assert_eq!(result.warning, Some("level-limit"));
        assert_eq!(result.faces.len(), 4096);
    }

    #[test]
    fn explicit_nearest_filter_does_not_blend_with_the_adjacent_texel() {
        let bytes = fixture(
            &POSITIONS[..3],
            &[[0.45, 0.5]; 3],
            &[0, 1, 2],
            &png(2, 1, &[255, 0, 0, 255, 0, 0, 255, 255]),
            |doc| {
                doc["samplers"][0]["magFilter"] = json!(9728);
                doc["samplers"][0]["minFilter"] = json!(9984);
            },
        );
        let result = import(&bytes, 16, false).unwrap();
        assert_eq!(result.face_colors, [[255, 0, 0, 255]; 4]);
        let original = serde_json::to_value(result.original).unwrap();
        assert_eq!(original["magFilter"], 9728);
        assert_eq!(original["minFilter"], 9984);
    }

    #[test]
    fn reapplies_the_linear_tint_and_wraps_negative_uvs_per_axis() {
        let bytes = fixture(
            &POSITIONS[..3],
            &[[-0.25, -0.25]; 3],
            &[0, 1, 2],
            &png(
                2,
                2,
                &[
                    255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255, 255, 255, 255, 255,
                ],
            ),
            |doc| {
                doc["samplers"][0] = json!({"wrapS":33648,"wrapT":10497});
                doc["materials"][0]["pbrMetallicRoughness"]["baseColorFactor"] =
                    json!([1.0, 1.0, 0.25, 1.0]);
            },
        );
        let result = import(&bytes, 16, false).unwrap();
        assert_eq!(result.face_colors, [[0, 0, 137, 255]; 4]);
        assert_eq!(
            (
                result.original.wrap_s.as_str(),
                result.original.wrap_t.as_str()
            ),
            ("mirror", "repeat")
        );
    }

    #[test]
    fn reduces_seventeen_texture_colors_to_the_selected_limit() {
        let pixels: Vec<_> = (0..17).flat_map(|i| [i * 15, 0, 0, 255]).collect();
        let positions: Vec<_> = (0..17)
            .flat_map(|i| {
                let offset = i as f32 * 0.001;
                [[offset, 0.0, 0.0], [offset, 0.01, 0.0], [offset, 0.0, 0.01]]
            })
            .collect();
        let uvs: Vec<_> = (0..17)
            .flat_map(|i| [[(i as f32 + 0.5) / 17.0, 0.5]; 3])
            .collect();
        let bytes = fixture(
            &positions,
            &uvs,
            &(0..51).collect::<Vec<_>>(),
            &png(17, 1, &pixels),
            |_| {},
        );
        for limit in [1, 16] {
            let result = import(&bytes, limit, false).unwrap();
            assert_eq!(result.palette.len(), limit as usize);
            assert!(result
                .face_colors
                .iter()
                .all(|c| c[3] == 255 && result.palette.contains(&[c[0], c[1], c[2]])));
        }
    }

    #[test]
    fn rejects_missing_uv_external_resources_and_invalid_geometry() {
        for (bytes, message) in [
            (
                seam_fixture(|doc| {
                    doc["meshes"][0]["primitives"][0]["attributes"]
                        .as_object_mut()
                        .unwrap()
                        .remove("TEXCOORD_0");
                }),
                "UV",
            ),
            (
                seam_fixture(|doc| {
                    doc["images"][0] = json!({"uri":"other.png"});
                }),
                "embedded",
            ),
            (
                seam_fixture(|doc| {
                    doc["accessors"][0]["count"] = json!(1_000_000);
                }),
                "budget",
            ),
            (
                seam_fixture(|doc| {
                    doc["bufferViews"][2]["byteLength"] = json!(usize::MAX);
                }),
                "range",
            ),
            (
                fixture(
                    &POSITIONS,
                    &[[0.5, 0.5]; 6],
                    &[0, 1, 30],
                    &png(1, 1, &[0, 0, 0, 255]),
                    |_| {},
                ),
                "index",
            ),
        ] {
            assert!(
                import(&bytes, 16, false).unwrap_err().contains(message),
                "expected {message}"
            );
        }
    }

    #[test]
    fn rejects_bad_images_and_nonopaque_materials() {
        for (bytes, message) in [
            (
                fixture(
                    &POSITIONS,
                    &[[0.5, 0.5]; 6],
                    &[0, 1, 2],
                    b"broken PNG",
                    |_| {},
                ),
                "image",
            ),
            (
                fixture(
                    &POSITIONS,
                    &[[0.5, 0.5]; 6],
                    &[0, 1, 2],
                    &png(1, 1, &[0, 0, 0, 254]),
                    |_| {},
                ),
                "opaque",
            ),
            (
                seam_fixture(|doc| {
                    doc["materials"][0]["alphaMode"] = json!("BLEND");
                }),
                "opaque",
            ),
        ] {
            assert!(
                import(&bytes, 16, false).unwrap_err().contains(message),
                "expected {message}"
            );
        }
    }

    #[test]
    fn rejects_invalid_palette_sizes_and_cancelled_loads() {
        let bytes = seam_fixture(|_| {});
        for limit in [0, 17] {
            assert!(import(&bytes, limit, false).unwrap_err().contains("1–16"));
        }
        assert!(import(&bytes, 16, true).unwrap_err().contains("cancelled"));
    }
}
