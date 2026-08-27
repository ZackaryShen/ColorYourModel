//! Encoder for the `paint_color` triangle attribute consumed by
//! BambuStudio / OrcaSlicer / Snapmaker Orca.
//!
//! This is the only per-face colour channel those slicers actually read. The
//! 3MF core `basematerials` / `displaycolor` resource has zero readers in the
//! entire `libslic3r/Format/` directory of either tree that was audited
//! (upstream stock OrcaSlicer 2.5.0-dev and the Snapmaker 2.3.3 fork), so
//! emitting it has no effect on the rendered colours.
//!
//! The attribute does not carry RGB. It carries a 1-based *extruder slot
//! index*, bit-packed exactly the way `TriangleSelector::serialize()` does it
//! in libslic3r. For a leaf triangle (the whole face painted, no subdivision)
//! the layout is:
//!
//!   - 2 bits: `split_sides`, always 0 because we never subdivide
//!   - state:  0..2 are written as two raw bits; 3 and above write the escape
//!             code `0b11` followed by `state - 3` in 4-bit little-endian
//!             chunks, where a chunk of `0b1111` means another chunk follows
//!
//! Bits are little-endian inside each nibble, and `Model.cpp` *prepends* every
//! finished nibble to the output string, which reverses the nibble order. The
//! unit tests below pin the six values that were read straight out of both
//! source trees.
//!
//! Hex digits must be upper case: the release build of the slicer parses the
//! string with a table that only covers `0-9A-F` and silently yields 0 for
//! lower case input, which would repaint the face with extruder 0.

/// Highest extruder slot we are willing to emit.
///
/// Upstream stock caps `EnforcerBlockerType::ExtruderMax` at `Extruder16`
/// (an `int8_t`, the header comment spells out "Maximum is 15 ... 2 bit prefix
/// code"), while the Snapmaker fork widens the same enum to 255 on an
/// `int16_t`. Sixteen is the intersection, so a file produced here loads
/// correctly in both.
pub const MAX_EXTRUDER_SLOT: u8 = 16;

/// Encode a 1-based extruder slot into a `paint_color` attribute value.
///
/// Returns `None` for slot 0 (which means "unpainted" and must be written as
/// an absent attribute, not an empty string) and for slots above
/// [`MAX_EXTRUDER_SLOT`].
pub fn encode_paint_color(slot: u8) -> Option<String> {
    if slot == 0 || slot > MAX_EXTRUDER_SLOT {
        return None;
    }

    let mut bits: Vec<u8> = Vec::with_capacity(8);

    // split_sides == 0: this is a leaf triangle, the whole face is one colour.
    push_bits(&mut bits, 0, 2);

    let state = u32::from(slot);
    if state < 3 {
        push_bits(&mut bits, state, 2);
    } else {
        push_bits(&mut bits, 0b11, 2);
        let mut remainder = state - 3;
        loop {
            if remainder < 0b1111 {
                push_bits(&mut bits, remainder, 4);
                break;
            }
            push_bits(&mut bits, 0b1111, 4);
            remainder -= 0b1111;
        }
    }

    // Pad out to a whole nibble so the reader never sees a partial chunk.
    while bits.len() % 4 != 0 {
        bits.push(0);
    }

    let mut nibbles: Vec<u8> = bits
        .chunks(4)
        .map(|c| c[0] | (c[1] << 1) | (c[2] << 2) | (c[3] << 3))
        .collect();
    // Model.cpp prepends, so the on-disk order is the reverse of emission order.
    nibbles.reverse();

    Some(
        nibbles
            .iter()
            .map(|n| format!("{:X}", n))
            .collect::<String>(),
    )
}

fn push_bits(bits: &mut Vec<u8>, value: u32, count: usize) {
    for i in 0..count {
        bits.push(((value >> i) & 1) as u8);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The six values transcribed from `TriangleSelector.cpp` in both trees.
    /// If any of these drift, the exported file will paint the wrong extruder.
    #[test]
    fn golden_values_match_libslic3r() {
        assert_eq!(encode_paint_color(1).as_deref(), Some("4"));
        assert_eq!(encode_paint_color(2).as_deref(), Some("8"));
        assert_eq!(encode_paint_color(3).as_deref(), Some("0C"));
        assert_eq!(encode_paint_color(4).as_deref(), Some("1C"));
        assert_eq!(encode_paint_color(5).as_deref(), Some("2C"));
        assert_eq!(encode_paint_color(16).as_deref(), Some("DC"));
    }

    #[test]
    fn slot_zero_is_unpainted_and_has_no_encoding() {
        assert_eq!(encode_paint_color(0), None);
    }

    #[test]
    fn slots_above_the_intersection_ceiling_are_rejected() {
        assert_eq!(encode_paint_color(17), None);
        assert_eq!(encode_paint_color(255), None);
    }

    #[test]
    fn every_valid_slot_encodes_and_uses_upper_case_hex() {
        for slot in 1..=MAX_EXTRUDER_SLOT {
            let encoded = encode_paint_color(slot)
                .unwrap_or_else(|| panic!("slot {} failed to encode", slot));
            assert!(!encoded.is_empty(), "slot {} produced an empty string", slot);
            assert!(
                encoded.chars().all(|c| c.is_ascii_digit() || ('A'..='F').contains(&c)),
                "slot {} produced non-upper-case hex: {}",
                slot,
                encoded
            );
        }
    }

    #[test]
    fn encoding_is_injective_across_the_whole_range() {
        let mut seen = std::collections::BTreeSet::new();
        for slot in 1..=MAX_EXTRUDER_SLOT {
            let encoded = encode_paint_color(slot).unwrap();
            assert!(
                seen.insert(encoded.clone()),
                "slot {} collided with an earlier slot on {}",
                slot,
                encoded
            );
        }
    }
}
