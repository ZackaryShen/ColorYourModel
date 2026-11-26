//! Physical filament palettes and representative colours for native mixed slots.
//!
//! `paint_color` addresses the project's one-based extruder slots (see
//! [`super::paint`]). Physical slots come from `filament_colour`. Active
//! `mixed_filament_definitions` supply the virtual slots that follow them.
//! Both keys live in `Metadata/project_settings.config` and apply to all plates.

use serde_json::Value;

use crate::mesh::model::DEFAULT_FACE_COLOR;

/// A project's filament palette, 1-based by convention: slot `k` is index
/// `k - 1`. Entries are always opaque `[r, g, b, 255]`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Palette(pub Vec<[u8; 4]>);

pub(super) struct ProjectMaterials {
    pub(super) physical: Palette,
    pub(super) mixed: Vec<[u8; 4]>,
}

impl ProjectMaterials {
    pub(super) fn colour(&self, slot: u8) -> Option<[u8; 4]> {
        let index = usize::from(slot.checked_sub(1)?);
        if index < self.physical.0.len() {
            self.physical.0.get(index).copied()
        } else {
            self.mixed.get(index - self.physical.0.len()).copied()
        }
    }

    pub(super) fn source_colours(&self) -> Vec<Option<[u8; 4]>> {
        self.physical
            .0
            .iter()
            .chain(&self.mixed)
            .copied()
            .map(Some)
            .collect()
    }
}

pub(super) fn project_materials(json: &str) -> Result<ProjectMaterials, String> {
    let root = parse_project_settings(json)?;
    project_materials_from_config(&root)
}

pub(super) fn project_materials_from_config(root: &Value) -> Result<ProjectMaterials, String> {
    let physical = physical_palette(root)?;
    let mixed = root["mixed_filament_definitions"]
        .as_str()
        .unwrap_or_default()
        .split(';')
        .filter_map(|row| mixed_preview(row, &physical.0))
        .take(usize::from(u8::MAX).saturating_sub(physical.0.len()))
        .collect();
    Ok(ProjectMaterials { physical, mixed })
}

/// Parse `Metadata/project_settings.config` into a palette.
///
/// Accepts `#RRGGBB` in either case. Rejects a missing, empty or malformed
/// `filament_colour` with a message naming the key; the caller (the import
/// path) degrades any `Err` to "no palette", which renders default grey.
#[cfg(test)]
fn palette_from_project_settings(json: &str) -> Result<Palette, String> {
    physical_palette(&parse_project_settings(json)?)
}

fn parse_project_settings(json: &str) -> Result<Value, String> {
    serde_json::from_str(json).map_err(|err| {
        format!(
            "project_settings.config is not valid JSON, so filament_colour cannot be read: {err}"
        )
    })
}

fn physical_palette(root: &Value) -> Result<Palette, String> {
    let config = root.as_object().ok_or_else(|| {
        "project_settings.config must be a JSON object with a filament_colour array".to_string()
    })?;
    let colours = config
        .get("filament_colour")
        .ok_or_else(|| "project_settings.config has no filament_colour array".to_string())?;
    let entries = colours
        .as_array()
        .ok_or_else(|| "filament_colour must be an array of \"#RRGGBB\" strings".to_string())?;
    if entries.is_empty() {
        return Err("filament_colour is empty: a palette needs at least one colour".to_string());
    }

    let mut palette = Vec::with_capacity(entries.len());
    for (index, entry) in entries.iter().enumerate() {
        let hex = entry.as_str().ok_or_else(|| {
            format!("filament_colour[{index}] must be a \"#RRGGBB\" string, got {entry}")
        })?;
        let rgb = parse_hex(hex)
            .ok_or_else(|| format!("filament_colour[{index}] must be #RRGGBB, got {hex:?}"))?;
        palette.push([rgb[0], rgb[1], rgb[2], 255]);
    }
    Ok(Palette(palette))
}

/// The slicer allocates virtual IDs to enabled, non-deleted definitions in
/// file order. Their preview is a weighted swatch of physical filament colours.
fn mixed_preview(row: &str, physical: &[[u8; 4]]) -> Option<[u8; 4]> {
    let fields: Vec<_> = row.split(',').map(str::trim).collect();
    let (a, b, enabled, mix_b) = match fields.as_slice() {
        [a, b, enabled, mix] => (a, b, enabled, mix),
        [a, b, enabled, custom, mix, ..] => {
            custom.parse::<i32>().ok()?;
            (a, b, enabled, mix)
        }
        _ => return None,
    };
    let a = a.parse::<usize>().ok()?;
    let b = b.parse::<usize>().ok()?;
    let mix_b = f64::from(mix_b.parse::<i32>().ok()?.clamp(0, 100)) / 100.0;
    if enabled.parse::<i32>().ok()? == 0 || a == b || a == 0 || b == 0 || a.max(b) > physical.len()
    {
        return None;
    }

    let mut components = "";
    let mut weights = "";
    let mut layer_mode = false;
    let mut gradient = None;
    let mut pattern = Vec::new();
    for (index, &token) in fields.iter().enumerate().skip(5) {
        let Some(prefix) = token.as_bytes().first().map(u8::to_ascii_lowercase) else {
            continue;
        };
        match prefix {
            b'd' if token[1..].parse::<i32>().is_ok_and(|deleted| deleted != 0) => return None,
            b'g' => components = &token[1..],
            b'w' => weights = &token[1..],
            b'm' => layer_mode = matches!(&token[1..], "0" | "1"),
            b'r' => gradient = gradient_fraction(&token[1..]),
            b'1' if index == 5 && token == "1" => (), // Legacy pointillism flag.
            b'1'..=b'9' | b'[' => {
                let group = cycle_components(token, a, b)?;
                if !group.is_empty() {
                    pattern.push(group);
                }
            }
            _ => (),
        }
    }

    let ids = multi_components(components)?;
    if ids
        .iter()
        .chain(pattern.iter().flatten())
        .any(|&id| id > physical.len())
    {
        return None;
    }
    if !pattern.is_empty() {
        // Each cycle group represents a wall and contributes the same weight.
        return weighted_colour(
            physical,
            pattern.into_iter().flat_map(|group| {
                let weight = 1.0 / group.len() as f64;
                group.into_iter().map(move |id| (id, weight))
            }),
        );
    }
    if layer_mode && ids.len() >= 3 {
        let mut weights: Vec<_> = weights
            .split('/')
            .filter_map(|w| w.parse::<u32>().ok())
            .collect();
        if weights.len() != ids.len() || weights.iter().all(|&weight| weight == 0) {
            weights = vec![1; ids.len()];
        }
        return weighted_colour(
            physical,
            ids.into_iter().zip(weights.into_iter().map(f64::from)),
        );
    }

    let fraction_b = gradient.unwrap_or(mix_b);
    weighted_colour(physical, [(a, 1.0 - fraction_b), (b, fraction_b)])
}

fn multi_components(token: &str) -> Option<Vec<usize>> {
    let mut ids = Vec::new();
    if token.contains('/') {
        for id in token.split('/') {
            let id = id.parse().ok()?;
            if id > 0 && !ids.contains(&id) {
                ids.push(id);
            }
        }
    } else {
        for digit in token.bytes().filter(|digit| (b'1'..=b'9').contains(digit)) {
            let id = usize::from(digit - b'0');
            if !ids.contains(&id) {
                ids.push(id);
            }
        }
    }
    Some(ids)
}

fn cycle_components(mut token: &str, a: usize, b: usize) -> Option<Vec<usize>> {
    let mut sequence = Vec::new();
    while let Some(digit) = token.as_bytes().first().copied() {
        match digit {
            b'[' => {
                let (id, rest) = token[1..].split_once(']')?;
                match id.parse().ok()? {
                    0 => (),
                    1 => sequence.push(a),
                    2 => sequence.push(b),
                    id => sequence.push(id),
                }
                token = rest;
                continue;
            }
            b'1' => sequence.push(a),
            b'2' => sequence.push(b),
            b'3'..=b'9' => sequence.push(usize::from(digit - b'0')),
            _ => (),
        }
        token = token.get(token.chars().next()?.len_utf8()..)?;
    }
    Some(sequence)
}

fn gradient_fraction(token: &str) -> Option<f64> {
    let (flag, endpoints) = token.split_once('/')?;
    let (start, end) = endpoints.split_once('/')?;
    let start: f64 = start.parse().ok()?;
    let end: f64 = end.parse().ok()?;
    if flag != "1" || !(0.0..=1.0).contains(&start) || !(0.0..=1.0).contains(&end) {
        return None;
    }
    Some(1.0 - (start + end) * 0.5)
}

fn weighted_colour(
    palette: &[[u8; 4]],
    ingredients: impl IntoIterator<Item = (usize, f64)>,
) -> Option<[u8; 4]> {
    let mut total = 0.0;
    let mut rgb = [0.0; 3];
    for (slot, weight) in ingredients {
        let colour = palette.get(slot.checked_sub(1)?)?;
        for (value, &channel) in rgb.iter_mut().zip(colour) {
            *value += f64::from(channel) * weight;
        }
        total += weight;
    }
    (total > 0.0).then(|| {
        let [r, g, b] = rgb.map(|value| (value / total).round() as u8);
        [r, g, b, 255]
    })
}

/// Resolve one colour per entry of `slots`.
///
/// This is the per-face form: the returned vector has the same length as
/// `slots`. A mesh with no paint information has no slots to walk, so its
/// object-level base colour is resolved with [`resolve_with_base`] instead.
pub fn resolve(slots: &[u8], palette: Option<&Palette>, object_base: Option<u8>) -> Vec<[u8; 4]> {
    resolve_with_base(slots.len(), slots, palette, object_base)
}

/// Resolve `face_count` colours, using `object_base` when there are no slots.
///
/// `slots` is parallel to the faces. An empty `slots` means the mesh carries no
/// `paint_color` at all (not "a mesh with zero faces" — that is
/// `face_count == 0`), and every face then takes the object's base colour:
/// `object_base` is a normalized zero-based palette index. Without `object_base`,
/// the unpainted fallback (palette index 0) applies. A `slots` slice shorter
/// than `face_count` treats the remaining faces as unpainted; nothing panics.
pub fn resolve_with_base(
    face_count: usize,
    slots: &[u8],
    palette: Option<&Palette>,
    object_base: Option<u8>,
) -> Vec<[u8; 4]> {
    let Some(palette) = palette else {
        return vec![DEFAULT_FACE_COLOR; face_count];
    };
    if slots.is_empty() {
        return vec![base_colour(palette, object_base); face_count];
    }
    (0..face_count)
        .map(|index| match slots.get(index).copied().unwrap_or(0) {
            0 => colour_at(palette, 0),
            slot => colour_at(palette, usize::from(slot) - 1),
        })
        .collect()
}

/// The colour of a face with no per-face slot: the object's `extruder`
/// metadata (0-based, so palette index `base`) or the unpainted fallback.
fn base_colour(palette: &Palette, object_base: Option<u8>) -> [u8; 4] {
    match object_base {
        Some(base) => colour_at(palette, usize::from(base)),
        None => colour_at(palette, 0),
    }
}

/// Palette index `index`, clamped to the last colour. An empty palette (which
/// [`palette_from_project_settings`] rejects, but the public tuple struct can
/// still be built) degrades to the default grey instead of indexing.
fn colour_at(palette: &Palette, index: usize) -> [u8; 4] {
    match palette.0.len() {
        0 => DEFAULT_FACE_COLOR,
        len => palette.0[index.min(len - 1)],
    }
}

/// Parse `#RRGGBB` into RGB. Exactly seven bytes, and no indexing path can
/// panic: the length is checked first and the pairs are taken with
/// `chunks_exact`.
fn parse_hex(hex: &str) -> Option<[u8; 3]> {
    let bytes = hex.as_bytes();
    if bytes.len() != 7 || bytes[0] != b'#' {
        return None;
    }
    let mut rgb = [0u8; 3];
    for (channel, pair) in rgb.iter_mut().zip(bytes[1..].chunks_exact(2)) {
        *channel = (hex_digit(pair[0])? << 4) | hex_digit(pair[1])?;
    }
    Some(rgb)
}

/// One hex digit, in either case.
fn hex_digit(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn palette(colours: &[[u8; 4]]) -> Palette {
        Palette(colours.to_vec())
    }

    /// The palette measured in `~/Downloads/梁圣.3mf`.
    const SAMPLE: &str = r##"{"filament_colour":["#FFFFFF","#FFFF00","#00C1AE","#F4E2C1"]}"##;

    #[test]
    fn filament_colour_becomes_a_one_based_palette() {
        let p = palette_from_project_settings(r##"{"filament_colour":["#E03131","#1971C2"]}"##)
            .expect("valid palette");
        assert_eq!(p.0[0], [0xE0, 0x31, 0x31, 255]);
        assert_eq!(p.0[1], [0x19, 0x71, 0xC2, 255]);
    }

    #[test]
    fn the_sample_project_palette_keeps_all_four_colours() {
        let p = palette_from_project_settings(SAMPLE).expect("the measured palette");
        assert_eq!(
            p.0,
            vec![
                [0xFF, 0xFF, 0xFF, 255],
                [0xFF, 0xFF, 0x00, 255],
                [0x00, 0xC1, 0xAE, 255],
                [0xF4, 0xE2, 0xC1, 255],
            ]
        );
    }

    #[test]
    fn lower_case_hex_is_accepted_and_parsed_by_value() {
        let p = palette_from_project_settings(r##"{"filament_colour":["#e03131","#1971c2"]}"##)
            .expect("lower case is still #RRGGBB");
        assert_eq!(p.0, vec![[0xE0, 0x31, 0x31, 255], [0x19, 0x71, 0xC2, 255]]);
    }

    #[test]
    fn slot_zero_and_unpainted_resolve_to_slot_one() {
        // Design ruling: an unpainted face follows slot 1, not the default
        // grey, because the slicer maps "no paint" to extruder 1.
        let p = palette(&[[0xFF, 0xFF, 0xFF, 255], [0xFF, 0xFF, 0x00, 255]]);

        assert_eq!(
            resolve(&[0, 2, 1], Some(&p), None),
            vec![
                [0xFF, 0xFF, 0xFF, 255],
                [0xFF, 0xFF, 0x00, 255],
                [0xFF, 0xFF, 0xFF, 255],
            ]
        );
        assert_ne!(resolve(&[0], Some(&p), None)[0], DEFAULT_FACE_COLOR);
    }

    #[test]
    fn a_slot_beyond_the_palette_clamps_instead_of_panicking() {
        // A file claims 16 slots but the project only provides four colours.
        let p = palette(&[
            [0xAA, 0x00, 0x00, 255],
            [0xBB, 0x00, 0x00, 255],
            [0xCC, 0x00, 0x00, 255],
            [0xDD, 0x00, 0x00, 255],
        ]);

        assert_eq!(
            resolve(&[255, 16, 5], Some(&p), None),
            vec![[0xDD, 0x00, 0x00, 255]; 3]
        );
    }

    #[test]
    fn without_project_settings_every_face_is_the_default_grey() {
        assert_eq!(resolve(&[0, 1, 2], None, None), vec![DEFAULT_FACE_COLOR; 3]);
        // The object base cannot mint a colour out of a missing palette.
        assert_eq!(
            resolve(&[0, 1, 2], None, Some(2)),
            vec![DEFAULT_FACE_COLOR; 3]
        );
        assert_eq!(
            resolve_with_base(2, &[], None, Some(2)),
            vec![DEFAULT_FACE_COLOR; 2]
        );
    }

    #[test]
    fn an_objects_extruder_metadata_colours_its_unpainted_faces() {
        // model_settings has extruder=2 (0-based), so slot 3 = palette[2].
        let p = palette(&[
            [0xFF, 0xFF, 0xFF, 255],
            [0xFF, 0xFF, 0x00, 255],
            [0x00, 0xC1, 0xAE, 255],
        ]);

        // A mesh with no paint_color at all: every face takes the base colour.
        assert_eq!(
            resolve_with_base(2, &[], Some(&p), Some(2)),
            vec![[0x00, 0xC1, 0xAE, 255]; 2]
        );

        // A painted mesh keeps the slot-0 rule; the base fills no faces when
        // per-face paint information exists.
        assert_eq!(
            resolve_with_base(2, &[0, 2], Some(&p), Some(2)),
            vec![[0xFF, 0xFF, 0xFF, 255], [0xFF, 0xFF, 0x00, 255]]
        );
    }

    #[test]
    fn an_out_of_range_object_base_clamps() {
        let p = palette(&[[0x01, 0x01, 0x01, 255], [0x02, 0x02, 0x02, 255]]);
        assert_eq!(
            resolve_with_base(1, &[], Some(&p), Some(200)),
            vec![[0x02, 0x02, 0x02, 255]]
        );
    }

    #[test]
    fn resolve_is_the_one_entry_per_slot_form_of_resolve_with_base() {
        let p = palette(&[[0x10, 0x20, 0x30, 255]]);
        assert_eq!(
            resolve(&[0, 1], Some(&p), None),
            resolve_with_base(2, &[0, 1], Some(&p), None)
        );
        // With no slots there is no face count to fill, so this form is empty;
        // the object base is reached through resolve_with_base.
        assert!(resolve(&[], Some(&p), Some(2)).is_empty());
    }

    #[test]
    fn a_shorter_slots_slice_treats_the_tail_as_unpainted() {
        let p = palette(&[[0x01, 0x01, 0x01, 255], [0x02, 0x02, 0x02, 255]]);
        assert_eq!(
            resolve_with_base(3, &[2], Some(&p), None),
            vec![
                [0x02, 0x02, 0x02, 255],
                [0x01, 0x01, 0x01, 255],
                [0x01, 0x01, 0x01, 255],
            ]
        );
    }

    #[test]
    fn an_engineered_empty_palette_never_panics() {
        // palette_from_project_settings rejects an empty array, but the tuple
        // struct is public and a caller can still build one.
        let empty = Palette(Vec::new());
        assert_eq!(
            resolve(&[0, 1, 2], Some(&empty), None),
            vec![DEFAULT_FACE_COLOR; 3]
        );
        assert_eq!(
            resolve_with_base(2, &[], Some(&empty), Some(2)),
            vec![DEFAULT_FACE_COLOR; 2]
        );
    }

    #[test]
    fn short_or_long_hex_is_rejected_with_a_message_naming_the_key() {
        for bad in [
            r##"{"filament_colour":["#fff"]}"##,
            r##"{"filament_colour":["#E031311"]}"##,
            r##"{"filament_colour":["E03131"]}"##,
            r##"{"filament_colour":["#GGGGGG"]}"##,
        ] {
            let err = palette_from_project_settings(bad).expect_err("malformed hex is an error");
            assert!(
                err.contains("filament_colour"),
                "error must name the key: {err}"
            );
        }
    }

    #[test]
    fn a_missing_filament_colour_is_an_error_naming_the_key() {
        let err = palette_from_project_settings(r##"{"other_key":1}"##)
            .expect_err("a missing key is an error");
        assert!(
            err.contains("filament_colour"),
            "error must name the key: {err}"
        );
    }

    #[test]
    fn an_empty_palette_is_an_error_naming_the_key() {
        let err = palette_from_project_settings(r##"{"filament_colour":[]}"##)
            .expect_err("a zero-slot palette cannot colour anything");
        assert!(
            err.contains("filament_colour"),
            "error must name the key: {err}"
        );
    }

    #[test]
    fn a_non_string_entry_is_an_error_naming_the_key_and_index() {
        let err = palette_from_project_settings(r##"{"filament_colour":["#FFFFFF",7]}"##)
            .expect_err("entries must be strings");
        assert!(
            err.contains("filament_colour[1]"),
            "error must name key and index: {err}"
        );
    }

    #[test]
    fn invalid_json_or_a_non_object_root_is_an_error_not_a_panic() {
        for bad in ["", "not json", "[]", "{\"filament_colour\":\"#FFFFFF\"}"] {
            let err =
                palette_from_project_settings(bad).expect_err("unreadable config is an error");
            assert!(
                err.contains("filament_colour"),
                "error must name the key: {err}"
            );
        }
    }
}
