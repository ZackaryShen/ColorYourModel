//! Snapmaker Orca native Gradient recipes use the painted height span in
//! each object. CYM's surface samples stay exact in the project archive.
use std::collections::{BTreeMap, HashMap};

use nalgebra::Vector3;
use petgraph::graph::NodeIndex;

use crate::mesh::model::MeshModel;
use crate::paint::gradient::GradientSample;

use super::paint_color::MAX_EXTRUDER_SLOT;
use super::quantize::{
    bound_color_counts, color_histogram_with_limit, map_colors_to_palette, nearest_palette_index,
    quantize_histogram, Quantized, MAX_HISTOGRAM_COLORS,
};
use super::threemf::ProjectPlate;

type Rgb = [u8; 3];
type Counts = BTreeMap<Rgb, usize>;

// Snapmaker Orca 2.4.2 counts physical and virtual filaments together.
const SNAPMAKER_SLOT_LIMIT: usize = 64;
// The 2.4.2 parser accepts 1% to 99% and disables gradients below 5 points.
const ENDPOINT_RANGE: std::ops::RangeInclusive<f64> = 0.01..=0.99;
const MIN_GRADIENT_SPAN: f64 = 0.06;

#[derive(Default)]
pub(crate) struct GradientColors {
    stops: Counts,
    solids: Counts,
    reduced: bool,
}

pub(crate) struct ColorHistograms {
    pub(crate) flat: Vec<(Rgb, usize)>,
    pub(crate) gradient: Option<GradientColors>,
}

impl ColorHistograms {
    pub(crate) fn from_mesh(mesh: &MeshModel) -> Self {
        // A gradient plate caches two additional histograms inside the same
        // per-plate budget as the ordinary palette preview.
        let gradient = (!mesh.gradient_faces.is_empty()).then(|| GradientColors::from_mesh(mesh));
        Self {
            flat: Self::flat_from_mesh(mesh),
            gradient,
        }
    }

    pub(crate) fn flat_from_mesh(mesh: &MeshModel) -> Vec<(Rgb, usize)> {
        let limit = if mesh.gradient_faces.is_empty() {
            MAX_HISTOGRAM_COLORS
        } else {
            MAX_HISTOGRAM_COLORS / 2
        };
        color_histogram_with_limit(&mesh.face_colors, mesh.faces.len(), limit)
    }
}

impl GradientColors {
    fn from_mesh(mesh: &MeshModel) -> Self {
        let mut result = Self::default();
        for (face, &color) in mesh.face_colors.iter().enumerate() {
            if let Some(sample) = mesh.gradient_faces.get(&(face as u32)) {
                result.add_sample(sample);
            } else {
                *result.solids.entry(rgb(color)).or_default() += 1;
            }
        }
        let limit = MAX_HISTOGRAM_COLORS / 4;
        result.reduced = result.solids.len() > limit || result.stops.len() > limit;
        result.solids = bound_color_counts(result.solids, limit)
            .into_iter()
            .collect();
        result.stops = bound_color_counts(result.stops, limit)
            .into_iter()
            .collect();
        result
    }

    fn add_sample(&mut self, sample: &GradientSample) {
        if sample.mix < u16::MAX {
            *self.stops.entry(rgb(sample.from)).or_default() += 1;
        }
        if sample.mix > 0 && (sample.from != sample.to || sample.mix == u16::MAX) {
            *self.stops.entry(rgb(sample.to)).or_default() += 1;
        }
    }

    pub(crate) fn absorb(&mut self, histograms: &ColorHistograms) {
        if let Some(gradient) = &histograms.gradient {
            merge(&mut self.stops, &gradient.stops);
            merge(&mut self.solids, &gradient.solids);
            self.reduced |= gradient.reduced;
        } else {
            merge(
                &mut self.solids,
                histograms.flat.iter().map(|(rgb, count)| (rgb, count)),
            );
        }
    }

    pub(crate) fn palette(&self, limit: usize) -> Option<PhysicalPalette> {
        if self.stops.is_empty() || limit == 0 {
            return None;
        }
        let mut stops = self
            .stops
            .iter()
            .map(|(&color, &n)| (color, n))
            .collect::<Vec<_>>();
        stops.sort_unstable_by_key(|&(color, n)| (std::cmp::Reverse(n), color));
        let mut colors = stops
            .into_iter()
            .take(limit)
            .map(|(color, _)| color)
            .collect::<Vec<_>>();
        let remaining = limit - colors.len();
        if remaining > 0 {
            let solids = self
                .solids
                .iter()
                .filter(|(color, _)| !colors.contains(color))
                .map(|(&color, &n)| (color, n))
                .collect::<Vec<_>>();
            for color in quantize_histogram(&solids, remaining) {
                if !colors.contains(&color) {
                    colors.push(color);
                }
            }
        }
        let approximated = self.reduced
            || self
                .stops
                .keys()
                .chain(self.solids.keys())
                .any(|color| !colors.contains(color));
        Some(PhysicalPalette {
            colors,
            approximated,
        })
    }
}

fn merge<'a>(counts: &mut Counts, source: impl IntoIterator<Item = (&'a Rgb, &'a usize)>) {
    for (&color, &n) in source {
        *counts.entry(color).or_default() += n;
    }
}

pub(crate) struct PhysicalPalette {
    pub(crate) colors: Vec<Rgb>,
    pub(crate) approximated: bool,
}

fn rgb([r, g, b, _]: [u8; 4]) -> Rgb {
    [r, g, b]
}

struct GradientGroup<'a> {
    plate: usize,
    pair: [u8; 2],
    samples: Vec<(u32, &'a GradientSample)>,
}

fn collect_groups<'a>(
    plate: usize,
    mesh: &'a MeshModel,
    palette: &[Rgb],
) -> Vec<GradientGroup<'a>> {
    let mut owners = HashMap::with_capacity(mesh.gradient_faces.len());
    let entries: Vec<_> = mesh.history.applied_gradients().collect();
    for (index, entry) in entries.iter().enumerate() {
        for &(face, _) in &entry.colors {
            if mesh.gradient_faces.contains_key(&face) {
                owners.entry(face).or_insert(index);
            }
        }
    }
    // An independent recipe for each applied gesture prevents two strokes
    // on one object from sharing the same native height span.
    let mut groups = BTreeMap::new();
    for (&face, sample) in &mesh.gradient_faces {
        let key = (
            owners.get(&face).copied().unwrap_or(usize::MAX),
            sample.from,
            sample.to,
        );
        groups
            .entry(key)
            .or_insert_with(Vec::new)
            .push((face, sample));
    }
    groups
        .into_iter()
        .flat_map(|((owner, from, to), mut samples)| {
            let pair = [from, to].map(|c| nearest_palette_index(palette, rgb(c)) as u8 + 1);
            if pair[0] == pair[1] {
                return Vec::new();
            }
            samples.sort_unstable_by_key(|&(face, _)| face);
            let group = GradientGroup {
                plate,
                pair,
                samples,
            };
            let repeated_pair = entries
                .get(owner)
                .and_then(|entry| entry.gradient_context.as_deref())
                .is_some_and(|context| {
                    context
                        .stops
                        .windows(2)
                        .filter(|stops| stops == &[from, to])
                        .count()
                        > 1
                });
            if (owner == usize::MAX || repeated_pair)
                && mesh.face_adjacency.node_count() == mesh.faces.len()
            {
                connected_groups(mesh, group)
            } else {
                vec![group]
            }
        })
        .filter(|group| {
            let all_at = |mix| group.samples.iter().all(|(_, sample)| sample.mix == mix);
            !all_at(0) && !all_at(u16::MAX)
        })
        .collect()
}

/// Recover independent height ranges if the older gesture history is gone
/// or the same ordered pair appears in more than one part of one gesture.
fn connected_groups<'a>(mesh: &MeshModel, group: GradientGroup<'a>) -> Vec<GradientGroup<'a>> {
    let mut unvisited: HashMap<_, _> = group.samples.iter().copied().collect();
    let mut groups = Vec::new();
    for (face, _) in group.samples {
        let Some(sample) = unvisited.remove(&face) else {
            continue;
        };
        let mut samples = vec![(face, sample)];
        let mut cursor = 0;
        while let Some(&(face, _)) = samples.get(cursor) {
            for node in mesh.face_adjacency.neighbors(NodeIndex::new(face as usize)) {
                let neighbor = node.index() as u32;
                if let Some(sample) = unvisited.remove(&neighbor) {
                    samples.push((neighbor, sample));
                }
            }
            cursor += 1;
        }
        samples.sort_unstable_by_key(|&(face, _)| face);
        groups.push(GradientGroup {
            plate: group.plate,
            pair: group.pair,
            samples,
        });
    }
    groups
}

struct NativeGradient {
    pair: [u8; 2],
    low_a: f64,
    high_a: f64,
}

impl NativeGradient {
    fn fit(mesh: &MeshModel, group: &GradientGroup<'_>) -> Self {
        let mut min_z = f64::INFINITY;
        let mut max_z = f64::NEG_INFINITY;
        let mut area_sum = 0.0;
        let mut mean_z = 0.0;
        let mut mean_a = 0.0;
        let mut variance_z = 0.0;
        let mut covariance = 0.0;
        for &(face, sample) in &group.samples {
            let [a, b, c] = mesh.faces[face as usize]
                .map(|vertex| Vector3::from(mesh.vertices[vertex as usize].map(f64::from)));
            min_z = min_z.min(a.z.min(b.z).min(c.z));
            max_z = max_z.max(a.z.max(b.z).max(c.z));
            let z = (a.z + b.z + c.z) / 3.0;
            let ratio_a = 1.0 - f64::from(sample.mix) / f64::from(u16::MAX);
            // Weight by surface area so the native fit is insensitive to
            // the original painter's uneven triangle subdivision.
            let weight = (b - a).cross(&(c - a)).norm().max(f64::EPSILON);
            area_sum += weight;
            let dz = z - mean_z;
            let da = ratio_a - mean_a;
            mean_z += (weight / area_sum) * dz;
            mean_a += (weight / area_sum) * da;
            variance_z += weight * dz * (z - mean_z);
            covariance += weight * dz * (ratio_a - mean_a);
        }
        let slope = if variance_z > area_sum * 1e-8 {
            covariance / variance_z
        } else {
            0.0
        };
        let predict = |height: f64| {
            (mean_a + slope * (height - mean_z))
                .clamp(*ENDPOINT_RANGE.start(), *ENDPOINT_RANGE.end())
        };
        let mut result = Self {
            pair: group.pair,
            low_a: predict(min_z),
            high_a: predict(max_z),
        };
        if result.low_a < result.high_a {
            result.pair.swap(0, 1);
            result.low_a = 1.0 - result.low_a;
            result.high_a = 1.0 - result.high_a;
        }
        if result.low_a - result.high_a < MIN_GRADIENT_SPAN {
            let half = MIN_GRADIENT_SPAN / 2.0;
            let middle = (0.5 * (result.low_a + result.high_a))
                .clamp(*ENDPOINT_RANGE.start() + half, *ENDPOINT_RANGE.end() - half);
            // Same-height paint has no native XY gradient. Keep its average
            // mix within a valid native row and let Orca resolve its layer.
            result.low_a = middle + half;
            result.high_a = middle - half;
        }
        result
    }

    fn serialize(&self, stable_id: usize) -> String {
        let [a, b] = self.pair;
        let Self { low_a, high_a, .. } = self;
        format!(
            "{a},{b},1,1,50,0,g,w,m0,z2,xa0,xb0,d0,o0,u{stable_id},cm3,r1/{low_a:.4}/{high_a:.4}"
        )
    }
}

pub(crate) struct PrintGradients {
    pub(crate) quantized: Quantized,
    pub(crate) definitions: String,
    pub(crate) omitted_recipes: usize,
}

impl PrintGradients {
    pub(crate) fn build(
        plates: &[ProjectPlate<'_>],
        colors: &[[u8; 4]],
        limit: usize,
    ) -> Option<Self> {
        if !plates
            .iter()
            .any(|plate| !plate.mesh.gradient_faces.is_empty())
        {
            return None;
        }
        let mut gradient_colors = GradientColors::default();
        for plate in plates {
            gradient_colors.absorb(&ColorHistograms::from_mesh(plate.mesh));
        }
        let limit = limit.min(usize::from(MAX_EXTRUDER_SLOT));
        let palette = gradient_colors.palette(limit)?.colors;
        Some(Self::with_palette(plates, colors, palette, "", 0))
    }

    pub(crate) fn with_palette(
        plates: &[ProjectPlate<'_>],
        colors: &[[u8; 4]],
        palette: Vec<Rgb>,
        native_definitions: &str,
        native_count: usize,
    ) -> Self {
        let first_new_slot = palette.len() + native_count;
        let budget = SNAPMAKER_SLOT_LIMIT.saturating_sub(first_new_slot);
        let mut face_slots = map_colors_to_palette(colors, &palette);
        let mut offsets = Vec::with_capacity(plates.len());
        let mut groups = Vec::new();
        let mut offset = 0;
        for (index, plate) in plates.iter().enumerate() {
            offsets.push(offset);
            if !native_definitions.is_empty() {
                for (face, &slot) in plate.mesh.source_material_slots.iter().enumerate() {
                    if slot == 0
                        || slot as usize > first_new_slot
                        || plate.mesh.gradient_faces.contains_key(&(face as u32))
                    {
                        continue;
                    }
                    let source = plate
                        .mesh
                        .source_material_colors
                        .get(slot as usize - 1)
                        .copied()
                        .flatten();
                    if source.is_some_and(|rgba| plate.mesh.face_colors.get(face) == Some(&rgba)) {
                        face_slots[offset + face] = slot;
                    }
                }
            }
            groups.extend(collect_groups(index, plate.mesh, &palette));
            offset += plate.mesh.faces.len();
        }
        // Keep the largest painted groups when the native slot table is full.
        groups.sort_by_key(|group| std::cmp::Reverse(group.samples.len()));
        let omitted_recipes = groups.len().saturating_sub(budget);
        let occupied_ids = native_definitions
            .split([';', ','])
            .filter_map(|token| token.strip_prefix('u')?.parse::<usize>().ok())
            .collect::<std::collections::HashSet<_>>();
        let mut next_id = (1..).filter(|id| !occupied_ids.contains(id));
        let mut definitions = native_definitions.to_owned();
        for (index, group) in groups.into_iter().take(budget).enumerate() {
            if !definitions.is_empty() && !definitions.ends_with(';') {
                definitions.push(';');
            }
            let gradient = NativeGradient::fit(plates[group.plate].mesh, &group);
            definitions.push_str(&gradient.serialize(next_id.next().unwrap()));
            let slot = (first_new_slot + index + 1) as u8;
            for (face, _) in group.samples {
                face_slots[offsets[group.plate] + face as usize] = slot;
            }
        }
        Self {
            quantized: Quantized {
                palette,
                face_slots,
            },
            definitions,
            omitted_recipes,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mesh::history::{GradientContext, GradientTool};

    #[test]
    fn strokes_with_the_same_filaments_keep_independent_height_directions() {
        let stops = [[33, 39, 33, 255], [0, 255, 255, 255]];
        let mut mesh = MeshModel::new();
        for [x, z] in [[0.0, 2.0], [0.0, 8.0], [2.0, 2.0], [2.0, 8.0]] {
            let first = mesh.vertices.len() as u32;
            mesh.vertices
                .extend([[x, 0.0, z], [x + 1.0, 0.0, z], [x, 1.0, z]]);
            mesh.faces.push([first, first + 1, first + 2]);
            mesh.face_colors.push(stops[0]);
        }
        let context = GradientContext {
            tool: GradientTool::Brush,
            stops: stops.into(),
            points: Box::new([]),
            radius: 2.0,
            whole_region: false,
        };
        let sample = |mix| GradientSample {
            from: stops[0],
            to: stops[1],
            mix,
            ramp_id: 0,
        };
        mesh.apply_gradient(&[(0, sample(16384)), (1, sample(49151))], context.clone());
        mesh.apply_gradient(&[(2, sample(49151)), (3, sample(16384))], context);
        let plates = [ProjectPlate {
            name: "Two strokes".into(),
            mesh: &mesh,
        }];
        let gradient =
            PrintGradients::with_palette(&plates, &mesh.face_colors, stops.map(rgb).into(), "", 0);

        let rows: Vec<_> = gradient.definitions.split(';').collect();
        assert_eq!(rows.len(), 2);
        assert!(rows[0].starts_with("2,1,") && rows[0].ends_with(",cm3,r1/0.7500/0.2500"));
        assert!(rows[1].starts_with("1,2,") && rows[1].ends_with(",cm3,r1/0.7500/0.2500"));
        assert_eq!(gradient.quantized.face_slots, vec![4, 4, 3, 3]);
    }

    #[test]
    fn disconnected_gradient_regions_without_history_get_separate_recipes() {
        let [from, to] = [[33, 39, 33, 255], [0, 255, 255, 255]];
        let mut mesh = MeshModel::new();
        for bottom in [0.0, 20.0] {
            let first_vertex = mesh.vertices.len() as u32;
            let first_face = mesh.faces.len() as u32;
            mesh.vertices.extend([
                [0.0, 0.0, bottom],
                [1.0, 0.0, bottom],
                [1.0, 0.0, bottom + 6.0],
                [0.0, 0.0, bottom + 6.0],
            ]);
            mesh.faces.extend([
                [first_vertex, first_vertex + 1, first_vertex + 2],
                [first_vertex, first_vertex + 2, first_vertex + 3],
            ]);
            for (face, mix) in [(first_face, 16384), (first_face + 1, 49151)] {
                let sample = GradientSample { from, to, mix, ramp_id: 0 };
                mesh.face_colors.push(sample.rgba());
                mesh.gradient_faces.insert(face, sample);
            }
        }
        mesh.build_adjacency();
        let plates = [ProjectPlate {
            name: "Legacy".into(),
            mesh: &mesh,
        }];
        let gradient = PrintGradients::with_palette(
            &plates,
            &mesh.face_colors,
            [rgb(from), rgb(to)].into(),
            "",
            0,
        );
        assert_eq!(gradient.definitions.matches(",cm3,r1/").count(), 2);
        assert_eq!(gradient.quantized.face_slots, [3, 3, 4, 4]);
    }
}
