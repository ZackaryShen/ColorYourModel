//! Project-state codec shared by the 3MF writer, the 3MF reader's fallback
//! path and (in P2) the tab swap cache.
//!
//! Core parts and optional paint metadata:
//!
//! * `project.json` — the header: format version, app, unit, active plate and
//!   one [`PlateStateHeader`] per plate with the geometry fingerprint that
//!   decides whether the per-face state still applies.
//! * `plate_<i>.bin` — exact per-face state: labels, pre-quantisation RGBA,
//!   the manual-label high-water mark and the user-supplied region names.
//! * `history_<i>.bin` — the undo/redo stacks, budgeted separately from the
//!   in-memory history so a save can never be as large as the live process.
//! * `gradient_faces_<i>.bin` — authored ramps and the exact blend sample for
//!   each gradient-painted face. Undo and redo samples use the same ramp IDs.
//!
//! Geometry is never duplicated here: the 3MF mesh is authoritative, and the
//! [`Fingerprint`] is what ties the saved state to it.

use std::collections::HashMap;

use serde::Deserialize;

use crate::mesh::history::{
    GradientContext, GradientTool, History, OpKind, RawEntry, DEFAULT_BYTE_BUDGET,
    DEFAULT_MAX_DEPTH,
};
use crate::mesh::model::{GradientSample, MeshModel, DEFAULT_FACE_COLOR};
use crate::paint::gradient::{GradientHit, MAX_POINTS, MAX_RADIUS, MAX_STOPS};

/// Major version of the engineering parts.
///
/// The reader rejects any other major version rather than guessing at a layout
/// it does not know. Unknown *minor* fields are ignored by serde, which is how
/// a newer writer can add metadata without breaking an older reader.
pub const CYM_FORMAT_VERSION: u32 = 1;

/// Value of the header's `app` field. Informational only: the reader relies on
/// `format_version`, not on this string.
const PROJECT_APP: &str = "ColorYourModel";

/// The unit a project written before the `unit` field existed is assumed to
/// be in. The 3MF exporter always writes millimetres.
fn default_unit() -> String {
    "millimeter".to_string()
}

// ---------------------------------------------------------------------------
// Fingerprint
// ---------------------------------------------------------------------------

const FNV_OFFSET_BASIS: u64 = 0xcbf2_9ce4_8422_2325;
const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;

fn fnv1a_update(mut hash: u64, bytes: &[u8]) -> u64 {
    for &byte in bytes {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(FNV_PRIME);
    }
    hash
}

/// Canonicalise one coordinate the way the 3MF writer does, so the hash is of
/// the value a re-read of our own file will hold.
///
/// The writer emits `format!("{:.6}", c)`. Parsing that string back does not
/// always reproduce the original f32 bit pattern — measured on a real file,
/// 1.29% of coordinates change bits — so hashing raw in-memory bits would make
/// every re-opened project fail its own fingerprint. Formatting and parsing
/// here reproduces the exact value the reader will parse.
fn canonical_component(component: f32) -> f32 {
    format!("{:.6}", component)
        .parse::<f32>()
        // Unreachable: every f32 formats to a string `str::parse::<f32>`
        // accepts. The fallback keeps the function total rather than panicking
        // on a value that cannot occur.
        .unwrap_or(component)
}

/// Geometry identity of one plate.
///
/// The hash covers the canonicalised vertex components followed by every face
/// index. The counts are carried alongside rather than folded into the hash:
/// the reader compares all three fields, so a mismatch on any of them is a
/// mismatch.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, Deserialize)]
pub struct Fingerprint {
    pub vertex_count: u32,
    pub face_count: u32,
    #[serde(with = "fingerprint_hash")]
    pub hash: u64,
}

/// Wire form of [`Fingerprint::hash`]: the spec writes `"fnv1a64:<hex>"`.
/// Deserialisation also accepts a bare integer so a hand-written or older
/// header still parses.
mod fingerprint_hash {
    use serde::{Deserializer, Serializer};

    pub fn serialize<S: Serializer>(value: &u64, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&format!("fnv1a64:{value:016x}"))
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<u64, D::Error> {
        struct HashVisitor;

        impl<'de> serde::de::Visitor<'de> for HashVisitor {
            type Value = u64;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("an \"fnv1a64:<hex>\" string or a u64 hash")
            }

            fn visit_u64<E: serde::de::Error>(self, value: u64) -> Result<u64, E> {
                Ok(value)
            }

            fn visit_i64<E: serde::de::Error>(self, value: i64) -> Result<u64, E> {
                u64::try_from(value).map_err(|_| E::custom("fingerprint hash must not be negative"))
            }

            fn visit_str<E: serde::de::Error>(self, value: &str) -> Result<u64, E> {
                let hex = value.strip_prefix("fnv1a64:").unwrap_or(value);
                u64::from_str_radix(hex, 16)
                    .map_err(|_| E::custom("fingerprint hash is not valid hexadecimal"))
            }
        }

        deserializer.deserialize_any(HashVisitor)
    }
}

/// Hash a plate's geometry for the project header.
///
/// Passing the writer's `mesh.vertices` and `mesh.faces` directly is the
/// intended call: canonicalisation happens inside, so both sides of a
/// write/read round trip hash the same values.
pub fn fingerprint_of(vertices: &[[f32; 3]], faces: &[[u32; 3]]) -> Fingerprint {
    let mut hash = FNV_OFFSET_BASIS;
    for vertex in vertices {
        for &component in vertex {
            hash = fnv1a_update(hash, &canonical_component(component).to_le_bytes());
        }
    }
    for face in faces {
        for &index in face {
            hash = fnv1a_update(hash, &index.to_le_bytes());
        }
    }
    Fingerprint {
        vertex_count: vertices.len() as u32,
        face_count: faces.len() as u32,
        hash,
    }
}

// ---------------------------------------------------------------------------
// project.json
// ---------------------------------------------------------------------------

/// One plate's row in `project.json`.
#[derive(Debug, Clone, PartialEq, serde::Serialize, Deserialize)]
pub struct PlateStateHeader {
    pub plater_id: u32,
    pub name: String,
    pub object_ids: Vec<u32>,
    pub fingerprint: Fingerprint,
    #[serde(default)]
    pub history_truncated: bool,
    #[serde(default)]
    pub placement_offset: [f32; 3],
}

/// The parsed project header.
///
/// `unit` is what the 3MF declared at write time. It is persisted because a
/// project may legitimately be opened by a build whose current unit setting
/// differs; the reader keeps the file's units rather than its own.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct ProjectState {
    pub active_plate: u32,
    /// This writer accounted for provenance, even on plates with no brush strokes.
    #[serde(default)]
    pub paint_sources_complete: bool,
    pub plates: Vec<PlateStateHeader>,
    #[serde(default = "default_unit")]
    pub unit: String,
}

/// The actual on-disk shape. Kept separate from [`ProjectState`] because the
/// version and app fields are constants of the format, not state a caller may
/// set — and because a caller must never be able to write a file whose
/// `format_version` disagrees with its own encoder.
#[derive(serde::Serialize)]
struct ProjectFileJson<'a> {
    format_version: u32,
    app: &'static str,
    unit: &'a str,
    active_plate: u32,
    paint_sources_complete: bool,
    plates: &'a [PlateStateHeader],
}

/// Serialise `project.json`.
pub fn encode_project_json(state: &ProjectState) -> String {
    let wire = ProjectFileJson {
        format_version: CYM_FORMAT_VERSION,
        app: PROJECT_APP,
        unit: &state.unit,
        active_plate: state.active_plate,
        paint_sources_complete: state.paint_sources_complete,
        plates: &state.plates,
    };
    // No maps with non-string keys, no non-finite floats: this cannot fail.
    serde_json::to_string_pretty(&wire)
        .expect("the project header is a plain struct and always serialises")
}

/// Parse `project.json`.
///
/// A file from a different major version is an error, not a best-effort
/// partial read: the per-face parts are laid out by version, and guessing
/// would corrupt state silently. Unknown fields are ignored so a newer writer
/// can add metadata; a missing `unit` defaults to millimetres.
pub fn decode_project_json(text: &str) -> Result<ProjectState, String> {
    let value: serde_json::Value =
        serde_json::from_str(text).map_err(|e| format!("project.json: invalid JSON: {e}"))?;
    let version = value
        .get("format_version")
        .and_then(serde_json::Value::as_u64)
        .ok_or_else(|| "project.json: missing format_version".to_string())?;
    if version != u64::from(CYM_FORMAT_VERSION) {
        return Err(format!(
            "project.json: unsupported major format_version {version} (this build reads {CYM_FORMAT_VERSION})"
        ));
    }
    let state: ProjectState =
        serde_json::from_value(value).map_err(|e| format!("project.json: {e}"))?;
    if state.plates.iter().any(|plate| {
        plate
            .placement_offset
            .iter()
            .any(|coordinate| !coordinate.is_finite())
    }) {
        return Err("project.json: non-finite placement offset".into());
    }
    Ok(state)
}

// ---------------------------------------------------------------------------
// plate_<i>.bin
// ---------------------------------------------------------------------------

/// Magic at the start of every plate-state part. The trailing NUL pads the
/// documented `CYMST1\x01` signature to the eight bytes the layout reserves.
pub const PLATE_MAGIC: &[u8; 8] = b"CYMST1\x01\0";

/// Plausible upper bound on the region-name table: a plate names at most a
/// handful of regions per face, and the slack absorbs stale names (names are
/// deliberately not pruned when a region disappears). A larger table is a
/// forged count, not a file this editor wrote, and must be rejected before it
/// can drive a `HashMap` allocation.
const MAX_NAME_TABLE_PER_FACE: usize = 8;
const MAX_NAME_TABLE_SLACK: usize = 1024;

fn name_table_limit(face_count: usize) -> usize {
    face_count
        .saturating_mul(MAX_NAME_TABLE_PER_FACE)
        .saturating_add(MAX_NAME_TABLE_SLACK)
}

/// Per-face project state for one plate. The history lives in its own part and
/// is deliberately absent here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlateState {
    pub segment_labels: Vec<u32>,
    pub face_colors: Vec<[u8; 4]>,
    pub next_manual_label: u32,
    pub segment_names: HashMap<u32, String>,
}

/// Serialise `plate_<i>.bin`: magic, face count, labels, exact RGBA, the label
/// high-water mark and the region-name table.
pub fn encode_plate_state(mesh: &MeshModel) -> Vec<u8> {
    // A mesh cannot index past `u32::MAX` faces; clamping keeps the redundant
    // count field honest instead of silently wrapping.
    let face_count = mesh.faces.len().min(u32::MAX as usize);

    let mut out = Vec::with_capacity(8 + 4 + face_count * 8 + 4 + 4);
    out.extend_from_slice(PLATE_MAGIC);
    out.extend_from_slice(&(face_count as u32).to_le_bytes());

    // The per-face arrays are index-aligned with `faces`; a short buffer is a
    // mesh invariant violation, not a file-format case. Reading with `get`
    // keeps encoding total instead of panicking on the way out.
    for index in 0..face_count {
        let label = mesh.segment_labels.get(index).copied().unwrap_or(0);
        out.extend_from_slice(&label.to_le_bytes());
    }
    for index in 0..face_count {
        let color = mesh
            .face_colors
            .get(index)
            .copied()
            .unwrap_or(DEFAULT_FACE_COLOR);
        out.extend_from_slice(&color);
    }

    out.extend_from_slice(&mesh.next_manual_label.to_le_bytes());

    // HashMap iteration order is randomised; sorting by label keeps the part
    // byte-identical across saves of the same model.
    let mut names: Vec<(&u32, &String)> = mesh.segment_names.iter().collect();
    names.sort_by_key(|(label, _)| **label);
    out.extend_from_slice(&(names.len().min(u32::MAX as usize) as u32).to_le_bytes());
    for (label, name) in names {
        out.extend_from_slice(&label.to_le_bytes());
        let bytes = name.as_bytes();
        // Names are capped at 64 characters upstream, well under the u16
        // limit; the clamp is here so an out-of-band model cannot truncate a
        // multi-byte character mid-codepoint.
        let mut len = bytes.len().min(u16::MAX as usize);
        while len > 0 && !name.is_char_boundary(len) {
            len -= 1;
        }
        out.extend_from_slice(&(len as u16).to_le_bytes());
        out.extend_from_slice(&bytes[..len]);
    }
    out
}

/// Parse `plate_<i>.bin` and verify it belongs to a mesh with `face_count`
/// faces.
///
/// Every count is checked against the remaining byte length before a buffer
/// is allocated: a header claiming four billion faces on a twelve-byte body
/// must fail cheaply, not try to reserve 32 GB.
pub fn decode_plate_state(bytes: &[u8], face_count: usize) -> Result<PlateState, String> {
    if bytes.len() < PLATE_MAGIC.len() {
        return Err("plate state: truncated magic".to_string());
    }
    if &bytes[..PLATE_MAGIC.len()] != &PLATE_MAGIC[..] {
        return Err("plate state: bad magic".to_string());
    }

    let mut cursor = Cursor::new(&bytes[PLATE_MAGIC.len()..]);
    let stored_face_count = cursor.u32()? as usize;
    if stored_face_count != face_count {
        return Err(format!(
            "plate state: stored face_count {stored_face_count} does not match the mesh's {face_count}"
        ));
    }

    // Labels and RGBA are 8 bytes per face; the high-water mark and name count
    // add another 8. Check all of it before allocating either array.
    let fixed_bytes = (stored_face_count as u64)
        .checked_mul(8)
        .and_then(|n| n.checked_add(8))
        .ok_or_else(|| "plate state: size overflow".to_string())?;
    if (cursor.remaining() as u64) < fixed_bytes {
        return Err(format!(
            "plate state: size mismatch (declared {stored_face_count} faces, {} body bytes)",
            cursor.remaining()
        ));
    }

    let mut segment_labels = Vec::with_capacity(stored_face_count);
    for _ in 0..stored_face_count {
        segment_labels.push(cursor.u32()?);
    }

    let mut face_colors = Vec::with_capacity(stored_face_count);
    for _ in 0..stored_face_count {
        face_colors.push(
            cursor
                .take(4)?
                .try_into()
                .expect("Cursor::take returned exactly four bytes"),
        );
    }

    let next_manual_label = cursor.u32()?;
    let name_count = cursor.u32()? as usize;
    if !cursor.has_room_for(name_count, 6) {
        return Err("plate state: size mismatch in the region-name table".to_string());
    }
    // The byte check above only says the body could hold the table. Bound the
    // count itself: a name table much larger than the face count is forged.
    let name_limit = name_table_limit(stored_face_count);
    if name_count > name_limit {
        return Err(format!(
            "plate state: region-name table claims {name_count} names for \
             {stored_face_count} faces (limit {name_limit})"
        ));
    }
    let mut segment_names = HashMap::with_capacity(name_count.min(name_limit));
    for _ in 0..name_count {
        let label = cursor.u32()?;
        let len = cursor.u16()? as usize;
        let name_bytes = cursor.take(len)?;
        let name = std::str::from_utf8(name_bytes)
            .map_err(|_| "plate state: region name is not valid UTF-8".to_string())?;
        segment_names.insert(label, name.to_string());
    }

    if cursor.remaining() != 0 {
        return Err(format!(
            "plate state: size mismatch, {} trailing bytes",
            cursor.remaining()
        ));
    }

    Ok(PlateState {
        segment_labels,
        face_colors,
        next_manual_label,
        segment_names,
    })
}

/// Optional sidecar for the brush colour and its blended result per face.
const PAINT_SOURCE_MAGIC: &[u8; 8] = b"CYMPS1\x01\0";
pub type PaintSources = HashMap<u32, ([u8; 4], [u8; 4])>;

pub fn encode_paint_sources(mesh: &MeshModel) -> Vec<u8> {
    let mut entries: Vec<_> = mesh.paint_sources.iter().collect();
    entries.sort_unstable_by_key(|(&face, _)| face);
    let mut bytes = Vec::with_capacity(8 + entries.len() * 12);
    bytes.extend_from_slice(PAINT_SOURCE_MAGIC);
    for (&face, (source, painted)) in entries {
        bytes.extend_from_slice(&face.to_le_bytes());
        bytes.extend_from_slice(source);
        bytes.extend_from_slice(painted);
    }
    bytes
}

pub fn decode_paint_sources(bytes: &[u8], face_count: usize) -> Result<PaintSources, String> {
    let body = bytes
        .strip_prefix(PAINT_SOURCE_MAGIC)
        .ok_or("paint sources: invalid header")?;
    if body.len() % 12 != 0 || body.len() / 12 > face_count {
        return Err("paint sources: invalid record count".into());
    }
    let mut cursor = Cursor::new(body);
    let mut sources = HashMap::with_capacity(body.len() / 12);
    while cursor.remaining() != 0 {
        let face = cursor.u32()?;
        let source = cursor.take(4)?.try_into().unwrap();
        let painted = cursor.take(4)?.try_into().unwrap();
        if face as usize >= face_count || sources.insert(face, (source, painted)).is_some() {
            return Err("paint sources: invalid or duplicate face".into());
        }
    }
    Ok(sources)
}

/// Optional exact per-face gradient semantics for the current state.
const LEGACY_GRADIENT_MAGIC: &[u8; 8] = b"CYMGR1\x01\0";
const GRADIENT_MAGIC: &[u8; 8] = b"CYMGR1\x02\0";
const LEGACY_GRADIENT_RECORD_BYTES: usize = 4 + 4 + 4 + 2;
const GRADIENT_RECORD_BYTES: usize = LEGACY_GRADIENT_RECORD_BYTES + 4;

fn gradient_record_bytes(has_ramp_ids: bool) -> usize {
    if has_ramp_ids {
        GRADIENT_RECORD_BYTES
    } else {
        LEGACY_GRADIENT_RECORD_BYTES
    }
}

fn write_gradient(out: &mut Vec<u8>, (face, sample): (u32, GradientSample)) {
    out.extend_from_slice(&face.to_le_bytes());
    out.extend_from_slice(&sample.from);
    out.extend_from_slice(&sample.to);
    out.extend_from_slice(&sample.mix.to_le_bytes());
    out.extend_from_slice(&sample.ramp_id.to_le_bytes());
}

fn read_gradient(
    cursor: &mut Cursor<'_>,
    has_ramp_ids: bool,
    ramps: &[Vec<[u8; 4]>],
) -> Result<(u32, GradientSample), String> {
    let face = cursor.u32()?;
    let from = cursor.take(4)?.try_into().unwrap();
    let to = cursor.take(4)?.try_into().unwrap();
    let mix = cursor.u16()?;
    let ramp_id = if has_ramp_ids { cursor.u32()? } else { 0 };
    if ramp_id != 0 {
        let ramp = ramps
            .get((ramp_id - 1) as usize)
            .ok_or("gradients: unknown ramp")?;
        if !ramp.windows(2).any(|stops| stops == [from, to]) {
            return Err("gradients: sample colours disagree with its ramp".into());
        }
    }
    Ok((
        face,
        GradientSample {
            from,
            to,
            mix,
            ramp_id,
        },
    ))
}

pub fn encode_gradient_faces(mesh: &MeshModel) -> Vec<u8> {
    let mut entries: Vec<_> = mesh.gradient_faces.iter().map(|(&f, &s)| (f, s)).collect();
    entries.sort_unstable_by_key(|&(face, _)| face);
    let ramp_bytes: usize = mesh
        .gradient_ramps
        .iter()
        .map(|ramp| 1 + 4 * ramp.len())
        .sum();
    let mut bytes = Vec::with_capacity(
        GRADIENT_MAGIC.len() + 4 + ramp_bytes + entries.len() * GRADIENT_RECORD_BYTES,
    );
    bytes.extend_from_slice(GRADIENT_MAGIC);
    bytes.extend_from_slice(&(mesh.gradient_ramps.len() as u32).to_le_bytes());
    for ramp in &mesh.gradient_ramps {
        bytes.push(ramp.len() as u8);
        for stop in ramp {
            bytes.extend_from_slice(stop);
        }
    }
    for record in entries {
        write_gradient(&mut bytes, record);
    }
    bytes
}

pub fn decode_gradient_faces(
    bytes: &[u8],
    colors: &[[u8; 4]],
) -> Result<(HashMap<u32, GradientSample>, Vec<Vec<[u8; 4]>>), String> {
    let (body, has_ramp_ids) = if let Some(body) = bytes.strip_prefix(GRADIENT_MAGIC) {
        (body, true)
    } else {
        (
            bytes
                .strip_prefix(LEGACY_GRADIENT_MAGIC)
                .ok_or("gradients: invalid header")?,
            false,
        )
    };
    let mut cursor = Cursor::new(body);
    let mut ramps = Vec::new();
    if has_ramp_ids {
        let ramp_count = cursor.u32()? as usize;
        // Bound the table before allocating. Every ramp occupies at least
        // nine wire bytes and at most MAX_STOPS colours in memory.
        let max_ramp_bytes = std::mem::size_of::<Vec<[u8; 4]>>() + MAX_STOPS * 4;
        if !cursor.has_room_for(ramp_count, 1 + 2 * 4)
            || ramp_count > DEFAULT_BYTE_BUDGET / max_ramp_bytes
        {
            return Err("gradients: invalid ramp count".into());
        }
        ramps.reserve_exact(ramp_count);
        for _ in 0..ramp_count {
            let stops = cursor.u8()? as usize;
            if !(2..=MAX_STOPS).contains(&stops) {
                return Err("gradients: invalid colour stop count".into());
            }
            let ramp = cursor
                .take(stops * 4)?
                .chunks_exact(4)
                .map(|rgba| rgba.try_into().unwrap())
                .collect();
            ramps.push(ramp);
        }
    }
    let size = gradient_record_bytes(has_ramp_ids);
    let count = cursor.remaining() / size;
    if cursor.remaining() % size != 0 || count > colors.len() {
        return Err("gradients: invalid record count".into());
    }
    let mut gradients = HashMap::with_capacity(count);
    for _ in 0..count {
        let (face, sample) = read_gradient(&mut cursor, has_ramp_ids, &ramps)?;
        if colors.get(face as usize) != Some(&sample.rgba())
            || gradients.insert(face, sample).is_some()
        {
            return Err("gradients: invalid face, duplicate face or mismatched colour".into());
        }
    }
    Ok((gradients, ramps))
}

// ---------------------------------------------------------------------------
// history_<i>.bin
// ---------------------------------------------------------------------------

/// Ceiling on the persisted history, separate from the 256 MB in-memory
/// budget: a save may legitimately keep less history than the live process.
pub const MAX_PERSISTED_HISTORY_BYTES: usize = 64 * 1024 * 1024;

/// Smallest possible entry on the wire: kind, stroke flag, two count fields.
const MIN_ENTRY_BYTES: usize = 1 + 1 + 4 + 4;
/// Container overhead: the undo and redo stack counts.
const HISTORY_HEADER_BYTES: usize = 4 + 4;
const GRADIENT_CONTEXT_HEADER_BYTES: usize = 4 + 1 + 1 + 2;
const GRADIENT_HIT_BYTES: usize = 4 + 3 * 4;

/// Serialise `history_<i>.bin` under [`MAX_PERSISTED_HISTORY_BYTES`].
pub fn encode_history(history: &History) -> Vec<u8> {
    encode_history_report(history).0
}

/// Return the encoded history and whether some earlier history was lost.
pub fn encode_history_report(history: &History) -> (Vec<u8>, bool) {
    encode_history_with_budget_report(history, MAX_PERSISTED_HISTORY_BYTES)
}

/// Byte length of one entry exactly as [`write_entry`] emits it.
fn entry_wire_len(entry: &RawEntry) -> usize {
    let stroke = if entry.stroke_id.is_some() { 8 } else { 0 };
    // kind + flag + (stroke) + colour count + colours + label count + labels
    let gradients = if entry.gradients.is_empty() {
        0
    } else {
        4 + entry.gradients.len() * GRADIENT_RECORD_BYTES
    };
    let context = entry.gradient_context.as_ref().map_or(0, |context| {
        GRADIENT_CONTEXT_HEADER_BYTES
            + context.stops.len() * 4
            + context.points.len() * GRADIENT_HIT_BYTES
    });
    1 + 1 + stroke + 4 + entry.colors.len() * 8 + 4 + entry.labels.len() * 8 + gradients + context
}

fn write_entry(out: &mut Vec<u8>, entry: &RawEntry) {
    out.push(entry.kind.to_u8());
    // Bit 0 carries the stroke ID, bit 1 gradient samples, bit 2 the gesture,
    // and bit 3 the ramp ID in each sample. Earlier samples omit the ID.
    out.push(
        u8::from(entry.stroke_id.is_some())
            | (u8::from(!entry.gradients.is_empty()) << 1)
            | (u8::from(entry.gradient_context.is_some()) << 2)
            | (u8::from(!entry.gradients.is_empty()) << 3),
    );
    if let Some(stroke_id) = entry.stroke_id {
        out.extend_from_slice(&stroke_id.to_le_bytes());
    }
    out.extend_from_slice(&(entry.colors.len() as u32).to_le_bytes());
    for (face, rgba) in &entry.colors {
        out.extend_from_slice(&face.to_le_bytes());
        out.extend_from_slice(rgba);
    }
    out.extend_from_slice(&(entry.labels.len() as u32).to_le_bytes());
    for (face, label) in &entry.labels {
        out.extend_from_slice(&face.to_le_bytes());
        out.extend_from_slice(&label.to_le_bytes());
    }
    if !entry.gradients.is_empty() {
        out.extend_from_slice(&(entry.gradients.len() as u32).to_le_bytes());
        for &record in &entry.gradients {
            write_gradient(out, record);
        }
    }
    if let Some(context) = &entry.gradient_context {
        out.extend_from_slice(&context.radius.to_le_bytes());
        out.push(u8::from(context.whole_region));
        out.push(context.stops.len() as u8);
        out.extend_from_slice(&(context.points.len() as u16).to_le_bytes());
        for stop in &context.stops {
            out.extend_from_slice(stop);
        }
        for hit in &context.points {
            out.extend_from_slice(&hit.face_id.to_le_bytes());
            for value in hit.point {
                out.extend_from_slice(&value.to_le_bytes());
            }
        }
    }
}

/// [`encode_history`] with an explicit budget, so tests can exercise trimming
/// without materialising 64 MiB of payload.
///
/// Entries are kept from the **newest** end. Undo keeps a contiguous suffix
/// (the most recent operations, which is what a user wants after reopening);
/// redo keeps a contiguous suffix too, dropping its bottom entries so the
/// remaining ones stay contiguous with the top — the next redo. If even the
/// newest undo entry does not fit, no entry at all is written. The container
/// itself is always valid (the two count fields), even when the budget is
/// smaller than those eight bytes.
#[cfg(test)]
fn encode_history_with_budget(history: &History, budget: usize) -> Vec<u8> {
    encode_history_with_budget_report(history, budget).0
}

fn encode_history_with_budget_report(history: &History, budget: usize) -> (Vec<u8>, bool) {
    let (undo, redo) = history.to_raw();
    let mut remaining = budget.saturating_sub(HISTORY_HEADER_BYTES);

    let mut kept_undo = 0usize;
    let mut undo_bytes = 0usize;
    for entry in undo.iter().rev() {
        let len = entry_wire_len(entry);
        if len > remaining {
            break;
        }
        remaining -= len;
        undo_bytes += len;
        kept_undo += 1;
    }

    // Neither stack is worth keeping if the applied timeline's newest step was
    // too large to save: a history whose most recent operation is missing
    // would undo the user's work in the wrong order. (If the undo stack is
    // empty the redo branch is still a legitimate state; it is budgeted
    // below.)
    if !undo.is_empty() && kept_undo == 0 {
        return (empty_history_container(), true);
    }

    let mut kept_redo = 0usize;
    let mut redo_bytes = 0usize;
    for entry in redo.iter().rev() {
        let len = entry_wire_len(entry);
        if len > remaining {
            break;
        }
        remaining -= len;
        redo_bytes += len;
        kept_redo += 1;
    }

    let mut out = Vec::with_capacity(HISTORY_HEADER_BYTES + undo_bytes + redo_bytes);
    out.extend_from_slice(&(kept_undo as u32).to_le_bytes());
    for entry in &undo[undo.len() - kept_undo..] {
        write_entry(&mut out, entry);
    }
    out.extend_from_slice(&(kept_redo as u32).to_le_bytes());
    for entry in &redo[redo.len() - kept_redo..] {
        write_entry(&mut out, entry);
    }
    let truncated =
        history.persisted_truncated() || kept_undo < undo.len() || kept_redo < redo.len();
    (out, truncated)
}

/// A valid container with neither stack populated.
fn empty_history_container() -> Vec<u8> {
    let mut out = Vec::with_capacity(HISTORY_HEADER_BYTES);
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    out
}

/// Parse `history_<i>.bin`.
///
/// `face_count` is the mesh the history is about to be applied to. Every face
/// index is validated against it: [`History::from_raw`] indexes the live
/// buffers on the first undo, so an out-of-range index saved into a project
/// would panic later. When anything makes the history unusable the whole
/// stack is dropped (`Ok(None)`) rather than partially restored, because a
/// partial timeline silently changes what Ctrl+Z does.
///
/// The file-supplied counts are bounded by the format's own budgets before any
/// buffer is allocated: the part may not exceed
/// [`MAX_PERSISTED_HISTORY_BYTES`], each stack may not exceed the in-memory
/// depth limit, and the decoded payload may not exceed the in-memory byte
/// budget. Without that, a crafted 512 MB (allowed) entry could ask
/// [`Vec::with_capacity`] for gigabytes.
pub fn decode_history_with_ramps(
    bytes: &[u8],
    face_count: usize,
    ramps: &[Vec<[u8; 4]>],
) -> Result<Option<History>, String> {
    if bytes.len() > MAX_PERSISTED_HISTORY_BYTES {
        return Err(format!(
            "history: {} bytes exceeds the {} byte persisted-history budget",
            bytes.len(),
            MAX_PERSISTED_HISTORY_BYTES
        ));
    }
    let mut cursor = Cursor::new(bytes);

    let undo_count = cursor.u32()? as usize;
    if undo_count > DEFAULT_MAX_DEPTH {
        log::warn!(
            "history: {undo_count} undo entries exceed the {DEFAULT_MAX_DEPTH}-entry depth \
             budget; dropping the persisted history"
        );
        return Ok(None);
    }
    if !cursor.has_room_for(undo_count, MIN_ENTRY_BYTES) {
        return Err("history: size mismatch in the undo stack".to_string());
    }
    // The count is at most `DEFAULT_MAX_DEPTH` now; clamping (rather than
    // passing a file-supplied value) makes the capacity bound explicit.
    let mut undo = Vec::with_capacity(undo_count.min(DEFAULT_MAX_DEPTH));
    let mut decoded_bytes = 0usize;
    for _ in 0..undo_count {
        match decode_entry(&mut cursor, face_count, ramps)? {
            Some(entry) => {
                decoded_bytes = decoded_bytes.saturating_add(entry_memory_bytes(&entry));
                if decoded_bytes > DEFAULT_BYTE_BUDGET {
                    log::warn!(
                        "history: decoded undo payload exceeds the {DEFAULT_BYTE_BUDGET}-byte \
                         budget; dropping the persisted history"
                    );
                    return Ok(None);
                }
                undo.push(entry);
            }
            None => return Ok(None),
        }
    }

    let redo_count = cursor.u32()? as usize;
    if redo_count > DEFAULT_MAX_DEPTH {
        log::warn!(
            "history: {redo_count} redo entries exceed the {DEFAULT_MAX_DEPTH}-entry depth \
             budget; dropping the persisted history"
        );
        return Ok(None);
    }
    if !cursor.has_room_for(redo_count, MIN_ENTRY_BYTES) {
        return Err("history: size mismatch in the redo stack".to_string());
    }
    let mut redo = Vec::with_capacity(redo_count.min(DEFAULT_MAX_DEPTH));
    for _ in 0..redo_count {
        match decode_entry(&mut cursor, face_count, ramps)? {
            Some(entry) => {
                decoded_bytes = decoded_bytes.saturating_add(entry_memory_bytes(&entry));
                if decoded_bytes > DEFAULT_BYTE_BUDGET {
                    log::warn!(
                        "history: decoded history payload exceeds the {DEFAULT_BYTE_BUDGET}-byte \
                         budget; dropping the persisted history"
                    );
                    return Ok(None);
                }
                redo.push(entry);
            }
            None => return Ok(None),
        }
    }

    if cursor.remaining() != 0 {
        return Err(format!(
            "history: size mismatch, {} trailing bytes",
            cursor.remaining()
        ));
    }

    // `from_raw` charges the loaded payload by `len`; both vectors were built
    // with `with_capacity(n)` and exactly `n` pushes, so their capacity does
    // not overstate what is stored.
    Ok(Some(History::from_raw(undo, redo)))
}

#[cfg(test)]
pub fn decode_history(bytes: &[u8], face_count: usize) -> Result<Option<History>, String> {
    decode_history_with_ramps(bytes, face_count, &[])
}

/// In-memory footprint of one decoded entry, matching [`History::from_raw`]'s
/// len-based accounting. Used to keep a file from exceeding the live history's
/// byte budget.
fn entry_memory_bytes(entry: &RawEntry) -> usize {
    std::mem::size_of::<RawEntry>()
        + entry.colors.len() * std::mem::size_of::<(u32, [u8; 4])>()
        + entry.labels.len() * std::mem::size_of::<(u32, u32)>()
        + entry.gradients.len() * std::mem::size_of::<(u32, GradientSample)>()
        + entry
            .gradient_context
            .as_deref()
            .map_or(0, GradientContext::bytes)
}

fn decode_entry(
    cursor: &mut Cursor<'_>,
    face_count: usize,
    ramps: &[Vec<[u8; 4]>],
) -> Result<Option<RawEntry>, String> {
    let kind_byte = cursor.u8()?;
    let Some(kind) = OpKind::from_u8(kind_byte) else {
        log::warn!("history: unknown op kind {kind_byte}; dropping the persisted history");
        return Ok(None);
    };

    let flags = cursor.u8()?;
    let has_ramp_ids = flags & 8 != 0;
    if flags & !15 != 0 || (has_ramp_ids && flags & 2 == 0) {
        return Err(format!("history: unknown entry flags {flags}"));
    }
    let stroke_id = (flags & 1 != 0).then(|| cursor.u64()).transpose()?;

    let color_count = cursor.u32()? as usize;
    // Each colour record is a u32 face plus four RGBA bytes.
    if !cursor.has_room_for(color_count, 8) {
        return Err("history: size mismatch in the colour payload".to_string());
    }
    // Never pre-allocate a file-supplied count beyond what the mesh can hold:
    // an entry cannot touch more faces than exist.
    let mut colors = Vec::with_capacity(color_count.min(face_count));
    for _ in 0..color_count {
        let face = cursor.u32()?;
        let rgba: [u8; 4] = cursor
            .take(4)?
            .try_into()
            .expect("Cursor::take returned exactly four bytes");
        if !face_is_valid(face, face_count) {
            log::warn!(
                "history: face {face} is out of range for a {face_count}-face mesh; dropping the persisted history"
            );
            return Ok(None);
        }
        colors.push((face, rgba));
    }

    let label_count = cursor.u32()? as usize;
    // Each label record is a u32 face plus a u32 label.
    if !cursor.has_room_for(label_count, 8) {
        return Err("history: size mismatch in the label payload".to_string());
    }
    let mut labels = Vec::with_capacity(label_count.min(face_count));
    for _ in 0..label_count {
        let face = cursor.u32()?;
        let label = cursor.u32()?;
        if !face_is_valid(face, face_count) {
            log::warn!(
                "history: face {face} is out of range for a {face_count}-face mesh; dropping the persisted history"
            );
            return Ok(None);
        }
        labels.push((face, label));
    }

    let mut gradients = Vec::new();
    if flags & 2 != 0 {
        let count = cursor.u32()? as usize;
        if count == 0
            || count > color_count.min(face_count)
            || !cursor.has_room_for(count, gradient_record_bytes(has_ramp_ids))
        {
            return Err("history: invalid gradient record count".into());
        }
        // Gradient records follow the colour records' order, but omit the
        // faces that held solid paint. Check membership, order and RGBA.
        let mut color_entries = colors.iter();
        gradients.reserve_exact(count);
        for _ in 0..count {
            let (face, sample) = read_gradient(cursor, has_ramp_ids, ramps)?;
            let previous_color = color_entries.find(|(candidate, _)| *candidate == face);
            if previous_color.map(|&(_, color)| color) != Some(sample.rgba()) {
                return Err("history: gradient disagrees with the colour payload".into());
            }
            gradients.push((face, sample));
        }
    }

    let gradient_context = match (kind, flags & 4 != 0) {
        (OpKind::GradientBrush, true) => Some(Box::new(read_gradient_context(
            cursor,
            GradientTool::Brush,
            face_count,
        )?)),
        (OpKind::GradientFill, true) => Some(Box::new(read_gradient_context(
            cursor,
            GradientTool::Fill,
            face_count,
        )?)),
        (OpKind::GradientBrush | OpKind::GradientFill, false) | (_, true) => {
            return Err("history: invalid gradient context flag".into());
        }
        _ => None,
    };

    Ok(Some(RawEntry {
        kind,
        stroke_id,
        colors,
        labels,
        gradients,
        gradient_context,
    }))
}

fn read_gradient_context(
    cursor: &mut Cursor<'_>,
    tool: GradientTool,
    face_count: usize,
) -> Result<GradientContext, String> {
    let radius = cursor.f32()?;
    let region_flag = cursor.u8()?;
    let stop_count = cursor.u8()? as usize;
    let point_count = cursor.u16()? as usize;
    let valid_gesture = match tool {
        GradientTool::Brush => region_flag == 0 && (1..=MAX_POINTS).contains(&point_count),
        GradientTool::Fill => region_flag <= 1 && point_count == 2,
    };
    if !valid_gesture
        || !radius.is_finite()
        || radius <= 0.0
        || radius > MAX_RADIUS
        || !(2..=MAX_STOPS).contains(&stop_count)
        || cursor.remaining() < stop_count * 4 + point_count * GRADIENT_HIT_BYTES
    {
        return Err("history: invalid gradient context".into());
    }
    let mut stops = Vec::with_capacity(stop_count);
    for _ in 0..stop_count {
        stops.push(cursor.take(4)?.try_into().unwrap());
    }
    let mut points = Vec::with_capacity(point_count);
    for _ in 0..point_count {
        let face_id = cursor.u32()?;
        let point = [cursor.f32()?, cursor.f32()?, cursor.f32()?];
        if !face_is_valid(face_id, face_count) || !point.iter().all(|value| value.is_finite()) {
            return Err("history: invalid gradient hit".into());
        }
        points.push(GradientHit { face_id, point });
    }
    Ok(GradientContext {
        tool,
        stops: stops.into_boxed_slice(),
        points: points.into_boxed_slice(),
        radius,
        whole_region: region_flag == 1,
    })
}

fn face_is_valid(face: u32, face_count: usize) -> bool {
    u64::from(face) < face_count as u64
}

// ---------------------------------------------------------------------------
// Cursor
// ---------------------------------------------------------------------------

/// Bounds-checked reader. Every accessor checks the length before slicing, so
/// no untrusted byte can panic a decode.
struct Cursor<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl<'a> Cursor<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, pos: 0 }
    }

    fn remaining(&self) -> usize {
        self.bytes.len() - self.pos
    }

    fn has_room_for(&self, count: usize, min_bytes_each: usize) -> bool {
        (count as u64).saturating_mul(min_bytes_each as u64) <= self.remaining() as u64
    }

    fn take(&mut self, len: usize) -> Result<&'a [u8], String> {
        let end = self
            .pos
            .checked_add(len)
            .ok_or_else(|| "size mismatch: length overflow".to_string())?;
        if end > self.bytes.len() {
            return Err(format!(
                "size mismatch: need {len} bytes at offset {}, only {} remain",
                self.pos,
                self.remaining()
            ));
        }
        let slice = &self.bytes[self.pos..end];
        self.pos = end;
        Ok(slice)
    }

    fn u8(&mut self) -> Result<u8, String> {
        Ok(self.take(1)?[0])
    }

    fn u16(&mut self) -> Result<u16, String> {
        let bytes: [u8; 2] = self
            .take(2)?
            .try_into()
            .expect("Cursor::take returned exactly two bytes");
        Ok(u16::from_le_bytes(bytes))
    }

    fn u32(&mut self) -> Result<u32, String> {
        let bytes: [u8; 4] = self
            .take(4)?
            .try_into()
            .expect("Cursor::take returned exactly four bytes");
        Ok(u32::from_le_bytes(bytes))
    }

    fn f32(&mut self) -> Result<f32, String> {
        self.u32().map(f32::from_bits)
    }

    fn u64(&mut self) -> Result<u64, String> {
        let bytes: [u8; 8] = self
            .take(8)?
            .try_into()
            .expect("Cursor::take returned exactly eight bytes");
        Ok(u64::from_le_bytes(bytes))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mesh::history::{History, OpKind, RawEntry};
    use crate::mesh::model::MeshModel;
    use std::collections::HashMap;

    #[test]
    fn gradient_parts_and_history_accept_legacy_samples_and_validate_new_ramp_ids() {
        let ramp = vec![[255, 0, 0, 255], [0, 255, 0, 255], [0, 0, 255, 255]];
        let sample = GradientSample {
            from: ramp[1],
            to: ramp[2],
            mix: 32768,
            ramp_id: 1,
        };
        let colors = [sample.rgba()];
        let mut mesh = MeshModel::new();
        mesh.gradient_ramps.push(ramp);
        mesh.gradient_faces.insert(0, sample);
        let bytes = encode_gradient_faces(&mesh);
        let (faces, ramps) = decode_gradient_faces(&bytes, &colors).unwrap();
        assert_eq!(faces, mesh.gradient_faces);
        assert_eq!(ramps, mesh.gradient_ramps);
        // A paint entry can restore a gradient even when the live mesh has
        // no gradient faces. The ramp table belongs to the whole plate.
        mesh.gradient_faces.clear();
        assert_eq!(
            decode_gradient_faces(&encode_gradient_faces(&mesh), &colors)
                .unwrap()
                .1,
            ramps
        );

        let entry = RawEntry {
            kind: OpKind::Paint,
            stroke_id: None,
            colors: vec![(0, colors[0])],
            labels: vec![],
            gradients: vec![(0, sample)],
            gradient_context: None,
        };
        let mut history = vec![];
        write_entry(&mut history, &entry);
        assert_eq!(history.len(), entry_wire_len(&entry));
        let decoded = decode_entry(&mut Cursor::new(&history), 1, &ramps)
            .unwrap()
            .unwrap();
        assert_eq!(decoded.gradients, entry.gradients);
        assert!(decode_entry(&mut Cursor::new(&history), 1, &[]).is_err());
        history[1] &= !8;
        history.truncate(history.len() - 4);
        let decoded = decode_entry(&mut Cursor::new(&history), 1, &[])
            .unwrap()
            .unwrap();
        let legacy = GradientSample {
            ramp_id: 0,
            ..sample
        };
        assert_eq!(decoded.gradients, [(0, legacy)]);

        let mut legacy_part = LEGACY_GRADIENT_MAGIC.to_vec();
        let record_start = bytes.len() - GRADIENT_RECORD_BYTES;
        legacy_part.extend_from_slice(&bytes[record_start..bytes.len() - 4]);
        assert_eq!(
            decode_gradient_faces(&legacy_part, &colors).unwrap(),
            (HashMap::from([(0, legacy)]), vec![])
        );
        let mut corrupt = bytes.clone();
        *corrupt.last_mut().unwrap() = 1;
        assert!(decode_gradient_faces(&corrupt, &colors).is_err());
        let mut corrupt = bytes;
        corrupt[GRADIENT_MAGIC.len()..GRADIENT_MAGIC.len() + 4].fill(255);
        assert!(decode_gradient_faces(&corrupt, &colors).is_err());
    }

    /// A mesh with every persisted field set to a non-default value, including
    /// a non-ASCII region name.
    fn stateful_mesh() -> MeshModel {
        let mut mesh = MeshModel::new();
        mesh.vertices = vec![
            [0.0, 0.0, 0.0],
            [1.0, 0.0, 0.0],
            [0.0, 1.0, 0.0],
            [1.0, 1.0, 0.0],
        ];
        mesh.faces = vec![[0, 1, 2], [1, 3, 2], [0, 1, 3], [0, 3, 2]];
        mesh.face_colors = vec![
            [10, 20, 30, 255],
            [0, 0, 0, 255],
            [200, 100, 50, 128],
            [138, 138, 138, 255],
        ];
        mesh.segment_labels = vec![0, 100_000, 100_001, 7];
        mesh.next_manual_label = 100_002;
        mesh.segment_names = HashMap::new();
        mesh.segment_names.insert(100_000, "左臂".to_string());
        mesh.segment_names.insert(100_001, "right arm".to_string());
        mesh.segment_names.insert(7, "auto".to_string());
        mesh.unit = "millimeter".to_string();
        mesh
    }

    #[test]
    fn plate_state_round_trips_every_field() {
        let mesh = stateful_mesh();
        let bytes = encode_plate_state(&mesh);

        assert_eq!(PLATE_MAGIC.len(), 8, "the header contract is 8 magic bytes");
        assert_eq!(&bytes[..8], PLATE_MAGIC.as_slice());
        assert_eq!(
            u32::from_le_bytes(bytes[8..12].try_into().unwrap()),
            4,
            "redundant face_count must be written"
        );

        let decoded = decode_plate_state(&bytes, mesh.faces.len()).expect("well-formed state");
        assert_eq!(decoded.segment_labels, mesh.segment_labels);
        assert_eq!(decoded.face_colors, mesh.face_colors);
        assert_eq!(
            decoded.next_manual_label, 100_002,
            "high-water restored verbatim"
        );
        assert_eq!(decoded.segment_names.len(), 3);
        assert_eq!(
            decoded.segment_names.get(&100_000).map(String::as_str),
            Some("左臂")
        );
        assert_eq!(
            decoded.segment_names.get(&100_001).map(String::as_str),
            Some("right arm")
        );
        assert_eq!(
            decoded.segment_names.get(&7).map(String::as_str),
            Some("auto")
        );
    }

    #[test]
    fn plate_state_rejects_a_face_count_that_does_not_match_the_mesh() {
        let mesh = stateful_mesh();
        let bytes = encode_plate_state(&mesh);
        assert!(
            decode_plate_state(&bytes, mesh.faces.len() + 1).is_err(),
            "a state for a different face count cannot be applied"
        );
    }

    #[test]
    fn plate_state_with_a_bad_magic_is_rejected() {
        let mut bytes = vec![0u8; 8 + 4 + 4];
        bytes[..8].copy_from_slice(b"NOTCYM!!");
        assert!(decode_plate_state(&bytes, 0).is_err());
    }

    /// The face count is file-supplied. A huge declared count on a tiny body
    /// must be rejected by the length check, not by trying to allocate.
    #[test]
    fn truncated_plate_state_is_an_error_before_any_allocation() {
        let declared: u32 = 0xFFFF_FFF0;
        let mut bytes = Vec::new();
        bytes.extend_from_slice(PLATE_MAGIC);
        bytes.extend_from_slice(&declared.to_le_bytes());
        let err = decode_plate_state(&bytes, declared as usize)
            .expect_err("a header with no body must not decode");
        assert!(err.contains("mismatch"), "unexpected error: {err}");
    }

    #[test]
    fn a_short_body_for_the_declared_face_count_is_an_error() {
        // face_count = 3, but only the label array follows; the RGBA array,
        // high-water mark and name table are all missing.
        let mut bytes = Vec::new();
        bytes.extend_from_slice(PLATE_MAGIC);
        bytes.extend_from_slice(&3u32.to_le_bytes());
        bytes.extend_from_slice(&0u32.to_le_bytes());
        bytes.extend_from_slice(&0u32.to_le_bytes());
        bytes.extend_from_slice(&0u32.to_le_bytes());
        assert!(decode_plate_state(&bytes, 3).is_err());

        // A complete body plus one trailing byte is also not a valid state.
        let mesh = stateful_mesh();
        let mut good = encode_plate_state(&mesh);
        good.push(0);
        assert!(decode_plate_state(&good, mesh.faces.len()).is_err());
    }

    #[test]
    fn the_fingerprint_uses_canonicalized_coordinates_not_raw_f32_bits() {
        // Regression: a project written with {:.6} vertices and re-read gets a
        // different f32 bit pattern for coordinates with more precision than
        // the writer keeps. Hashing raw bits would make every re-opened project
        // fail its own fingerprint.
        let v = vec![
            [0.123456789_f32, 1.0, 2.0],
            [3.0, 4.0, 5.0],
            [6.0, 7.0, 8.0],
        ];
        let faces = vec![[0u32, 1, 2]];
        let reparsed: Vec<[f32; 3]> = v
            .iter()
            .map(|p| p.map(|c| format!("{:.6}", c).parse::<f32>().unwrap()))
            .collect();
        assert_ne!(
            reparsed[0][0].to_bits(),
            v[0][0].to_bits(),
            "precondition: this value must actually lose bits through {{:.6}}"
        );

        let raw = fingerprint_of(&v, &faces);
        let canonical = fingerprint_of(&reparsed, &faces);
        assert_eq!(
            raw.hash, canonical.hash,
            "raw and re-parsed inputs must fingerprint identically"
        );
        assert_eq!(raw.vertex_count, 3);
        assert_eq!(raw.face_count, 1);
        assert_eq!(canonical.vertex_count, 3);
        assert_eq!(canonical.face_count, 1);
    }

    #[test]
    fn a_different_geometries_do_not_share_a_fingerprint() {
        let faces = vec![[0u32, 1, 2]];
        let a = fingerprint_of(&[[0.0, 1.0, 2.0], [3.0, 4.0, 5.0], [6.0, 7.0, 8.0]], &faces);
        let b = fingerprint_of(&[[0.0, 1.0, 2.0], [3.0, 4.0, 5.0], [6.0, 7.0, 9.0]], &faces);
        assert_ne!(a.hash, b.hash);

        let other_faces = vec![[0u32, 2, 1]];
        assert_ne!(
            a.hash,
            fingerprint_of(
                &[[0.0, 1.0, 2.0], [3.0, 4.0, 5.0], [6.0, 7.0, 8.0]],
                &other_faces
            )
            .hash
        );
    }

    #[test]
    fn a_wrong_major_version_is_rejected() {
        assert!(decode_project_json(r#"{"format_version":2,"plates":[]}"#).is_err());
        assert!(
            decode_project_json(r#"{"format_version":0,"active_plate":1,"plates":[]}"#).is_err()
        );
        assert!(
            decode_project_json(r#"{"active_plate":1,"plates":[]}"#).is_err(),
            "a file without a format version is not a project header"
        );
    }

    #[test]
    fn project_header_round_trips_unit_plates_and_fingerprints() {
        let state = ProjectState {
            active_plate: 2,
            paint_sources_complete: true,
            plates: vec![
                PlateStateHeader {
                    history_truncated: false,
                    placement_offset: [0.0; 3],
                    plater_id: 1,
                    name: "左盘".to_string(),
                    object_ids: vec![3, 9],
                    fingerprint: Fingerprint {
                        vertex_count: 74226,
                        face_count: 993568,
                        hash: 0x0123_4567_89ab_cdef,
                    },
                },
                PlateStateHeader {
                    history_truncated: false,
                    placement_offset: [0.0; 3],
                    plater_id: 2,
                    name: "plate 2".to_string(),
                    object_ids: vec![],
                    fingerprint: Fingerprint {
                        vertex_count: 0,
                        face_count: 0,
                        hash: 0,
                    },
                },
            ],
            unit: "inch".to_string(),
        };

        let text = encode_project_json(&state);
        let decoded = decode_project_json(&text).expect("round trip");
        assert_eq!(decoded, state);
        assert!(text.contains("\"format_version\": 1") || text.contains("\"format_version\":1"));
        assert!(text.contains("\"app\""));
    }

    #[test]
    fn unknown_fields_are_tolerated_and_a_missing_unit_defaults_to_millimeter() {
        let text = r#"{
            "format_version": 1,
            "app": "ColorYourModel",
            "someFutureField": {"nested": [1, 2, 3]},
            "active_plate": 1,
            "plates": [{
                "plater_id": 1,
                "name": "plate 1",
                "object_ids": [1],
                "state": "plate_1.bin",
                "history": "history_1.bin",
                "futurePerPlateField": true,
                "fingerprint": {
                    "vertex_count": 3,
                    "face_count": 1,
                    "hash": "fnv1a64:0123456789abcdef",
                    "futureFingerprintField": "ignored"
                }
            }]
        }"#;
        let decoded = decode_project_json(text).expect("unknown fields must be ignored");
        assert_eq!(decoded.unit, "millimeter");
        assert_eq!(decoded.active_plate, 1);
        assert_eq!(decoded.plates.len(), 1);
        assert_eq!(decoded.plates[0].fingerprint.hash, 0x0123_4567_89ab_cdef);
    }

    /// Paint (colour + stroke id) then merge (label-only, no stroke id), so a
    /// round trip has to carry both payload shapes and both flag values.
    fn two_entry_history() -> (History, Vec<[u8; 4]>, Vec<u32>) {
        let mut colors = vec![[0u8, 0, 0, 255]; 3];
        let mut labels = vec![0u32; 3];
        let mut history = History::new();

        history.record(OpKind::Paint, Some(7), &[(0, colors[0])], &[]);
        colors[0] = [10, 20, 30, 255];

        history.record(OpKind::Merge, None, &[], &[(2, labels[2])]);
        labels[2] = 100_000;

        (history, colors, labels)
    }

    #[test]
    fn history_round_trips_and_stays_self_inverse() {
        let (mut history, colors_after_all, labels_after_all) = two_entry_history();
        let bytes = encode_history(&history);
        let mut restored = decode_history(&bytes, 3)
            .expect("well-formed history")
            .expect("known kinds must decode");

        assert_eq!(
            restored.to_raw(),
            history.to_raw(),
            "payloads and stack order"
        );
        assert_eq!(restored.undo_depth(), 2);
        assert_eq!(restored.redo_depth(), 0);

        let (undo, _redo) = history.to_raw();
        assert_eq!(undo[0].kind, OpKind::Paint);
        assert_eq!(undo[0].stroke_id, Some(7));
        assert_eq!(undo[1].kind, OpKind::Merge);
        assert_eq!(undo[1].stroke_id, None);

        // Undo both stacks in lockstep: the restored entries must be their own
        // inverse exactly like the originals.
        let mut live_colors = colors_after_all.clone();
        let mut live_labels = labels_after_all.clone();
        let mut restored_colors = colors_after_all;
        let mut restored_labels = labels_after_all;
        for step in 0..2 {
            let live = history.undo(&mut live_colors, &mut live_labels);
            let reloaded = restored.undo(&mut restored_colors, &mut restored_labels);
            assert_eq!(
                live.is_some(),
                reloaded.is_some(),
                "undo {step} availability"
            );
            assert_eq!(live_colors, restored_colors, "undo {step} colours");
            assert_eq!(live_labels, restored_labels, "undo {step} labels");
        }
        assert!(!history.can_undo() && !restored.can_undo());
        assert_eq!(live_colors, vec![[0, 0, 0, 255]; 3]);
        assert_eq!(live_labels, vec![0, 0, 0]);

        // Redo back to the saved state.
        for step in 0..2 {
            let live = history.redo(&mut live_colors, &mut live_labels);
            let reloaded = restored.redo(&mut restored_colors, &mut restored_labels);
            assert_eq!(
                live.is_some(),
                reloaded.is_some(),
                "redo {step} availability"
            );
            assert_eq!(live_colors, restored_colors, "redo {step} colours");
            assert_eq!(live_labels, restored_labels, "redo {step} labels");
        }
        assert_eq!(
            live_colors,
            vec![[10, 20, 30, 255], [0, 0, 0, 255], [0, 0, 0, 255]]
        );
        assert_eq!(live_labels, vec![0, 0, 100_000]);
    }

    #[test]
    fn an_unknown_kind_drops_the_whole_history() {
        let (history, _colors, _labels) = two_entry_history();
        let mut bytes = encode_history(&history);
        // undo_count is the first u32; the first entry's kind byte follows.
        bytes[4] = 5; // reserved for a future model-transform op
        assert!(
            matches!(decode_history(&bytes, 3), Ok(None)),
            "kind 5 must drop the history"
        );

        let mut other = encode_history(&history);
        other[4] = 200;
        assert!(
            matches!(decode_history(&other, 3), Ok(None)),
            "unknown kinds must drop it too"
        );
    }

    #[test]
    fn an_out_of_range_face_index_drops_the_whole_history() {
        let (history, _colors, _labels) = two_entry_history();
        let bytes = encode_history(&history);
        assert!(
            matches!(decode_history(&bytes, 2), Ok(None)),
            "face 2 is out of range for a 2-face mesh"
        );
        assert!(decode_history(&bytes, 3).expect("in range").is_some());
    }

    #[test]
    fn a_truncated_or_corrupt_history_is_an_error() {
        let (history, _colors, _labels) = two_entry_history();
        let bytes = encode_history(&history);
        assert!(decode_history(&[], 3).is_err());
        for cut in 1..bytes.len() {
            assert!(
                decode_history(&bytes[..cut], 3).is_err(),
                "a truncation at {cut} bytes must not decode"
            );
        }

        // undo_count(u32) | kind | stroke flag: flag 2 is not a defined value.
        let mut bad_flag = bytes.clone();
        bad_flag[5] = 2;
        assert!(decode_history(&bad_flag, 3).is_err());
    }

    #[test]
    fn history_truncation_report_distinguishes_complete_trimmed_and_previously_trimmed_stacks() {
        let mut history = History::new();
        assert!(!encode_history_with_budget_report(&history, 100).1);
        history.record(OpKind::Paint, None, &[(0, [0, 0, 0, 255])], &[]);
        assert!(!encode_history_with_budget_report(&history, 100).1);
        assert!(encode_history_with_budget_report(&history, 8).1);
        history.set_persisted_truncated(true);
        assert!(encode_history_with_budget_report(&history, 100).1);
        history.clear();
        assert!(!encode_history_with_budget_report(&history, 100).1);
    }

    #[test]
    fn history_over_the_byte_budget_drops_the_oldest_entries() {
        let mut colors = vec![[0u8, 0, 0, 255]; 4];
        let mut labels = vec![0u32; 4];
        let mut history = History::new();
        for i in 0..3u32 {
            history.record(
                OpKind::Paint,
                Some(i as u64 + 1),
                &[(i, colors[i as usize])],
                &[],
            );
            colors[i as usize] = [i as u8 + 1, 0, 0, 255];
        }

        // One entry is 1(kind) + 1(flag) + 8(stroke) + 4 + 8 + 4 = 26 bytes.
        // 60 = 8-byte container header + the two newest entries; the oldest
        // does not fit and must be dropped.
        let (bytes, truncated) = encode_history_with_budget_report(&history, 60);
        assert!(truncated);
        assert!(bytes.len() <= 60, "budget must bound the written bytes");
        let mut restored = decode_history(&bytes, 4)
            .expect("budget-trimmed history is still well formed")
            .expect("trimming keeps known kinds");
        assert_eq!(restored.undo_depth(), 2, "only the newest two survive");

        let mut live = colors.clone();
        restored.undo(&mut live, &mut labels).unwrap();
        assert_eq!(live[2], [0, 0, 0, 255], "the newest entry undoes first");
        assert_eq!(live[1], [2, 0, 0, 255], "the older entry has not run yet");
        restored.undo(&mut live, &mut labels).unwrap();
        assert_eq!(live[1], [0, 0, 0, 255]);
        assert_eq!(
            live[0],
            [1, 0, 0, 255],
            "the oldest entry was dropped, so face 0 stays painted"
        );
        assert!(!restored.can_undo(), "the oldest entry was dropped");
    }

    #[test]
    fn an_entry_bigger_than_the_whole_budget_writes_an_empty_container() {
        let mut colors = vec![[0u8, 0, 0, 255]; 4];
        let mut history = History::new();
        history.record(OpKind::Paint, None, &[(0, colors[0])], &[]);
        colors[0] = [1, 0, 0, 255];

        let bytes = encode_history_with_budget(&history, 8);
        let restored = decode_history(&bytes, 4)
            .expect("valid container")
            .expect("decodes");
        assert!(!restored.can_undo());
        assert!(!restored.can_redo());
        assert_eq!(bytes.len(), 8, "container stays valid with zero entries");
    }

    #[test]
    fn an_oversized_undo_entry_writes_no_entries_at_all() {
        // The redo entry would fit in the leftover budget, but a history whose
        // newest undo step cannot be saved must not be resurrected as a
        // redo-only timeline.
        let undo_entry = RawEntry {
            kind: OpKind::Paint,
            stroke_id: None,
            colors: (0..16u32).map(|face| (face, [1, 2, 3, 4])).collect(),
            labels: vec![],
            gradients: Vec::new(),
            gradient_context: None,
        };
        let redo_entry = RawEntry {
            kind: OpKind::Paint,
            stroke_id: None,
            colors: vec![(20, [5, 6, 7, 8])],
            labels: vec![],
            gradients: Vec::new(),
            gradient_context: None,
        };
        let history = History::from_raw(vec![undo_entry], vec![redo_entry]);

        // 8 header + 18 redo + 10 spare: the redo entry fits, the 138-byte
        // undo entry does not.
        let bytes = encode_history_with_budget(&history, 8 + 18 + 10);
        assert_eq!(bytes.len(), 8, "a valid but empty container");
        let restored = decode_history(&bytes, 32)
            .expect("valid container")
            .expect("decodes");
        assert_eq!(restored.undo_depth(), 0);
        assert_eq!(restored.redo_depth(), 0);
    }

    #[test]
    fn over_budget_history_keeps_the_top_of_the_redo_stack() {
        // An empty undo stack with a populated redo branch is a legitimate
        // state (everything undone), so the budget must trim redo from its
        // bottom, keeping the entries contiguous with the top.
        let bottom = RawEntry {
            kind: OpKind::Paint,
            stroke_id: None,
            colors: vec![(0, [1, 2, 3, 4])],
            labels: vec![],
            gradients: Vec::new(),
            gradient_context: None,
        };
        let top = RawEntry {
            kind: OpKind::Paint,
            stroke_id: None,
            colors: vec![(1, [5, 6, 7, 8])],
            labels: vec![],
            gradients: Vec::new(),
            gradient_context: None,
        };
        let history = History::from_raw(Vec::new(), vec![bottom, top]);
        // Entry wire size = 1+1+4+8+4 = 18; header = 8. One 18-byte slot fits.
        let bytes = encode_history_with_budget(&history, 8 + 18);
        let mut restored = decode_history(&bytes, 4)
            .expect("valid")
            .expect("known kinds");
        assert_eq!(restored.undo_depth(), 0);
        assert_eq!(restored.redo_depth(), 1, "only the redo top survives");

        let mut colors = vec![[0u8, 0, 0, 255]; 2];
        let mut labels = vec![0u32; 2];
        restored.redo(&mut colors, &mut labels).unwrap();
        assert_eq!(colors[1], [5, 6, 7, 8], "the top entry is the next redo");
        assert_eq!(colors[0], [0, 0, 0, 255], "the trimmed entry never runs");
    }

    #[test]
    fn an_empty_history_round_trips_as_an_empty_container() {
        let bytes = encode_history(&History::new());
        assert_eq!(bytes.len(), 8);
        let restored = decode_history(&bytes, 0)
            .expect("valid container")
            .expect("empty history is Some");
        assert_eq!(restored.undo_depth(), 0);
        assert_eq!(restored.redo_depth(), 0);
    }

    /// The entry count is file-supplied. A body whose length is consistent
    /// with the count (a few MB) but whose count is thousands of times the
    /// depth budget must be dropped before `Vec::with_capacity` sees it — the
    /// pre-fix code would reserve one `RawEntry` slot per claimed entry.
    #[test]
    fn an_absurd_stack_count_is_dropped_before_it_can_be_allocated() {
        let count: u32 = 200_000;
        let body_len = 4 + count as usize * MIN_ENTRY_BYTES;
        let mut bytes = Vec::with_capacity(body_len);
        bytes.extend_from_slice(&count.to_le_bytes());
        bytes.resize(body_len, 0);

        // The length check alone would pass: every entry's minimum ten bytes
        // are present. Only the depth budget can reject this.
        assert!(body_len > 1_000_000, "the fixture is deliberately multi-MB");
        assert!(
            matches!(decode_history(&bytes, 1), Ok(None)),
            "an over-depth count must drop the history, not allocate for it"
        );
    }

    /// The persisted part may not exceed its own byte budget, even when the
    /// bytes would otherwise parse as a valid (empty) container.
    #[test]
    fn a_history_body_over_the_persisted_budget_is_rejected() {
        let mut bytes = vec![0u8; MAX_PERSISTED_HISTORY_BYTES + 1];
        // A valid empty container at the front does not excuse the excess.
        bytes[..8].copy_from_slice(&[0u8; 8]);
        let err = match decode_history(&bytes, 0) {
            Err(err) => err,
            Ok(_) => panic!("over-budget part must be rejected"),
        };
        assert!(err.contains("budget"), "unexpected error: {err}");
    }

    /// A forged name count whose six-bytes-per-name body is present passes the
    /// old length check, so only a plausible-count bound can stop the
    /// `HashMap::with_capacity` from reserving hundreds of megabytes.
    #[test]
    fn a_forged_region_name_count_is_rejected_before_the_hashmap_is_allocated() {
        let faces = 4usize;
        let forged: u32 = 1_000_000;
        let mut bytes = Vec::new();
        bytes.extend_from_slice(PLATE_MAGIC);
        bytes.extend_from_slice(&(faces as u32).to_le_bytes());
        // Labels (4 B/face) + RGBA (4 B/face) + the high-water mark.
        bytes.resize(8 + 4 + faces * 4 + faces * 4 + 4, 0);
        bytes.extend_from_slice(&forged.to_le_bytes());
        // One empty name record is u32 label + u16 len = 6 bytes; the body is
        // therefore "length-valid" for the forged count.
        bytes.resize(bytes.len() + forged as usize * 6, 0);

        let err = decode_plate_state(&bytes, faces)
            .expect_err("a name table 250,000x the face count is forged");
        assert!(err.contains("region-name"), "unexpected error: {err}");
    }

    #[test]
    fn the_persisted_history_budget_is_64_mib() {
        assert_eq!(MAX_PERSISTED_HISTORY_BYTES, 64 * 1024 * 1024);
    }
}
