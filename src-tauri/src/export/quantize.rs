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
struct ColorBox<const CHANNELS: usize> {
    /// `(color, face_count)` pairs, always non-empty.
    entries: Vec<([u8; CHANNELS], usize)>,
}

impl<const CHANNELS: usize> ColorBox<CHANNELS> {
    fn total_count(&self) -> usize {
        self.entries.iter().map(|(_, c)| *c).sum()
    }

    /// Length of the longest side of the axis-aligned bounding box, and which
    /// channel that is. Ties resolve to the lowest channel index so the split
    /// is reproducible.
    fn longest_channel(&self) -> (usize, u8) {
        let mut best_channel = 0usize;
        let mut best_extent = 0u8;
        for channel in 0..CHANNELS {
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
    fn representative(&self) -> [u8; CHANNELS] {
        let total = self.total_count().max(1) as u64;
        std::array::from_fn(|channel| {
            let sum: u64 = self
                .entries
                .iter()
                .map(|(rgb, count)| u64::from(rgb[channel]) * *count as u64)
                .sum();
            ((sum * 2 + total) / (total * 2)).min(255) as u8
        })
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
    let mut histogram: std::collections::BTreeMap<[u8; 3], usize> =
        std::collections::BTreeMap::new();
    for rgba in face_colors {
        *histogram.entry([rgba[0], rgba[1], rgba[2]]).or_insert(0) += 1;
    }

    let entries: Vec<_> = histogram.into_iter().collect();
    let palette = quantize_histogram(&entries, max_slots);

    let face_slots = map_colors_to_palette(face_colors, &palette);

    Quantized {
        palette,
        face_slots,
    }
}

/// Assign real faces to an already chosen joint palette.
pub fn map_colors_to_palette(face_colors: &[[u8; 4]], palette: &[[u8; 3]]) -> Vec<u8> {
    let mut assignment: std::collections::BTreeMap<[u8; 3], u8> = std::collections::BTreeMap::new();
    face_colors
        .iter()
        .map(|rgba| {
            let rgb = [rgba[0], rgba[1], rgba[2]];
            *assignment
                .entry(rgb)
                .or_insert_with(|| nearest_slot(palette, rgb))
        })
        .collect()
}

/// Per-tab preview cache budget from the approved multi-plate design.
pub const MAX_HISTOGRAM_COLORS: usize = 4096;

/// Quantise weighted colours directly; counts are never expanded into faces.
pub fn quantize_histogram<const CHANNELS: usize>(
    histogram: &[([u8; CHANNELS], usize)],
    max_slots: usize,
) -> Vec<[u8; CHANNELS]> {
    let max_slots = max_slots.max(1);
    let mut merged = std::collections::BTreeMap::new();
    for &(rgb, count) in histogram {
        if count > 0 {
            *merged.entry(rgb).or_insert(0usize) += count;
        }
    }
    let entries: Vec<_> = merged.into_iter().collect();

    let boxes: Vec<_> = if entries.len() <= max_slots {
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
    let mut ranked: Vec<_> = boxes
        .iter()
        .map(|b| (b.representative(), b.total_count()))
        .collect();
    ranked.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));

    ranked.into_iter().map(|(rgb, _)| rgb).collect()
}

/// Keep the most frequent colours and merge rare colours into their nearest RGB.
pub fn bounded_color_histogram(colors: &[[u8; 4]], face_count: usize) -> Vec<([u8; 3], usize)> {
    color_histogram_with_limit(colors, face_count, MAX_HISTOGRAM_COLORS)
}

pub(crate) fn color_histogram_with_limit(
    colors: &[[u8; 4]],
    face_count: usize,
    limit: usize,
) -> Vec<([u8; 3], usize)> {
    if colors.is_empty() {
        return if face_count == 0 {
            vec![]
        } else {
            vec![([200, 200, 200], face_count)]
        };
    }
    let mut counts = std::collections::BTreeMap::new();
    for color in colors {
        *counts
            .entry([color[0], color[1], color[2]])
            .or_insert(0usize) += 1;
    }
    bound_color_counts(counts, limit)
}

pub(crate) fn bound_color_counts(
    counts: std::collections::BTreeMap<[u8; 3], usize>,
    limit: usize,
) -> Vec<([u8; 3], usize)> {
    let mut entries: Vec<_> = counts.into_iter().collect();
    if entries.len() <= limit {
        return entries;
    }
    entries.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    let rare = entries.split_off(limit);
    let palette: Vec<_> = entries.iter().map(|(rgb, _)| *rgb).collect();
    let tree = ColourTree::new(&palette);
    for (rgb, count) in rare {
        entries[tree.nearest(rgb)].1 += count;
    }
    entries
}

// Balanced construction tolerates entire RGB planes/lines without bucket limits.
struct ColourNode {
    point: usize,
    axis: usize,
    left: Option<usize>,
    right: Option<usize>,
}
struct ColourTree<'a> {
    points: &'a [[u8; 3]],
    nodes: Vec<ColourNode>,
    root: Option<usize>,
}
impl<'a> ColourTree<'a> {
    fn new(points: &'a [[u8; 3]]) -> Self {
        fn build(
            indices: &mut [usize],
            points: &[[u8; 3]],
            nodes: &mut Vec<ColourNode>,
        ) -> Option<usize> {
            if indices.is_empty() {
                return None;
            }
            let axis = (0..3)
                .max_by_key(|&axis| {
                    let lo = indices.iter().map(|&i| points[i][axis]).min().unwrap();
                    let hi = indices.iter().map(|&i| points[i][axis]).max().unwrap();
                    hi - lo
                })
                .unwrap();
            indices
                .sort_unstable_by(|&a, &b| points[a][axis].cmp(&points[b][axis]).then(a.cmp(&b)));
            let middle = indices.len() / 2;
            let point = indices[middle];
            let node = nodes.len();
            nodes.push(ColourNode {
                point,
                axis,
                left: None,
                right: None,
            });
            let (left, right) = indices.split_at_mut(middle);
            let left = build(left, points, nodes);
            let right = build(&mut right[1..], points, nodes);
            nodes[node].left = left;
            nodes[node].right = right;
            Some(node)
        }
        let mut nodes = Vec::with_capacity(points.len());
        let root = build(
            &mut (0..points.len()).collect::<Vec<_>>(),
            points,
            &mut nodes,
        );
        Self {
            points,
            nodes,
            root,
        }
    }
    fn nearest(&self, rgb: [u8; 3]) -> usize {
        fn visit(
            tree: &ColourTree<'_>,
            node: Option<usize>,
            rgb: [u8; 3],
            best: &mut (usize, u32),
        ) {
            let Some(node) = node else {
                return;
            };
            let node = &tree.nodes[node];
            let point = tree.points[node.point];
            let distance = (0..3)
                .map(|axis| {
                    let d = rgb[axis] as i32 - point[axis] as i32;
                    (d * d) as u32
                })
                .sum();
            if distance < best.1 || (distance == best.1 && node.point < best.0) {
                *best = (node.point, distance);
            }
            let delta = rgb[node.axis] as i32 - point[node.axis] as i32;
            let (near, far) = if delta < 0 {
                (node.left, node.right)
            } else {
                (node.right, node.left)
            };
            visit(tree, near, rgb, best);
            if (delta * delta) as u32 <= best.1 {
                visit(tree, far, rgb, best);
            }
        }
        let mut best = (0, u32::MAX);
        visit(self, self.root, rgb, &mut best);
        best.0
    }
}

/// Index of the closest palette entry by squared euclidean distance in RGB,
/// returned as a 1-based slot. Ties go to the lower slot.
fn nearest_slot(palette: &[[u8; 3]], rgb: [u8; 3]) -> u8 {
    (nearest_palette_index(palette, rgb) + 1) as u8
}

/// Zero-based index without the slicer's 1-based u8 slot limit (OBJ uses 256 materials).
pub fn nearest_palette_index<const CHANNELS: usize>(
    palette: &[[u8; CHANNELS]],
    color: [u8; CHANNELS],
) -> usize {
    let mut best_index = 0usize;
    let mut best_distance = u32::MAX;
    for (index, entry) in palette.iter().enumerate() {
        let distance: u32 = entry.iter().zip(color)
            .map(|(&a, b)| (i32::from(a) - i32::from(b)).pow(2) as u32)
            .sum();
        if distance < best_distance {
            best_distance = distance;
            best_index = index;
        }
    }
    best_index
}

fn median_cut<const CHANNELS: usize>(
    entries: Vec<([u8; CHANNELS], usize)>,
    max_slots: usize,
) -> Vec<ColorBox<CHANNELS>> {
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
fn split_box<const CHANNELS: usize>(
    mut victim: ColorBox<CHANNELS>,
) -> (ColorBox<CHANNELS>, ColorBox<CHANNELS>) {
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
    (
        ColorBox {
            entries: victim.entries,
        },
        ColorBox { entries: right },
    )
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
            .map(|i| {
                [
                    i.wrapping_mul(4),
                    255 - i.wrapping_mul(3),
                    i.wrapping_mul(2),
                    255,
                ]
            })
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

#[cfg(test)]
mod histogram_tests {
    use super::*;
    #[test]
    fn weighted_palette_matches_flattened_faces_without_expanding_counts() {
        let faces: Vec<_> = (0..2000u32)
            .map(|i| [(i % 120) as u8, (i % 71) as u8, (i % 37) as u8, 255])
            .collect();
        let histogram = bounded_color_histogram(&faces, faces.len());
        assert_eq!(
            quantize_histogram(&histogram, 16),
            quantize_face_colors(&faces, 16).palette
        );
        assert_eq!(
            histogram.iter().map(|(_, count)| count).sum::<usize>(),
            faces.len()
        );
    }
    #[test]
    fn cached_histogram_is_bounded_and_preserves_all_face_weight() {
        let mut faces: Vec<_> = (0..5000u32)
            .map(|i| [(i % 256) as u8, (i / 256) as u8, 31, 255])
            .collect();
        faces.extend(std::iter::repeat([250, 250, 250, 255]).take(200));
        let histogram = bounded_color_histogram(&faces, faces.len());
        assert_eq!(histogram.len(), MAX_HISTOGRAM_COLORS);
        assert_eq!(
            histogram.iter().map(|(_, count)| count).sum::<usize>(),
            faces.len()
        );
        assert!(histogram.iter().any(|(rgb, _)| rgb == &[250, 250, 250]));
    }
}

#[cfg(test)]
mod colour_tree_tests {
    use super::*;
    #[test]
    fn nearest_colour_on_a_degenerate_rgb_plane_matches_brute_force() {
        let points: Vec<_> = (0..4096u32)
            .map(|i| [(i % 256) as u8, (i / 256) as u8, 31])
            .collect();
        let tree = ColourTree::new(&points);
        for i in 0..256u32 {
            let rgb = [
                (i * 17 % 256) as u8,
                (i * 13 % 256) as u8,
                (i * 43 % 256) as u8,
            ];
            let expected = (0..points.len())
                .min_by_key(|&index| {
                    let p = points[index];
                    let distance = (0..3)
                        .map(|axis| {
                            let d = rgb[axis] as i32 - p[axis] as i32;
                            d * d
                        })
                        .sum::<i32>();
                    (distance, index)
                })
                .unwrap();
            assert_eq!(tree.nearest(rgb), expected);
        }
    }
}
