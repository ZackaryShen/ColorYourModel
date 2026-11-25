//! Parser for the 3MF core `3dmodel.model` part and the production-extension
//! parts it references through component `p:path` attributes.
//!
//! Three properties of the real files shape this module:
//!
//! - The main part may hold no geometry at all: it only carries an
//!   `<object>` whose `<components>` point at `/3D/Objects/*.model` parts.
//!   Those references are package paths, so every referenced part is parsed
//!   once and cached under its package-relative name.
//! - Production-extension attributes are namespaced (`p:path`, `p:transform`,
//!   `p:UUID`). Elements and attributes are matched by local name, so a
//!   prefix does not hide a field and unknown attributes are ignored.
//! - `model@unit` is not always millimetres. Vertices leave this module in
//!   millimetres, and so do transform translations, so a transform can be
//!   applied to the converted vertices directly.
//!
//! A malformed document is an `Err`; nothing here panics on untrusted input.

use std::collections::{HashMap, HashSet, VecDeque};

use quick_xml::events::{BytesEnd, BytesStart, Event};
use quick_xml::Reader;

use super::opc::{part_key, Package};
use super::{GeometryBudget, MAX_EDITOR_FACES};

/// A 3MF transform: row-vector 3x4 affine matrix, `p' = p · M3x3 + t`.
///
/// The twelve values are `m00 m01 m02 m10 m11 m12 m20 m21 m22 tx ty tz` in
/// file order, i.e. the first nine are the 3x3 linear part row by row and the
/// last three are the translation.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Transform(pub [f32; 12]);

impl Transform {
    /// The identity transform.
    pub fn identity() -> Transform {
        Transform([1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0])
    }

    /// The 3MF composition: `other` is applied first, then `self`.
    ///
    /// A build item's transform is the outer transform, so the placement of a
    /// component is `build.transform.compose(component.transform)`. With row
    /// vectors `p' = p · C · B`; the composed translation is therefore
    /// `t_c · B3x3 + t_b`, not `t_c + t_b` when the build item scales.
    pub fn compose(&self, other: Transform) -> Transform {
        let b = &self.0;
        let c = &other.0;
        Transform([
            c[0] * b[0] + c[1] * b[3] + c[2] * b[6],
            c[0] * b[1] + c[1] * b[4] + c[2] * b[7],
            c[0] * b[2] + c[1] * b[5] + c[2] * b[8],
            c[3] * b[0] + c[4] * b[3] + c[5] * b[6],
            c[3] * b[1] + c[4] * b[4] + c[5] * b[7],
            c[3] * b[2] + c[4] * b[5] + c[5] * b[8],
            c[6] * b[0] + c[7] * b[3] + c[8] * b[6],
            c[6] * b[1] + c[7] * b[4] + c[8] * b[7],
            c[6] * b[2] + c[7] * b[5] + c[8] * b[8],
            c[9] * b[0] + c[10] * b[3] + c[11] * b[6] + b[9],
            c[9] * b[1] + c[10] * b[4] + c[11] * b[7] + b[10],
            c[9] * b[2] + c[10] * b[5] + c[11] * b[8] + b[11],
        ])
    }

    /// Apply the transform to a point.
    pub fn apply(&self, p: [f32; 3]) -> [f32; 3] {
        let m = &self.0;
        [
            p[0] * m[0] + p[1] * m[3] + p[2] * m[6] + m[9],
            p[0] * m[1] + p[1] * m[4] + p[2] * m[7] + m[10],
            p[0] * m[2] + p[1] * m[5] + p[2] * m[8] + m[11],
        ]
    }
}

/// One object's geometry as written in a model part.
///
/// `paint[i]` is the raw `paint_color` attribute of `faces[i]` (still encoded;
/// [`super::paint::decode`] turns it into a subdivision tree). `paint` is
/// always the same length as `faces`.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct RawMesh {
    pub vertices: Vec<[f32; 3]>,
    pub faces: Vec<[u32; 3]>,
    pub paint: Vec<Option<String>>,
}

/// A `<object>` of a model part: inline geometry, components, or both.
#[derive(Debug, Clone, PartialEq)]
pub struct SceneObject {
    pub id: u32,
    pub name: Option<String>,
    /// Slicer-specific `extruder` metadata when the model part carries it.
    /// `Metadata/model_settings.config` is the authoritative source for
    /// Orca/Bambu files (worked on by the plate reader, not here).
    pub extruder: Option<u8>,
    pub mesh: Option<RawMesh>,
    pub components: Vec<Component>,
}

/// A `<component>` reference inside a [`SceneObject`].
#[derive(Debug, Clone, PartialEq)]
pub struct Component {
    pub object_id: u32,
    /// Package-relative name of the part holding the object, or `None` when
    /// the component references an object in the same part.
    pub path: Option<String>,
    pub transform: Transform,
}

/// One `<item>` of the main part's `<build>`.
#[derive(Debug, Clone, PartialEq)]
pub struct BuildItem {
    pub object_id: u32,
    pub transform: Transform,
}

/// Everything the reader needs from a package's model parts.
#[derive(Debug, Clone, PartialEq)]
pub struct Scene {
    /// `model@unit` of the main part, as written in the file.
    pub unit: String,
    /// Package part name → object id → object. Contains the main part plus
    /// every part reachable through component `p:path` references.
    pub parts: HashMap<String, HashMap<u32, SceneObject>>,
    /// Build items of the main part, in file order.
    pub build: Vec<BuildItem>,
}

/// Read the scene from an opened package.
///
/// Every referenced part is parsed once; reference cycles between parts are
/// tolerated. Coordinates and transform translations are returned in
/// millimetres. An unreadable main part, a component that names a missing
/// part, and malformed model XML are errors.
pub fn read_scene(pkg: &mut Package) -> Result<Scene, String> {
    read_scene_with_budget(pkg, GeometryBudget::new(MAX_EDITOR_FACES * 2))
}

struct ModelReadBudget {
    model_bytes: u64,
    geometry: GeometryBudget,
    paint_bytes: usize,
    object_nodes: usize,
}

impl ModelReadBudget {
    fn new(geometry: GeometryBudget) -> Self {
        Self {
            model_bytes: super::opc::MAX_ENTRY_BYTES,
            paint_bytes: geometry.faces * 4,
            geometry,
            object_nodes: MAX_EDITOR_FACES,
        }
    }

    fn read_part(&mut self, pkg: &mut Package, part: &str) -> Result<ParsedPart, String> {
        let bytes = pkg
            .read_at_most(part, self.model_bytes)
            .map_err(|e| format!("model part {part:?}: {e}"))?;
        self.model_bytes -= bytes.len() as u64;
        parse_part(&bytes, part, self)
    }

    fn claim_node(&mut self) -> Result<(), String> {
        self.object_nodes = self
            .object_nodes
            .checked_sub(1)
            .ok_or("3MF exceeds the model object and instance budget")?;
        Ok(())
    }
}

fn read_scene_with_budget(pkg: &mut Package, geometry: GeometryBudget) -> Result<Scene, String> {
    let main_part = pkg.model_part();

    let mut parts = HashMap::new();
    let mut build = Vec::new();
    let mut unit = "millimeter".to_string();
    let mut pending = VecDeque::new();
    let mut queued = HashSet::new();
    queued.insert(main_part.clone());
    pending.push_back(main_part.clone());
    let mut budget = ModelReadBudget::new(geometry);

    while let Some(name) = pending.pop_front() {
        let parsed = budget.read_part(pkg, &name)?;

        if name == main_part {
            unit = parsed.unit;
            build = parsed.build;
        }

        // Parts referenced by this part's components are read in a later pass;
        // `queued` keeps the same part from being read twice and breaks cycles.
        for object in parsed.objects.values() {
            for component in &object.components {
                if let Some(path) = &component.path {
                    if queued.insert(path.clone()) {
                        pending.push_back(path.clone());
                    }
                }
            }
        }

        parts.insert(name, parsed.objects);
    }

    Ok(Scene { unit, parts, build })
}

/// Parse only the parts reachable from the requested build items.
pub fn read_selected_scene(pkg: &mut Package, indices: &[usize]) -> Result<Scene, String> {
    let main = pkg.model_part();
    let mut budget = ModelReadBudget::new(GeometryBudget::new(MAX_EDITOR_FACES * 2));
    let parsed = budget.read_part(pkg, &main)?;
    let build = indices
        .iter()
        .map(|&i| {
            parsed
                .build
                .get(i)
                .cloned()
                .ok_or("Build item no longer exists")
        })
        .collect::<Result<Vec<_>, _>>()?;
    let mut queue: VecDeque<_> = build
        .iter()
        .map(|item| (main.clone(), item.object_id))
        .collect();
    let mut visited = HashSet::new();
    let mut parts = HashMap::from([(main, parsed.objects)]);
    while let Some((part, id)) = queue.pop_front() {
        if !visited.insert((part.clone(), id)) {
            continue;
        }
        if !parts.contains_key(&part) {
            let parsed = budget.read_part(pkg, &part)?;
            parts.insert(part.clone(), parsed.objects);
        }
        let object = parts[&part]
            .get(&id)
            .ok_or_else(|| format!("model part {part:?} has no object {id}"))?;
        queue.extend(object.components.iter().map(|child| {
            (
                child.path.clone().unwrap_or_else(|| part.clone()),
                child.object_id,
            )
        }));
    }
    Ok(Scene {
        unit: parsed.unit,
        parts,
        build,
    })
}

/// What one model part yields.
struct ParsedPart {
    unit: String,
    objects: HashMap<u32, SceneObject>,
    build: Vec<BuildItem>,
}

/// Parse one model part.
///
/// Each part carries its own `unit`; a missing unit is the 3MF default,
/// millimetre. A part without a complete `<model>` element (garbage, or XML
/// truncated before the close tag) is an error.
fn parse_part(xml: &[u8], part: &str, budget: &mut ModelReadBudget) -> Result<ParsedPart, String> {
    let mut parser = PartParser {
        part,
        budget,
        unit: "millimeter".to_string(),
        scale: 1.0,
        objects: HashMap::new(),
        build: Vec::new(),
        current: None,
        in_mesh: false,
        in_vertices: false,
        in_triangles: false,
        in_components: false,
        in_build: false,
        model_closed: false,
    };
    let mut reader = Reader::from_reader(xml);
    loop {
        match reader.read_event() {
            Ok(Event::Start(e)) => parser.start(&e, false)?,
            Ok(Event::Empty(e)) => parser.start(&e, true)?,
            Ok(Event::End(e)) => parser.end(&e),
            Ok(Event::Eof) => break,
            Ok(_) => {}
            Err(e) => return Err(format!("{part}: malformed model XML: {e}")),
        }
    }
    if !parser.model_closed {
        return Err(format!("{part}: model XML has no complete <model> element"));
    }
    // A malformed document that nests elements illegally may leave an object
    // open without a mismatched end tag for quick-xml to catch; keep it rather
    // than silently dropping geometry.
    if let Some(object) = parser.current.take() {
        parser.objects.insert(object.id, object);
    }
    Ok(ParsedPart {
        unit: parser.unit,
        objects: parser.objects,
        build: parser.build,
    })
}

struct PartParser<'a> {
    part: &'a str,
    budget: &'a mut ModelReadBudget,
    unit: String,
    scale: f32,
    objects: HashMap<u32, SceneObject>,
    build: Vec<BuildItem>,
    current: Option<SceneObject>,
    in_mesh: bool,
    in_vertices: bool,
    in_triangles: bool,
    in_components: bool,
    in_build: bool,
    model_closed: bool,
}

impl PartParser<'_> {
    fn start(&mut self, e: &BytesStart, self_closing: bool) -> Result<(), String> {
        match e.local_name().as_ref() {
            b"model" => {
                if let Some(unit) = self.optional_text(e, b"unit")? {
                    let unit = unit.trim();
                    if !unit.is_empty() {
                        self.unit = unit.to_string();
                        self.scale = unit_scale(unit);
                    }
                }
                if self_closing {
                    self.model_closed = true;
                }
            }
            b"object" => {
                let Some(id) = self.optional_id(e, b"id")? else {
                    return Ok(());
                };
                self.budget.claim_node()?;
                let name = self
                    .optional_text(e, b"name")?
                    .map(|name| name.trim().to_string())
                    .filter(|name| !name.is_empty());
                let object = SceneObject {
                    id,
                    name,
                    extruder: None,
                    mesh: None,
                    components: Vec::new(),
                };
                if self_closing {
                    self.objects.insert(id, object);
                } else {
                    self.current = Some(object);
                }
            }
            b"mesh" => {
                if let Some(object) = self.current.as_mut() {
                    object.mesh = Some(RawMesh::default());
                    self.in_mesh = !self_closing;
                }
            }
            b"vertices" => {
                if !self_closing {
                    self.in_vertices = true;
                }
            }
            b"vertex" => {
                if self.in_mesh && self.in_vertices {
                    self.budget.geometry.claim(1, 0)?;
                    let position = self.vertex(e)?;
                    if let Some(mesh) = self.current.as_mut().and_then(|o| o.mesh.as_mut()) {
                        mesh.vertices.push(position);
                    }
                }
            }
            b"triangles" => {
                if !self_closing {
                    self.in_triangles = true;
                }
            }
            b"triangle" => {
                if self.in_mesh && self.in_triangles {
                    self.budget.geometry.claim(0, 1)?;
                    let (face, paint) = self.triangle(e)?;
                    let part = self.part;
                    if let Some(mesh) = self.current.as_mut().and_then(|o| o.mesh.as_mut()) {
                        for vertex in face {
                            if vertex as usize >= mesh.vertices.len() {
                                return Err(format!(
                                    "{part}: <triangle> references vertex {vertex}, \
                                     but the mesh has {} vertices",
                                    mesh.vertices.len()
                                ));
                            }
                        }
                        mesh.faces.push(face);
                        mesh.paint.push(paint);
                    }
                }
            }
            b"components" => {
                if !self_closing {
                    self.in_components = true;
                }
            }
            b"component" => {
                if self.in_components {
                    self.budget.claim_node()?;
                    if let Some(component) = self.component(e)? {
                        if let Some(object) = self.current.as_mut() {
                            object.components.push(component);
                        }
                    }
                }
            }
            b"build" => {
                if !self_closing {
                    self.in_build = true;
                }
            }
            b"item" => {
                if self.in_build {
                    self.budget.claim_node()?;
                    if let Some(item) = self.build_item(e)? {
                        self.build.push(item);
                    }
                }
            }
            b"metadata" => {
                if self.current.is_some() && !self.in_mesh {
                    self.object_metadata(e)?;
                }
            }
            _ => {}
        }
        Ok(())
    }

    fn end(&mut self, e: &BytesEnd) {
        match e.local_name().as_ref() {
            b"model" => self.model_closed = true,
            b"object" => {
                if let Some(object) = self.current.take() {
                    self.objects.insert(object.id, object);
                }
            }
            b"mesh" => self.in_mesh = false,
            b"vertices" => self.in_vertices = false,
            b"triangles" => self.in_triangles = false,
            b"components" => self.in_components = false,
            b"build" => self.in_build = false,
            _ => {}
        }
    }

    /// The raw text of the first attribute with the given local name.
    fn optional_text(&self, e: &BytesStart, local: &[u8]) -> Result<Option<String>, String> {
        for attribute in e.attributes() {
            let attribute = attribute.map_err(|err| {
                format!(
                    "{}: malformed attribute in <{}>: {err}",
                    self.part,
                    element_name(e)
                )
            })?;
            if attribute.key.local_name().as_ref() == local {
                let value = attribute.unescape_value().map_err(|err| {
                    format!(
                        "{}: malformed value in <{}>: {err}",
                        self.part,
                        element_name(e)
                    )
                })?;
                return Ok(Some(value.into_owned()));
            }
        }
        Ok(None)
    }

    /// An object reference: missing and unparsable ids both mean "no object",
    /// and the element is skipped rather than failing the whole import.
    fn optional_id(&self, e: &BytesStart, local: &[u8]) -> Result<Option<u32>, String> {
        Ok(self
            .optional_text(e, local)?
            .and_then(|id| id.trim().parse::<u32>().ok()))
    }

    fn vertex(&self, e: &BytesStart) -> Result<[f32; 3], String> {
        let mut coordinates = [None; 3];
        for attribute in e.attributes() {
            let attribute = attribute
                .map_err(|err| format!("{}: malformed attribute in <vertex>: {err}", self.part))?;
            match attribute.key.local_name().as_ref() {
                b"x" => coordinates[0] = Some(self.f32_attribute(&attribute, "vertex", "x")?),
                b"y" => coordinates[1] = Some(self.f32_attribute(&attribute, "vertex", "y")?),
                b"z" => coordinates[2] = Some(self.f32_attribute(&attribute, "vertex", "z")?),
                _ => {}
            }
        }
        let [Some(x), Some(y), Some(z)] = coordinates else {
            return Err(format!("{}: <vertex> is missing x, y or z", self.part));
        };
        Ok([x * self.scale, y * self.scale, z * self.scale])
    }

    fn triangle(&mut self, e: &BytesStart) -> Result<([u32; 3], Option<String>), String> {
        let mut indices = [None; 3];
        let mut paint = None;
        for attribute in e.attributes() {
            let attribute = attribute.map_err(|err| {
                format!("{}: malformed attribute in <triangle>: {err}", self.part)
            })?;
            match attribute.key.local_name().as_ref() {
                b"v1" => indices[0] = Some(self.u32_attribute(&attribute, "triangle", "v1")?),
                b"v2" => indices[1] = Some(self.u32_attribute(&attribute, "triangle", "v2")?),
                b"v3" => indices[2] = Some(self.u32_attribute(&attribute, "triangle", "v3")?),
                b"paint_color" => {
                    // Count the XML attribute before unescaping or copying it
                    // so parsed parts cannot retain unchecked paint strings.
                    self.budget.paint_bytes = self
                        .budget
                        .paint_bytes
                        .checked_sub(attribute.value.len())
                        .ok_or("3MF exceeds the cumulative encoded paint budget")?;
                    let value = attribute.unescape_value().map_err(|err| {
                        format!("{}: malformed paint_color in <triangle>: {err}", self.part)
                    })?;
                    let value = value.trim();
                    if !value.is_empty() {
                        paint = Some(value.to_string());
                    }
                }
                _ => {}
            }
        }
        let [Some(v1), Some(v2), Some(v3)] = indices else {
            return Err(format!("{}: <triangle> is missing v1, v2 or v3", self.part));
        };
        Ok(([v1, v2, v3], paint))
    }

    fn component(&self, e: &BytesStart) -> Result<Option<Component>, String> {
        let Some(object_id) = self.optional_id(e, b"objectid")? else {
            return Ok(None);
        };
        let path = self
            .optional_text(e, b"path")?
            .map(|path| part_key(&path))
            .filter(|path| !path.is_empty());
        Ok(Some(Component {
            object_id,
            path,
            transform: self.transform(e)?,
        }))
    }

    fn build_item(&self, e: &BytesStart) -> Result<Option<BuildItem>, String> {
        let Some(object_id) = self.optional_id(e, b"objectid")? else {
            return Ok(None);
        };
        Ok(Some(BuildItem {
            object_id,
            transform: self.transform(e)?,
        }))
    }

    fn transform(&self, e: &BytesStart) -> Result<Transform, String> {
        let Some(text) = self.optional_text(e, b"transform")? else {
            return Ok(Transform::identity());
        };
        parse_transform(&text, self.part, self.scale)
    }

    /// `<metadata name="extruder" value="n"/>` on an object is the only model
    /// metadata the reader consumes; everything else is ignored.
    fn object_metadata(&mut self, e: &BytesStart) -> Result<(), String> {
        let name = self.optional_text(e, b"name")?;
        let value = self.optional_text(e, b"value")?;
        let (Some(name), Some(value)) = (name, value) else {
            return Ok(());
        };
        if name.trim().eq_ignore_ascii_case("extruder") {
            let extruder = value.trim().parse::<u8>().ok();
            if let (Some(extruder), Some(object)) = (extruder, self.current.as_mut()) {
                object.extruder = Some(extruder);
            }
        }
        Ok(())
    }

    fn f32_attribute(
        &self,
        attribute: &quick_xml::events::attributes::Attribute,
        element: &str,
        name: &str,
    ) -> Result<f32, String> {
        let raw = attribute.unescape_value().map_err(|err| {
            format!(
                "{}: <{element}> attribute {name} is not valid text: {err}",
                self.part
            )
        })?;
        let number = raw.trim().parse::<f32>().map_err(|_| {
            format!(
                "{}: <{element}> {name}={:?} is not a number",
                self.part,
                raw.as_ref()
            )
        })?;
        if !number.is_finite() {
            return Err(format!(
                "{}: <{element}> {name}={raw:?} is not a finite number",
                self.part
            ));
        }
        Ok(number)
    }

    fn u32_attribute(
        &self,
        attribute: &quick_xml::events::attributes::Attribute,
        element: &str,
        name: &str,
    ) -> Result<u32, String> {
        let raw = attribute.unescape_value().map_err(|err| {
            format!(
                "{}: <{element}> attribute {name} is not valid text: {err}",
                self.part
            )
        })?;
        raw.trim().parse::<u32>().map_err(|_| {
            format!(
                "{}: <{element}> {name}={:?} is not a vertex index",
                self.part,
                raw.as_ref()
            )
        })
    }
}

/// Parse a 3MF `transform` attribute into millimetre space.
///
/// Only the translation is unit-dependent: the linear part is unitless, but a
/// translation written in inches has to become millimetres before it can be
/// applied to converted vertices.
fn parse_transform(raw: &str, part: &str, scale: f32) -> Result<Transform, String> {
    let mut values = [0.0f32; 12];
    let mut count = 0usize;
    for token in raw.split_whitespace() {
        if count == values.len() {
            return Err(format!(
                "{part}: transform has more than {} numbers",
                values.len()
            ));
        }
        let value = token
            .parse::<f32>()
            .map_err(|_| format!("{part}: transform value {token:?} is not a number"))?;
        if !value.is_finite() {
            return Err(format!("{part}: transform value {token:?} is not finite"));
        }
        values[count] = value;
        count += 1;
    }
    if count != values.len() {
        return Err(format!(
            "{part}: transform needs {} numbers, got {count}",
            values.len()
        ));
    }
    if scale != 1.0 {
        values[9] *= scale;
        values[10] *= scale;
        values[11] *= scale;
    }
    Ok(Transform(values))
}

/// Millimetres per file unit. Unknown units (and the default, millimetre) are
/// read as millimetres.
fn unit_scale(unit: &str) -> f32 {
    match unit.trim().to_ascii_lowercase().as_str() {
        "inch" => 25.4,
        "meter" => 1000.0,
        "centimeter" => 10.0,
        "micron" => 0.001,
        _ => 1.0,
    }
}

/// The local name of a start tag, for error messages only.
fn element_name(e: &BytesStart) -> String {
    String::from_utf8_lossy(e.local_name().as_ref()).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::threemf::opc::Package;
    use std::io::Write;
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicU64, Ordering};

    #[test]
    fn a_selected_plate_reads_only_its_world_positioned_components() {
        let main = r#"<model xmlns:p="http://schemas.microsoft.com/3dmanufacturing/production/2015/06">
          <resources>
            <object id="1"><components><component objectid="5" p:path="/3D/Objects/ok.model"/></components></object>
            <object id="2"><components><component objectid="6" p:path="/3D/Objects/bad.model"/></components></object>
          </resources>
          <build><item objectid="1" transform="1 0 0 0 1 0 0 0 1 250 0 0"/><item objectid="2"/></build>
        </model>"#;
        let child = r#"<model><resources><object id="5"><mesh><vertices>
          <vertex x="0" y="0" z="0"/><vertex x="1" y="0" z="0"/><vertex x="0" y="1" z="0"/>
          </vertices><triangles><triangle v1="0" v2="1" v3="2"/></triangles></mesh></object></resources></model>"#;
        let fixture = Fixture::new(&[
            ("3D/3dmodel.model", main),
            ("3D/Objects/ok.model", child),
            ("3D/Objects/bad.model", "not XML"),
        ]);
        let mut package = Package::open(fixture.path()).unwrap();
        let selected = read_selected_scene(&mut package, &[0]).unwrap();
        assert_eq!(selected.build[0].transform.0[9], 250.0);
        assert!(selected.parts.contains_key("3D/Objects/ok.model"));
        assert!(!selected.parts.contains_key("3D/Objects/bad.model"));
        assert!(read_selected_scene(&mut package, &[1]).is_err());
    }

    /// Temporary synthetic 3MF package, removed when the fixture is dropped.
    struct Fixture {
        path: PathBuf,
    }

    impl Fixture {
        fn new(entries: &[(&str, &str)]) -> Self {
            static NEXT: AtomicU64 = AtomicU64::new(0);
            let serial = NEXT.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "cym-threemf-model-{}-{serial}.3mf",
                std::process::id()
            ));
            let file = std::fs::File::create(&path).expect("create fixture");
            let mut zip = zip::ZipWriter::new(file);
            for (name, body) in entries {
                zip.start_file(*name, zip::write::SimpleFileOptions::default())
                    .expect("start fixture entry");
                zip.write_all(body.as_bytes()).expect("write fixture entry");
            }
            zip.finish().expect("finish fixture");
            Self { path }
        }

        fn path(&self) -> &Path {
            &self.path
        }

        fn open(&self) -> Package {
            Package::open(&self.path).expect("fixture is a readable package")
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.path);
        }
    }

    /// Package relationships. The thumbnail comes first so the reader has to
    /// filter on the relationship type instead of taking the first target.
    const RELS: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
 <Relationship Target="/Metadata/plate_1.png" Id="rel-2" Type="http://schemas.openxmlformats.org/package/2006/relationships/metadata/thumbnail"/>
 <Relationship Target="/3D/3dmodel.model" Id="rel-1" Type="http://schemas.microsoft.com/3dmanufacturing/2013/01/3dmodel"/>
</Relationships>"#;

    fn model(unit: &str, resources: &str, build: &str) -> String {
        format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<model unit="{unit}" xml:lang="en-US" xmlns="http://schemas.microsoft.com/3dmanufacturing/core/2015/02" xmlns:p="http://schemas.microsoft.com/3dmanufacturing/production/2015/06" requiredextensions="p">
 <metadata name="Application">Snapmaker_Orca-2.4.1</metadata>
 <resources>{resources}</resources>
 <build>{build}</build>
</model>"#
        )
    }

    /// A three-vertex triangle in the default millimetre unit.
    fn triangle_part() -> String {
        model(
            "millimeter",
            r#"<object id="1" type="model"><mesh><vertices>
                 <vertex x="0" y="0" z="0"/>
                 <vertex x="1" y="0" z="0"/>
                 <vertex x="0" y="1" z="0"/>
               </vertices><triangles><triangle v1="0" v2="1" v3="2"/></triangles></mesh></object>"#,
            "",
        )
    }

    #[test]
    fn reads_an_inline_single_object_model() {
        let xml = model(
            "millimeter",
            r#"<object id="1" name="Cube" p:UUID="00000001-0000-0000-0000-000000000001" type="model">
                 <metadata name="extruder" value="2"/>
                 <mesh>
                   <vertices>
                     <vertex x="0" y="0" z="0"/>
                     <vertex x="10" y="0" z="0"/>
                     <vertex x="10" y="10" z="0"/>
                     <vertex x="0" y="10" z="0"/>
                     <vertex x="0" y="0" z="10"/>
                     <vertex x="10" y="0" z="10"/>
                     <vertex x="10" y="10" z="10"/>
                     <vertex x="0" y="10" z="10"/>
                   </vertices>
                   <triangles>
                     <triangle v1="0" v2="1" v3="2"/>
                     <triangle v1="2" v2="3" v3="0" paint_color="1C"/>
                   </triangles>
                 </mesh>
               </object>"#,
            r#"<item objectid="1" transform="1 0 0 0 1 0 0 0 1 0 0 0"/>"#,
        );
        let fixture = Fixture::new(&[("3D/3dmodel.model", xml.as_str())]);

        let mut pkg = fixture.open();
        // Without `_rels/.rels` the reader falls back to the conventional path.
        assert_eq!(pkg.model_part(), "3D/3dmodel.model");
        assert!(pkg.part_names().iter().any(|n| n == "3D/3dmodel.model"));

        let scene = read_scene(&mut pkg).expect("inline model parses");
        assert_eq!(scene.unit, "millimeter");
        assert_eq!(scene.parts.len(), 1);
        let main = &scene.parts["3D/3dmodel.model"];
        assert_eq!(main.len(), 1);
        let obj = &main[&1];
        assert_eq!(obj.id, 1);
        assert_eq!(obj.name.as_deref(), Some("Cube"));
        assert_eq!(obj.extruder, Some(2));
        let mesh = obj.mesh.as_ref().expect("inline mesh");
        assert_eq!(mesh.vertices.len(), 8);
        assert_eq!(mesh.vertices[6], [10.0, 10.0, 10.0]);
        assert_eq!(mesh.faces, vec![[0, 1, 2], [2, 3, 0]]);
        assert_eq!(mesh.paint, vec![None, Some("1C".to_string())]);
        assert!(obj.components.is_empty());
        assert_eq!(scene.build.len(), 1);
        assert_eq!(scene.build[0].object_id, 1);
        assert_eq!(
            scene.build[0].transform.apply([0.0, 0.0, 0.0]),
            [0.0, 0.0, 0.0]
        );
    }

    #[test]
    fn follows_rels_into_an_external_object_part() {
        let main = model(
            "millimeter",
            r#"<object id="2" type="model"><components>
                 <component p:path="/3D/Objects/part_1.model" objectid="1" transform="1 0 0 0 1 0 0 0 1 0 0 0"/>
               </components></object>"#,
            r#"<item objectid="2" transform="1 0 0 0 1 0 0 0 1 5 0 0"/>"#,
        );
        let part = triangle_part();
        let fixture = Fixture::new(&[
            ("_rels/.rels", RELS),
            ("3D/3dmodel.model", main.as_str()),
            ("3D/Objects/part_1.model", part.as_str()),
        ]);

        let scene = read_scene(&mut fixture.open()).expect("component reference resolves");
        assert_eq!(scene.parts.len(), 2);
        assert_eq!(scene.unit, "millimeter");

        let wrapper = &scene.parts["3D/3dmodel.model"][&2];
        assert!(wrapper.mesh.is_none());
        assert_eq!(wrapper.components.len(), 1);
        assert_eq!(wrapper.components[0].object_id, 1);
        assert_eq!(
            wrapper.components[0].path.as_deref(),
            Some("3D/Objects/part_1.model")
        );

        let external = &scene.parts["3D/Objects/part_1.model"];
        let mesh = external[&1].mesh.as_ref().expect("external mesh");
        assert_eq!(
            mesh.vertices,
            vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]]
        );
        assert_eq!(mesh.faces, vec![[0, 1, 2]]);
        assert_eq!(mesh.paint, vec![None]);
        assert_eq!(
            scene.build[0].transform.apply([0.0, 0.0, 0.0]),
            [5.0, 0.0, 0.0]
        );
    }

    #[test]
    fn composes_component_and_build_transforms() {
        let main = model(
            "millimeter",
            r#"<object id="2" type="model"><components>
                 <component p:path="/3D/Objects/part_1.model" objectid="1" p:transform="1 0 0 0 1 0 0 0 1 0 5 0"/>
               </components></object>"#,
            r#"<item objectid="2" transform="1 0 0 0 1 0 0 0 1 10 0 0"/>"#,
        );
        let part = triangle_part();
        let fixture = Fixture::new(&[
            ("_rels/.rels", RELS),
            ("3D/3dmodel.model", main.as_str()),
            ("3D/Objects/part_1.model", part.as_str()),
        ]);
        let scene = read_scene(&mut fixture.open()).expect("transforms parse");

        let component = &scene.parts["3D/3dmodel.model"][&2].components[0];
        let build = &scene.build[0];
        // The component moves the vertex up 5, then the build item moves it
        // right 10: build is the outer transform.
        assert_eq!(
            build
                .transform
                .compose(component.transform)
                .apply([0.0, 0.0, 0.0]),
            [10.0, 5.0, 0.0]
        );

        // A scale on the build item also scales the component's translation,
        // which the opposite composition order would not do.
        let scaled_build = Transform([3.0, 0.0, 0.0, 0.0, 3.0, 0.0, 0.0, 0.0, 3.0, 10.0, 0.0, 0.0]);
        assert_eq!(
            scaled_build
                .compose(component.transform)
                .apply([1.0, 0.0, 0.0]),
            [13.0, 15.0, 0.0]
        );
    }

    #[test]
    fn scales_non_millimeter_units_to_millimeter() {
        let xml = model(
            "inch",
            r#"<object id="1" type="model"><mesh><vertices>
                 <vertex x="1" y="2" z="0.5"/>
                 <vertex x="-1" y="0" z="0"/>
                 <vertex x="0" y="0" z="0"/>
               </vertices><triangles/></mesh></object>"#,
            r#"<item objectid="1" transform="1 0 0 0 1 0 0 0 1 1 0 0"/>"#,
        );
        let fixture = Fixture::new(&[("3D/3dmodel.model", xml.as_str())]);
        let scene = read_scene(&mut fixture.open()).expect("inch model parses");

        assert_eq!(scene.unit, "inch");
        let mesh = scene.parts["3D/3dmodel.model"][&1]
            .mesh
            .as_ref()
            .expect("mesh");
        assert_eq!(mesh.vertices[0], [25.4, 50.8, 12.7]);
        assert_eq!(mesh.vertices[1], [-25.4, 0.0, 0.0]);
        // The transform translation is in millimetres as well, so it can be
        // applied to the converted vertices.
        assert_eq!(
            scene.build[0].transform.apply([0.0, 0.0, 0.0]),
            [25.4, 0.0, 0.0]
        );
    }

    #[test]
    fn an_entry_whose_declared_size_exceeds_the_read_limit_is_an_error() {
        let xml = model("millimeter", "", "");
        let fixture = Fixture::new(&[("3D/3dmodel.model", xml.as_str())]);

        let mut pkg =
            Package::open_with_limit(fixture.path(), 8).expect("opening reads no entries");
        let err = pkg
            .read("3D/3dmodel.model")
            .expect_err("the size guard must reject the entry before reading it");
        assert!(err.contains("limit"), "unexpected error: {err}");

        // The same file is readable under the production limit.
        let mut pkg = fixture.open();
        assert!(pkg.read("3D/3dmodel.model").is_ok());
    }

    #[test]
    fn the_main_model_part_comes_from_the_package_relationships() {
        let rels = r#"<?xml version="1.0" encoding="UTF-8"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
 <Relationship Target="/3D/custom.model" Id="rel-1" Type="http://schemas.microsoft.com/3dmanufacturing/2013/01/3dmodel"/>
</Relationships>"#;
        let xml = model("millimeter", "", "");
        let fixture = Fixture::new(&[("_rels/.rels", rels), ("3D/custom.model", xml.as_str())]);

        let mut pkg = fixture.open();
        assert_eq!(pkg.model_part(), "3D/custom.model");
        let scene = read_scene(&mut pkg).expect("custom main part parses");
        assert!(scene.parts.contains_key("3D/custom.model"));
    }

    #[test]
    fn nested_component_references_across_parts_resolve_and_cycles_terminate() {
        let main = model(
            "millimeter",
            r#"<object id="2" type="model"><components>
                 <component p:path="/3D/Objects/a.model" objectid="1"/>
               </components></object>"#,
            r#"<item objectid="2"/>"#,
        );
        let a = model(
            "millimeter",
            r#"<object id="1" type="model"><components>
                 <component objectid="1"/>
                 <component p:path="/3D/Objects/b.model" objectid="5"/>
                 <component p:path="/3D/3dmodel.model" objectid="2"/>
               </components></object>"#,
            "",
        );
        let b = model(
            "millimeter",
            r#"<object id="5" type="model"><mesh><vertices>
                 <vertex x="0" y="0" z="0"/>
                 <vertex x="2" y="0" z="0"/>
                 <vertex x="0" y="2" z="0"/>
               </vertices><triangles><triangle v1="0" v2="1" v3="2"/></triangles></mesh></object>"#,
            "",
        );
        let fixture = Fixture::new(&[
            ("_rels/.rels", RELS),
            ("3D/3dmodel.model", main.as_str()),
            ("3D/Objects/a.model", a.as_str()),
            ("3D/Objects/b.model", b.as_str()),
        ]);

        let scene = read_scene(&mut fixture.open()).expect("nested references resolve");
        assert_eq!(scene.parts.len(), 3);
        let paths: Vec<Option<&str>> = scene.parts["3D/Objects/a.model"][&1]
            .components
            .iter()
            .map(|c| c.path.as_deref())
            .collect();
        assert_eq!(
            paths,
            vec![None, Some("3D/Objects/b.model"), Some("3D/3dmodel.model")]
        );
        let b_mesh = scene.parts["3D/Objects/b.model"][&5]
            .mesh
            .as_ref()
            .expect("transitively referenced mesh");
        assert_eq!(b_mesh.faces, vec![[0, 1, 2]]);
        // A build item without a transform is the identity.
        assert_eq!(scene.build[0].transform, Transform::identity());
    }

    #[test]
    fn malformed_model_xml_is_an_error_not_a_panic() {
        let cases: Vec<(&str, String)> = vec![
            (
                "unclosed",
                "<model><resources><object id=\"1\" type=\"model\">".to_string(),
            ),
            ("not xml at all", "this is not a model".to_string()),
            (
                "bad coordinate",
                model(
                    "millimeter",
                    r#"<object id="1"><mesh><vertices><vertex x="oops" y="0" z="0"/></vertices></mesh></object>"#,
                    "",
                ),
            ),
            (
                "short transform",
                model(
                    "millimeter",
                    "",
                    r#"<item objectid="1" transform="1 2 3"/>"#,
                ),
            ),
            (
                "missing triangle index",
                model(
                    "millimeter",
                    r#"<object id="1"><mesh><vertices><vertex x="0" y="0" z="0"/></vertices><triangles><triangle v1="0"/></triangles></mesh></object>"#,
                    "",
                ),
            ),
            (
                "triangle index out of range",
                model(
                    "millimeter",
                    r#"<object id="1"><mesh><vertices><vertex x="0" y="0" z="0"/></vertices><triangles><triangle v1="0" v2="1" v3="2"/></triangles></mesh></object>"#,
                    "",
                ),
            ),
        ];

        for (label, xml) in cases {
            let fixture = Fixture::new(&[("3D/3dmodel.model", xml.as_str())]);
            let mut pkg = fixture.open();
            assert!(
                read_scene(&mut pkg).is_err(),
                "{label} must be reported as an error"
            );
        }
    }
}
