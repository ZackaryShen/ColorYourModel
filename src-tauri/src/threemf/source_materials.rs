//! Original native assignments for CYM engineering saves.

use crate::export::project_state::Fingerprint;
use crate::mesh::model::MeshModel;

const MAGIC: &[u8; 8] = b"CYMSRC\x01\0";
const NONE: [u8; 4] = [0; 4];

pub(super) fn part_name(plate_index: usize) -> String {
    format!("Metadata/cym/source_materials_{}.bin", plate_index + 1)
}

pub(crate) fn source_materials_entry(
    plate_index: usize,
    mesh: &MeshModel,
    fingerprint: Fingerprint,
) -> Option<(String, Vec<u8>)> {
    let colors = &mesh.source_material_colors;
    let slots = &mesh.source_material_slots;
    let color_count = u8::try_from(colors.len()).ok()?;
    if slots.is_empty()
        || slots.len() != fingerprint.face_count as usize
        || color_count == 0
        || !valid_slots(slots, colors)
    {
        return None;
    }
    let geometry = fingerprint_bytes(fingerprint);
    let mut bytes =
        Vec::with_capacity(MAGIC.len() + geometry.len() + 1 + colors.len() * 4 + slots.len());
    bytes.extend_from_slice(MAGIC);
    bytes.extend_from_slice(&geometry);
    bytes.push(color_count);
    for &color in colors {
        bytes.extend_from_slice(&color.unwrap_or(NONE));
    }
    bytes.extend_from_slice(slots);
    Some((part_name(plate_index), bytes))
}

pub(super) fn restore(
    bytes: &[u8],
    mesh: &mut MeshModel,
    fingerprint: Fingerprint,
) -> Result<(), &'static str> {
    let Some((&count, body)) = bytes
        .strip_prefix(MAGIC)
        .and_then(|bytes| bytes.strip_prefix(fingerprint_bytes(fingerprint).as_slice()))
        .and_then(<[u8]>::split_first)
    else {
        return Err("source material version or geometry mismatch");
    };
    let color_bytes = usize::from(count) * 4;
    if count == 0 || body.len() != color_bytes + mesh.faces.len() {
        return Err("invalid source material counts");
    }
    let (color_bytes, slots) = body.split_at(color_bytes);
    let colors: Vec<_> = color_bytes
        .chunks_exact(4)
        .map(|rgba| match rgba {
            [r, g, b, 255] => Ok(Some([*r, *g, *b, 255])),
            [0, 0, 0, 0] => Ok(None),
            _ => Err("invalid source material swatch"),
        })
        .collect::<Result<_, _>>()?;
    if !valid_slots(slots, &colors) {
        return Err("invalid source material slot");
    }
    mesh.source_material_slots = slots.to_vec();
    mesh.source_material_colors = colors;
    Ok(())
}

fn valid_slots(slots: &[u8], colors: &[Option<[u8; 4]>]) -> bool {
    slots.iter().all(|&slot| {
        slot == 0
            || colors
                .get(usize::from(slot - 1))
                .is_some_and(Option::is_some)
    })
}

fn fingerprint_bytes(fingerprint: Fingerprint) -> [u8; 16] {
    let mut result = [0; 16];
    result[..4].copy_from_slice(&fingerprint.vertex_count.to_le_bytes());
    result[4..8].copy_from_slice(&fingerprint.face_count.to_le_bytes());
    result[8..].copy_from_slice(&fingerprint.hash.to_le_bytes());
    result
}
