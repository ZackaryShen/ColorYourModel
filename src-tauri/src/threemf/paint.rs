//! Decoder for the `paint_color` triangle attribute written by OrcaSlicer,
//! BambuStudio and Snapmaker Orca.
//!
//! The attribute is not an RGB value: it is a 1-based extruder slot, bit-packed
//! exactly like `TriangleSelector::serialize()` / `deserialize()` in libslic3r
//! and turned into hex by `FacetsAnnotation::get_triangle_as_string()` /
//! `set_triangle_from_string()` (`src/libslic3r/Model.cpp`).
//!
//! Bit-stream conventions (verified against the upstream source and an
//! 8,693-distinct-value real file):
//!
//! - The hex digits are stored in **reverse** order: the writer prepends every
//!   finished nibble, so the reader reverses the string first.
//! - Inside a nibble the bits are **little-endian**: bit 0 of the 4-bit `code`
//!   is the lowest bit of the hex digit.
//! - Every node is one nibble. `code & 0b11` is the number of split sides
//!   (0 = leaf, 1..=3 = subdivided into `split + 1` children).
//! - A split node stores the special side in the upper two bits (the split
//!   side for a 1-side split, the kept side for a 2-side split; ignored for a
//!   3-side split). It never uses the leaf escape.
//! - A leaf stores its state in the upper two bits. State 3 and above use the
//!   `0b11` escape followed by one nibble of `state - 3`; a nibble of `0b1111`
//!   means another nibble follows with `state - 18`.
//! - Children are written from slot `split` down to slot 0; [`decode`]
//!   reverses them so `children[i]` is slot `i`.
//! - All nodes and escape chunks are whole nibbles, so a well-formed value
//!   always consumes every nibble; anything left over is malformed.

/// One node of the painter's subdivision tree.
///
/// A leaf has `split == 0`, `children.is_empty()` and its extruder slot in
/// `state` (0 means unpainted). A subdivided triangle has `split` in 1..=3,
/// `split + 1` children in slot order, and `state == 0`.
#[derive(Debug, Clone, PartialEq)]
pub struct PaintNode {
    pub split: u8,
    pub special_side: u8,
    pub state: u8,
    pub children: Vec<PaintNode>,
}

/// Hard ceiling on tree nesting, matching the subdivision rebuild's limit.
/// Real files peak at 10 levels; the bound keeps every recursive walk over the
/// decoded tree (including `Drop`) off the stack limit for attacker-controlled
/// input.
const MAX_DEPTH: u8 = 64;

/// Decode a `paint_color` attribute value into its subdivision tree.
///
/// Returns `Err` for empty or non-hex input, values that end before the tree
/// is complete, values with nibbles left over after it is complete, trees
/// nested deeper than [`MAX_DEPTH`], and partial splits without a valid
/// special side.
pub fn decode(hex: &str) -> Result<PaintNode, String> {
    let mut nibbles = Vec::with_capacity(hex.len());
    for c in hex.chars().rev() {
        match c.to_digit(16) {
            Some(digit) => nibbles.push(digit as u8),
            None => {
                return Err(format!(
                    "paint_color {hex:?} contains a non-hex character {c:?}"
                ))
            }
        }
    }
    if nibbles.is_empty() {
        return Err("paint_color is empty".to_string());
    }

    let mut reader = Reader {
        nibbles: &nibbles,
        pos: 0,
        hex,
    };
    let root = decode_node(&mut reader, 1)?;
    if reader.pos != nibbles.len() {
        return Err(format!(
            "paint_color {hex:?} has {} trailing nibble(s) after the tree",
            nibbles.len() - reader.pos
        ));
    }
    Ok(root)
}

/// The leaf states of a tree in slot order, depth first.
pub fn leaf_slots(node: &PaintNode, out: &mut Vec<u8>) {
    if node.split == 0 {
        out.push(node.state);
    } else {
        for child in &node.children {
            leaf_slots(child, out);
        }
    }
}

/// How many leaf triangles the tree expands to.
pub fn leaf_count(node: &PaintNode) -> usize {
    if node.split == 0 {
        1
    } else {
        node.children.iter().map(leaf_count).sum()
    }
}

struct Reader<'a> {
    nibbles: &'a [u8],
    pos: usize,
    hex: &'a str,
}

impl Reader<'_> {
    fn next_nibble(&mut self) -> Result<u8, String> {
        let Some(&nibble) = self.nibbles.get(self.pos) else {
            return Err(format!(
                "paint_color {:?} ends before the tree is complete",
                self.hex
            ));
        };
        self.pos += 1;
        Ok(nibble)
    }
}

fn decode_node(reader: &mut Reader<'_>, depth: u8) -> Result<PaintNode, String> {
    if depth > MAX_DEPTH {
        return Err(format!(
            "paint_color {:?} nests deeper than {MAX_DEPTH} levels",
            reader.hex
        ));
    }

    let code = reader.next_nibble()?;
    let split = code & 0b11;

    if split == 0 {
        let mut state = code >> 2;
        if state == 0b11 {
            let chunk = reader.next_nibble()?;
            state = if chunk == 0b1111 {
                reader.next_nibble()? + 18
            } else {
                chunk + 3
            };
        }
        return Ok(PaintNode {
            split: 0,
            special_side: 0,
            state,
            children: Vec::new(),
        });
    }

    // The upper two bits are the special side; 3 is not a side, so upstream
    // `split_code_valid()` treats it as corruption on a 1- or 2-side split.
    let special_side = if split == 3 { 0 } else { code >> 2 };
    if split != 3 && special_side == 3 {
        return Err(format!(
            "paint_color {:?} has a split node without a valid special side",
            reader.hex
        ));
    }

    let mut children = Vec::with_capacity(usize::from(split) + 1);
    for _ in 0..=split {
        children.push(decode_node(reader, depth + 1)?);
    }
    // The stream stores children from slot `split` down to slot 0.
    children.reverse();
    Ok(PaintNode {
        split,
        special_side,
        state: 0,
        children,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn golden_leaf_values_match_the_slicer_table() {
        // Source of truth: Slic3r::CONST_FILAMENTS in Model.cpp.
        for (slot, hex) in [(1u8, "4"), (2, "8"), (3, "0C"), (4, "1C"), (5, "2C"), (16, "DC")] {
            let n = decode(hex).expect(hex);
            assert_eq!((n.split, n.state), (0, slot), "slot {slot}");
            assert!(n.children.is_empty());
            assert_eq!(leaf_count(&n), 1);
        }
    }

    #[test]
    fn decodes_a_real_subdivided_value_bit_exactly() {
        // Taken verbatim from ~/Downloads/梁圣.3mf (Snapmaker Orca 2.4.1); the
        // value occurs on five triangles of that file.
        let n = decode("000800303").expect("real value");
        assert_eq!((n.split, n.special_side), (3, 0));
        assert_eq!(n.children.len(), 4);
        // The plan text estimated 6 leaves for this value; the tree actually
        // carries 7 (root 3-side split + one nested 3-side split + 3 leaves).
        assert_eq!(leaf_count(&n), 7);
        let mut slots = Vec::new();
        leaf_slots(&n, &mut slots);
        assert_eq!(slots.iter().filter(|s| **s == 2).count(), 1);
        assert_eq!(slots.iter().filter(|s| **s == 0).count(), 6);
    }

    #[test]
    fn children_are_returned_in_slot_order_not_serialized_order() {
        // TriangleSelector::serialize writes children from split_sides down to 0
        // (reverse order, for PrusaSlicer 2.3.1 compatibility). decode() reverses it.
        let n = decode("88880883883883883").expect("real value");
        assert_eq!(n.children.len(), 4);
        assert_eq!(n.children[0].split, 0);
        assert_eq!(n.children[1].split, 3);
        assert_eq!(n.children[2].split, 0);
        assert_eq!(n.children[3].split, 0);
    }

    #[test]
    fn malformed_input_is_an_error_not_a_panic() {
        assert!(decode("").is_err());
        assert!(decode("ZZ").is_err());          // not hex
        assert!(decode("0C").is_err() == false); // 3 -> escape -> one nibble follows
        assert!(decode("3").is_err());           // split node whose children run past the end
        assert!(decode("C").is_err());           // escape with no following nibble
        assert!(decode("44").is_err());          // bits left over after the tree is complete
        assert!(decode("D").is_err());           // 1-side split can never keep side 3
    }

    #[test]
    fn the_depth_cap_keeps_pathological_values_bounded() {
        // A chain of 1-side splits: 63 splits then 64 leaves nests 64 deep.
        let at_cap = format!("{}{}", "0".repeat(64), "1".repeat(63));
        assert_eq!(leaf_count(&decode(&at_cap).expect("depth 64")), 64);
        let past_cap = format!("{}{}", "0".repeat(65), "1".repeat(64));
        assert!(decode(&past_cap).is_err());
    }

    #[test]
    #[ignore = "needs CYM_REAL_3MF (a real Orca-saved 3mf); skipped when unset"]
    fn every_paint_color_in_a_real_file_decodes_with_no_leftover_bits() {
        let Ok(path) = std::env::var("CYM_REAL_3MF") else { return };
        let mut seen = std::collections::BTreeSet::new();
        for hex in crate::threemf::test_support::paint_colors_in(std::path::Path::new(&path)) {
            if seen.insert(hex.clone()) {
                decode(&hex).unwrap_or_else(|e| panic!("{hex}: {e}"));
            }
        }
        assert!(seen.len() > 1000, "expected thousands of distinct values, got {}", seen.len());
    }
}
