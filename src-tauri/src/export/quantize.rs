//! Deterministic colour reduction from arbitrary per-face RGBA down to the
//! sixteen extruder slots a 3MF consumer can address.
//!
//! The painting UI lets the user pick any colour, but `paint_color` addresses
//! extruder slots, and the safe ceiling across both slicer trees is sixteen
//! (see `paint_color::MAX_EXTRUDER_SLOT`). Anything beyond that has to be
//! merged before export.
//!
//! Median cut was chosen over k-means specifically because it is
//! deterministic: k-means needs a seeded RNG and converges to different
//! palettes on different runs, which would make the same model export to two
//! different files and make regression diffing useless. Every ordering
//! decision here has an explicit tie-break so the output is a pure function of
//! the input.

/// Result of reducing a face colour array to a slot palette.
pub struct Quantized {
    /// Palette entries, ordered by descending face count. Entry `i` is
    /// extruder slot `i + 1`, so `palette[0]` is the dominant colour and lands
    /// on slot 1.
    pub palette: Vec<[u8; 3]>,
    /// One 1-based slot index per input face, parallel to the input slice.
    pub face_slots: Vec<u8>,
}

/// A working set of colours during the median cut.
struct ColorBox {
    /// `(rgb, face_count)` pairs, always non-empty.
    entries: Vec<([u8; 3], usize)>,
}

impl ColorBox {
    fn total_count(&self) -> usize {
        self.entries.iter().map(|(_, c)| *c).sum()
    }

    /// Length of the longest side of the axis-aligned bounding box, and which
    /// channel that is. Ties resolve to the lowest channel index so the split
    /// is reproducible.
    fn longest_channel(&self) -> (usize, u8) {
        let mut best_channel = 0usize;
        let mut best_extent = 0u8;
        for channel in 0..3 {
            let mut lo = u8::MAX;
            let mut hi = u8::MIN;
            for (rgb, _) in &self.entries {
                lo = lo.min(rgb[channel]);
                hi = hi.max(rgb[channel]);
            }
            let extent = hi - lo;
            if extent > best_extent {
                best_extent = extent;
                best_channel = channel;
            }
        }
        (best_channel, best_extent)
    }

    /// Priority for picking which box to split next: spatial extent weighted
    /// by how many faces sit in the box, so a wide but rarely used box does
    /// not steal a slot from a tight but dominant one.
    fn split_priority(&self) -> u64 {
        if self.entries.len() < 2 {
            return 0;
        }
        let (_, extent) = self.longest_channel();
        u64::from(extent) * self.total_count() as u64
    }

    /// Count-weighted mean, rounded half-up.
    fn representative(&self) -> [u8; 3] {
        let total = self.total_count().max(1) as u64;
        let mut out = [0u8; 3];
        for channel in 0..3 {
            let sum: u64 = self
                .entries
                .iter()
                .map(|(rgb, count)| u64::from(rgb[channel]) * *count as u64)
                .sum();
            out[channel] = ((sum * 2 + total) / (total * 2)).min(255) as u8;
        }
        out
    }
}

/// Reduce `face_colors` to at most `max_slots` palette entries.
///
/// Alpha is discarded: `paint_color` has no alpha channel and the slicer
/// resolves the actual RGB from `filament_colour` anyway.
///
/// An empty input yields an empty palette and no slots. `max_slots` is clamped
/// to at least 1.
pub fn quantize_face_colors(face_colors: &[[u8; 4]], max_slots: usize) -> Quantized {
    let max_slots = max_slots.max(1);

    if face_colors.is_empty() {
        return Quantized {
            palette: Vec::new(),
            face_slots: Vec::new(),
        };
    }

    // Histogram over unique RGB. BTreeMap rather than HashMap so iteration
    // order is the colour's lexicographic order rather than a hash seed.
    let mut histogram: std::collections::BTreeMap<[u8; 3], usize> = std::collections::BTreeMap::new();
    for rgba in face_colors {
        *histogram.entry([rgba[0], rgba[1], rgba[2]]).or_insert(0) += 1;
    }

    let entries: Vec<([u8; 3], usize)> = histogram.into_iter().collect();

    let boxes: Vec<ColorBox> = if entries.len() <= max_slots {
        // Everything already fits; one box per colour keeps the originals exact.
        entries
            .into_iter()
            .map(|e| ColorBox { entries: vec![e] })
            .collect()
    } else {
        median_cut(entries, max_slots)
    };

    // Dominant colour first so it lands on slot 1. Tie-break on the
    // representative colour to stay deterministic when counts are equal.
    let mut ranked: Vec<([u8; 3], usize)> = boxes
        .iter()
        .map(|b| (b.representative(), b.total_count()))
        .collect();
    ranked.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));

    let palette: Vec<[u8; 3]> = ranked.into_iter().map(|(rgb, _)| rgb).collect();

    // Map each distinct input colour once, then fan out to faces.
    let mut assignment: std::collections::BTreeMap<[u8; 3], u8> = std::collections::BTreeMap::new();
    let face_slots = face_colors
        .iter()
        .map(|rgba| {
            let rgb = [rgba[0], rgba[1], rgba[2]];
            *assignment
                .entry(rgb)
                .or_insert_with(|| nearest_slot(&palette, rgb))
        })
        .collect();

    Quantized {
        palette,
        face_slots,
    }
}

/// Index of the closest palette entry by squared euclidean distance in RGB,
/// returned as a 1-based slot. Ties go to the lower slot.
fn nearest_slot(palette: &[[u8; 3]], rgb: [u8; 3]) -> u8 {
    let mut best_index = 0usize;
    let mut best_distance = u32::MAX;
    for (index, entry) in palette.iter().enumerate() {
        let dr = i32::from(entry[0]) - i32::from(rgb[0]);
        let dg = i32::from(entry[1]) - i32::from(rgb[1]);
        let db = i32::from(entry[2]) - i32::from(rgb[2]);
        let distance = (dr * dr + dg * dg + db * db) as u32;
        if distance < best_distance {
            best_distance = distance;
            best_index = index;
        }
    }
    (best_index + 1) as u8
}

fn median_cut(entries: Vec<([u8; 3], usize)>, max_slots: usize) -> Vec<ColorBox> {
    let mut boxes = vec![ColorBox { entries }];

    while boxes.len() < max_slots {
        // Pick the highest-priority splittable box; ties go to the lowest
        // index so the traversal order never depends on allocator behaviour.
        let mut target: Option<usize> = None;
        let mut best_priority = 0u64;
        for (index, b) in boxes.iter().enumerate() {
            let priority = b.split_priority();
            if priority > best_priority {
                best_priority = priority;
                target = Some(index);
            }
        }

        let Some(target) = target else {
            // Every remaining box holds a single colour; nothing left to split.
            break;
        };

        let victim = boxes.swap_remove(target);
        let (left, right) = split_box(victim);
        boxes.push(left);
        boxes.push(right);
    }

    boxes
}

/// Split along the longest channel at the count-weighted median, guaranteeing
/// both halves are non-empty.
fn split_box(mut victim: ColorBox) -> (ColorBox, ColorBox) {
    let (channel, _) = victim.longest_channel();

    // Sort on the split channel, falling back to the full colour so equal
    // channel values still have a total order.
    victim
        .entries
        .sort_by(|a, b| a.0[channel].cmp(&b.0[channel]).then(a.0.cmp(&b.0)));

    let total: usize = victim.entries.iter().map(|(_, c)| *c).sum();
    let half = total / 2;

    let mut running = 0usize;
    let mut cut = 0usize;
    for (index, (_, count)) in victim.entries.iter().enumerate() {
        running += count;
        if running > half {
            cut = index + 1;
            break;
        }
    }
    // Both halves must be non-empty, otherwise the loop above would spin.
    let cut = cut.clamp(1, victim.entries.len() - 1);

    let right = victim.entries.split_off(cut);
    (ColorBox { entries: victim.entries }, ColorBox { entries: right })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_input_yields_empty_output() {
        let q = quantize_face_colors(&[], 16);
        assert!(q.palette.is_empty());
        assert!(q.face_slots.is_empty());
    }

    #[test]
    fn colours_below_the_ceiling_are_preserved_exactly() {
        let faces = vec![
            [255, 0, 0, 255],
            [0, 255, 0, 255],
            [0, 0, 255, 255],
            [255, 0, 0, 255],
        ];
        let q = quantize_face_colors(&faces, 16);
        assert_eq!(q.palette.len(), 3);
        // Red appears twice so it must own slot 1.
        assert_eq!(q.palette[0], [255, 0, 0]);
        assert_eq!(q.face_slots[0], 1);
        assert_eq!(q.face_slots[3], 1);
        // Every face maps back to its own colour, unchanged.
        for (face, slot) in faces.iter().zip(&q.face_slots) {
            let entry = q.palette[usize::from(*slot) - 1];
            assert_eq!(entry, [face[0], face[1], face[2]]);
        }
    }

    #[test]
    fn never_exceeds_the_slot_ceiling() {
        // 64 well separated colours squeezed into 16 slots.
        let faces: Vec<[u8; 4]> = (0..64u8)
            .map(|i| [i.wrapping_mul(4), 255 - i.wrapping_mul(3), i.wrapping_mul(2), 255])
            .collect();
        let q = quantize_face_colors(&faces, 16);
        assert_eq!(q.palette.len(), 16);
        assert_eq!(q.face_slots.len(), 64);
        assert!(q.face_slots.iter().all(|s| *s >= 1 && *s <= 16));
    }

    #[test]
    fn output_is_deterministic_across_runs() {
        let faces: Vec<[u8; 4]> = (0..200u32)
            .map(|i| {
                [
                    (i * 37 % 256) as u8,
                    (i * 91 % 256) as u8,
                    (i * 173 % 256) as u8,
                    255,
                ]
            })
            .collect();
        let a = quantize_face_colors(&faces, 16);
        let b = quantize_face_colors(&faces, 16);
        assert_eq!(a.palette, b.palette);
        assert_eq!(a.face_slots, b.face_slots);
    }

    #[test]
    fn a_single_colour_collapses_to_one_slot() {
        let faces = vec![[138, 138, 138, 255]; 50];
        let q = quantize_face_colors(&faces, 16);
        assert_eq!(q.palette, vec![[138, 138, 138]]);
        assert!(q.face_slots.iter().all(|s| *s == 1));
    }

    #[test]
    fn alpha_is_ignored_when_grouping() {
        let faces = vec![[10, 20, 30, 255], [10, 20, 30, 0]];
        let q = quantize_face_colors(&faces, 16);
        assert_eq!(q.palette.len(), 1);
        assert_eq!(q.face_slots, vec![1, 1]);
    }
}
