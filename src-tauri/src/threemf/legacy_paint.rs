//! Recover palette strokes from the colour deltas in older CYM saves.
use crate::mesh::history::{HistoryEntry, OpKind};
use crate::mesh::model::{MeshModel, DEFAULT_FACE_COLOR};

type Color = [u8; 4];

#[derive(serde::Deserialize)]
struct Swatch {
    color: [u8; 3],
}

/// Rewind only a scratch colour buffer. For each live face, consider its last
/// colour-changing operation, preserving any provenance already in the file.
pub fn recover_missing(mesh: &mut MeshModel) {
    if !mesh.history.applied_entries().any(is_stroke) {
        return;
    }
    let Ok(swatches) =
        serde_json::from_str::<Vec<Swatch>>(include_str!("../../../src/types/amsPalette.json"))
    else {
        return;
    };
    let mut colors = mesh.face_colors.clone();
    let mut visited = vec![false; colors.len()];
    for entry in mesh.history.applied_entries().rev() {
        let needs_recovery = is_stroke(entry)
            && entry.colors.iter().any(|&(face, before)| {
                let index = face as usize;
                !visited[index]
                    && colors[index] != before
                    && !mesh.paint_sources.contains_key(&face)
            });
        let source = needs_recovery
            .then(|| match_stroke(entry, &colors, &swatches))
            .flatten();
        for &(face, before) in &entry.colors {
            let index = face as usize;
            let painted = std::mem::replace(&mut colors[index], before);
            if painted == before || std::mem::replace(&mut visited[index], true) {
                continue;
            }
            if let Some(source) = source {
                if painted != DEFAULT_FACE_COLOR && fits_blend(before, painted, source) {
                    mesh.paint_sources.entry(face).or_insert((source, painted));
                }
            }
        }
    }
}

fn is_stroke(entry: &HistoryEntry) -> bool {
    entry.kind == OpKind::Paint && entry.stroke_id.is_some()
}

fn match_stroke(entry: &HistoryEntry, colors: &[Color], swatches: &[Swatch]) -> Option<Color> {
    const SAMPLE_LIMIT: usize = 1024;
    // Near-zero changes carry too little hue information after 8-bit rounding.
    const MIN_DELTA: u8 = 24;
    let stride = entry.colors.len().div_ceil(SAMPLE_LIMIT).max(1);
    let samples: Vec<_> = entry
        .colors
        .iter()
        .step_by(stride)
        .filter_map(|&(face, before)| {
            let after = colors[face as usize];
            (after != DEFAULT_FACE_COLOR
                && (0..3).any(|channel| before[channel].abs_diff(after[channel]) >= MIN_DELTA))
            .then_some((before, after))
        })
        .collect();
    if samples.len() < 8 {
        return None;
    }

    let mut scores: Vec<_> = swatches
        .iter()
        .map(|swatch| {
            let [r, g, b] = swatch.color;
            let source = [r, g, b, 255];
            let hits = samples
                .iter()
                .filter(|&&(before, after)| fits_blend(before, after, source))
                .count();
            (hits, source)
        })
        .collect();
    scores.sort_unstable_by(|a, b| b.0.cmp(&a.0));
    let [(best, source), (runner_up, _), ..] = scores.as_slice() else {
        return None;
    };
    // Keep a wide margin for old strokes made with a custom or ambiguous colour.
    (*best * 10 >= samples.len() * 9 && (*best - *runner_up) * 5 >= samples.len())
        .then_some(*source)
}

fn fits_blend(before: Color, after: Color, source: Color) -> bool {
    // A channel is truncated on each dab. Allow a few levels of accumulated
    // drift in old coalesced strokes, while requiring a common blend strength.
    const DRIFT: f32 = 6.0;
    let (mut low, mut high) = (0.0f32, 1.0f32);
    for i in 0..3 {
        let delta = f32::from(source[i]) - f32::from(before[i]);
        let change = f32::from(after[i]) - f32::from(before[i]);
        if delta == 0.0 {
            if change.abs() > DRIFT {
                return false;
            }
        } else {
            let a = (change - DRIFT) / delta;
            let b = (change + DRIFT) / delta;
            low = low.max(a.min(b));
            high = high.min(a.max(b));
            if low > high {
                return false;
            }
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mesh::face_colors::mix_color;

    const WHITE: Color = [255, 255, 255, 255];
    const RED: Color = [255, 0, 0, 255];

    #[test]
    fn groups_legacy_shades_and_respects_later_edits_and_stored_sources() {
        let mut mesh = MeshModel::new();
        mesh.face_colors = vec![WHITE; 16];
        mesh.segment_labels = vec![0; 16];
        let stroke: Vec<_> = (0..16)
            .map(|face| (face, mix_color(&WHITE, &RED, (face + 2) as f32 / 18.0)))
            .collect();
        mesh.apply_paint(Some(1), &stroke);

        // A fill and a later erasure shadow the old stroke. The stored
        // source on another face is authoritative, even if it is unusual.
        mesh.apply_paint(None, &[(0, [9, 8, 7, 255])]);
        mesh.apply_paint(Some(2), &[(1, DEFAULT_FACE_COLOR)]);
        let stored = [251, 10, 12, 255];
        mesh.record_paint_sources(Some(stored), &[stroke[2]]);

        let exact = mesh.face_colors.clone();
        let history = mesh.history.to_raw();
        recover_missing(&mut mesh);
        assert_eq!(mesh.face_colors, exact);
        assert_eq!(mesh.history.to_raw(), history);
        assert_eq!(mesh.paint_source_groups(), vec![(RED, 13), (stored, 1)]);
        assert!(!mesh
            .used_colors()
            .iter()
            .any(|entry| entry.color == stroke[9].1));
        assert!(!mesh.paint_sources.contains_key(&0));
        assert!(!mesh.paint_sources.contains_key(&1));
    }

    #[test]
    fn leaves_an_ambiguous_grey_stroke_unclassified() {
        let mut mesh = MeshModel::new();
        mesh.face_colors = vec![WHITE; 16];
        let stroke: Vec<_> = (0..16)
            .map(|face| {
                (
                    face,
                    [216 + face as u8, 216 + face as u8, 216 + face as u8, 255],
                )
            })
            .collect();
        mesh.apply_paint(Some(1), &stroke);
        recover_missing(&mut mesh);
        assert!(mesh.paint_source_groups().is_empty());
    }
}
