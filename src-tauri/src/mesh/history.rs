//! Unified undo/redo history for every operation that mutates face colour or
//! segment label.
//!
//! # Why the history lives in Rust
//!
//! `face_colors` and `segment_labels` are owned by the backend. Keeping their
//! history anywhere else means two sources of truth for one piece of state,
//! which is precisely the structure that produces drift. Before this module the
//! project had exactly that: a full-snapshot stack in the frontend store for
//! paint strokes, and `manual_region_history` in the backend for lasso regions,
//! with `Ctrl+Z` branching between them.
//!
//! # Storage model
//!
//! An entry stores the state that existed *before* the operation ran, for the
//! faces the operation touched — not a copy of the whole buffer. On a 1.5M-face
//! model a full-model operation costs ~12 MB instead of ~30 MB, and an ordinary
//! brush stroke costs a few kilobytes.
//!
//! An entry is **its own inverse**. Applying it swaps the stored values with the
//! live ones, so the same entry that undid an operation now describes how to
//! redo it. That halves the memory a naive before/after pair would need and
//! makes it impossible for the two directions to disagree.
//!
//! # Stroke coalescing without a stroke protocol
//!
//! A brush stroke fires one IPC call per pointermove, so a single drag produces
//! hundreds of backend calls that the user thinks of as one action. The obvious
//! fix — `begin_stroke` / `end_stroke` commands — is stateful, and there are at
//! least five ways a pointer interaction ends without a clean `pointerup`
//! (async paint queue still draining, pointerleave, pointercancel, alt-tab,
//! component unmount). Every one of them leaves the stroke open and silently
//! merges the next stroke into it.
//!
//! Instead the caller passes a monotonic `stroke_id`. Consecutive calls carrying
//! the same id append to the same entry; a different id starts a new one. There
//! is no open/closed state to leak, so a lost `pointerup` costs at most one
//! extra history entry rather than a corrupted timeline.

use std::collections::{HashSet, VecDeque};

/// Which user action produced a history entry.
///
/// The first four variants map onto the operation classes in the Gate 0b'
/// contract. `Split` was added later as a dedicated command (`split_segment`):
/// it partitions one existing region into several along its internal creases,
/// rather than being a lasso side effect, so it needed its own history kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpKind {
    /// Brush, spray, smart brush, fill or eraser. Colour only; labels untouched.
    Paint,
    /// Segment brush (`paint_segment_face`). Colour and label.
    SegmentPaint,
    /// Lasso region finalize. Colour and label.
    ManualRegion,
    /// Absorbing one or more regions into another. **Label only** — merging
    /// changes which region a face belongs to, not how it is painted, and the
    /// paint is what the exporter actually reads.
    Merge,
    /// Partitioning one region into several along its internal creases. **Label
    /// only**, for the same reason as `Merge`: the 3MF exporter reads
    /// `face_colors`, so re-colouring the split-off faces would discard real
    /// output. Each resulting connected piece becomes its own region.
    Split,
}

impl OpKind {
    /// Whether this kind of operation can change `segment_labels`.
    ///
    /// Label-bearing undos have to take a different path back to the frontend:
    /// the segment view memoises on the whole mesh-data object, so patching
    /// labels in place would restore the data without ever repainting.
    pub fn touches_labels(self) -> bool {
        !matches!(self, OpKind::Paint)
    }
}

/// One undoable operation, stored as the state that existed before it ran.
pub struct HistoryEntry {
    pub kind: OpKind,
    /// Which pointer stroke this entry belongs to. `None` for one-shot
    /// operations (fill, lasso) that can never coalesce.
    pub stroke_id: Option<u64>,
    /// `(face, colour that was there before this operation)`.
    pub colors: Vec<(u32, [u8; 4])>,
    /// `(face, label that was there before this operation)`. Empty for
    /// [`OpKind::Paint`].
    pub labels: Vec<(u32, u32)>,
}

impl HistoryEntry {
    /// Heap footprint of this entry.
    ///
    /// Uses `capacity`, not `len`: a `Vec` grown by repeated `push` typically
    /// holds up to twice the bytes its length suggests, and the budget exists to
    /// bound real memory rather than to look tidy.
    fn bytes(&self) -> usize {
        std::mem::size_of::<Self>()
            + self.colors.capacity() * std::mem::size_of::<(u32, [u8; 4])>()
            + self.labels.capacity() * std::mem::size_of::<(u32, u32)>()
    }

    /// Swap every stored value with the live one, turning this entry into its
    /// own inverse.
    fn apply_swap(&mut self, colors: &mut [[u8; 4]], labels: &mut [u32]) {
        for (face, stored) in self.colors.iter_mut() {
            std::mem::swap(&mut colors[*face as usize], stored);
        }
        for (face, stored) in self.labels.iter_mut() {
            std::mem::swap(&mut labels[*face as usize], stored);
        }
    }
}

/// Depth ceiling. Secondary to [`DEFAULT_BYTE_BUDGET`] — a run of small brush
/// strokes will hit this first, a run of full-model fills will not get close.
pub const DEFAULT_MAX_DEPTH: usize = 50;

/// Memory ceiling across both stacks. This is the binding constraint: at
/// ~12 MB for a 1.5M-face full-model operation it permits roughly 20 such
/// entries, which is why the depth above must not be read as a promise.
pub const DEFAULT_BYTE_BUDGET: usize = 256 * 1024 * 1024;

/// What an undo or redo actually changed, for the command layer to turn into a
/// patch for the frontend.
pub struct HistoryOutcome {
    pub kind: OpKind,
    /// Faces whose colour changed, in entry order.
    pub faces: Vec<u32>,
    /// The colour each of those faces now has, index-aligned with `faces`.
    pub colors: Vec<[u8; 4]>,
    pub labels_changed: bool,
}

/// Undo and redo stacks for one loaded mesh.
pub struct History {
    undo: VecDeque<HistoryEntry>,
    redo: Vec<HistoryEntry>,
    /// The stroke currently accepting appends, if any.
    open_stroke: Option<u64>,
    /// Faces already recorded in the open stroke. A face painted twice in one
    /// drag must keep the colour it had before the drag started, not the one it
    /// had midway through, so only the first sighting is recorded.
    open_faces: HashSet<u32>,
    /// Live total across both stacks.
    bytes: usize,
    max_depth: usize,
    byte_budget: usize,
}

impl Default for History {
    fn default() -> Self {
        Self::new()
    }
}

impl History {
    pub fn new() -> Self {
        Self::with_limits(DEFAULT_MAX_DEPTH, DEFAULT_BYTE_BUDGET)
    }

    pub fn with_limits(max_depth: usize, byte_budget: usize) -> Self {
        Self {
            undo: VecDeque::new(),
            redo: Vec::new(),
            open_stroke: None,
            open_faces: HashSet::new(),
            bytes: 0,
            max_depth,
            byte_budget,
        }
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    pub fn undo_depth(&self) -> usize {
        self.undo.len()
    }

    pub fn redo_depth(&self) -> usize {
        self.redo.len()
    }

    pub fn bytes(&self) -> usize {
        self.bytes
    }

    /// Drop everything.
    ///
    /// Required whenever an operation rewrites state the diffs can no longer be
    /// reconciled against — re-running auto-segmentation replaces every label,
    /// so an entry recorded before it would restore pre-segmentation labels onto
    /// the new result.
    pub fn clear(&mut self) {
        self.undo.clear();
        self.redo.clear();
        self.open_stroke = None;
        self.open_faces.clear();
        self.bytes = 0;
    }

    /// Record the pre-operation state of the faces an operation is about to
    /// change.
    ///
    /// Must be called **before** the write, with values read from the live
    /// buffers.
    ///
    /// `prev_labels` must be index-aligned with `prev_colors` (same faces, same
    /// order) whenever `kind.touches_labels()`, and empty otherwise. The paint
    /// and lasso call sites build both vectors in one pass, so this costs
    /// nothing and avoids a second dedup set.
    ///
    /// A label-bearing operation may also pass **no** colours at all: a merge
    /// re-labels faces without repainting them. Recording a run of unchanged
    /// colours purely to satisfy the alignment rule would cost 8 bytes a face
    /// to store values that are identical on both sides of the swap, so the
    /// empty case is accepted and the entry carries labels only. That is safe
    /// downstream because `HistoryOutcome::labels_changed` already forces the
    /// command layer onto the full-payload path, where `faces`/`colors` are
    /// ignored and the whole buffer is resent.
    pub fn record(
        &mut self,
        kind: OpKind,
        stroke_id: Option<u64>,
        prev_colors: &[(u32, [u8; 4])],
        prev_labels: &[(u32, u32)],
    ) {
        debug_assert!(
            if kind.touches_labels() {
                prev_colors.is_empty() || prev_labels.len() == prev_colors.len()
            } else {
                prev_labels.is_empty()
            },
            "prev_labels must be aligned with prev_colors for label-bearing ops"
        );
        // "Touched nothing" is the only case that must not create an entry;
        // this used to test `prev_colors` alone, which silently discarded any
        // label-only operation and left it out of the timeline entirely.
        if prev_colors.is_empty() && prev_labels.is_empty() {
            return;
        }

        // Any new operation invalidates the redo branch. Doing it here, in the
        // one function every mutation funnels through, is the only way to be
        // sure no future command forgets: a stale redo entry would swap colours
        // recorded against a timeline that no longer exists, and after a manual
        // region is undone the next one reuses the same label number, so a stale
        // `prev_label` would not even be detectably wrong.
        self.drop_redo();

        // Two conditions that should always agree, checked separately on
        // purpose. `open_stroke` is the explicit "a stroke is still accepting
        // appends" flag, which undo/redo clear; the entry's own `stroke_id`
        // guards the case where the flag survives but the entry it referred to
        // has moved to the redo stack. Requiring both means neither can drift
        // into merging an append onto the wrong entry.
        let coalesce = stroke_id.is_some()
            && stroke_id == self.open_stroke
            && self.undo.back().map(|e| (e.kind, e.stroke_id)) == Some((kind, stroke_id));

        if !coalesce {
            self.open_stroke = stroke_id;
            self.open_faces.clear();
            self.push_entry(HistoryEntry {
                kind,
                stroke_id,
                colors: Vec::new(),
                labels: Vec::new(),
            });
        }

        // Split the borrow: the dedup set and the entry live in different fields.
        let Self {
            undo, open_faces, ..
        } = self;
        let entry = undo.back_mut().expect("entry pushed above");
        let before = entry.bytes();
        if prev_colors.is_empty() {
            // Label-only (merge). Same first-sighting-wins dedup, driven by the
            // only vector there is.
            for &(face, label) in prev_labels {
                if open_faces.insert(face) {
                    entry.labels.push((face, label));
                }
            }
        } else {
            let aligned = prev_labels.len() == prev_colors.len();
            for (i, &(face, color)) in prev_colors.iter().enumerate() {
                if open_faces.insert(face) {
                    entry.colors.push((face, color));
                    if aligned {
                        debug_assert_eq!(prev_labels[i].0, face, "label/colour face mismatch");
                        entry.labels.push(prev_labels[i]);
                    }
                }
            }
        }
        let grew = entry.bytes() - before;
        self.bytes += grew;

        self.evict();
    }

    /// Revert the most recent operation. Returns `None` when there is nothing
    /// to undo.
    pub fn undo(
        &mut self,
        colors: &mut [[u8; 4]],
        labels: &mut [u32],
    ) -> Option<HistoryOutcome> {
        let mut entry = self.undo.pop_back()?;
        let outcome = Self::apply(&mut entry, colors, labels);
        self.redo.push(entry);
        // An undo ends whatever stroke was still accepting appends; the next
        // paint call must not merge into an entry that has already moved.
        self.open_stroke = None;
        self.open_faces.clear();
        Some(outcome)
    }

    /// Re-apply the most recently undone operation. Returns `None` when there
    /// is nothing to redo.
    pub fn redo(
        &mut self,
        colors: &mut [[u8; 4]],
        labels: &mut [u32],
    ) -> Option<HistoryOutcome> {
        let mut entry = self.redo.pop()?;
        let outcome = Self::apply(&mut entry, colors, labels);
        self.undo.push_back(entry);
        self.open_stroke = None;
        self.open_faces.clear();
        Some(outcome)
    }

    fn apply(
        entry: &mut HistoryEntry,
        colors: &mut [[u8; 4]],
        labels: &mut [u32],
    ) -> HistoryOutcome {
        entry.apply_swap(colors, labels);
        HistoryOutcome {
            kind: entry.kind,
            faces: entry.colors.iter().map(|&(f, _)| f).collect(),
            colors: entry
                .colors
                .iter()
                .map(|&(f, _)| colors[f as usize])
                .collect(),
            labels_changed: !entry.labels.is_empty(),
        }
    }

    fn push_entry(&mut self, entry: HistoryEntry) {
        self.bytes += entry.bytes();
        self.undo.push_back(entry);
    }

    fn drop_redo(&mut self) {
        for entry in self.redo.drain(..) {
            self.bytes = self.bytes.saturating_sub(entry.bytes());
        }
    }

    /// Enforce both ceilings by discarding the oldest entries.
    ///
    /// Never empties the stack: a single operation larger than the whole budget
    /// is still worth one level of undo, and dropping it would leave the user
    /// with no way back from the most destructive action available.
    fn evict(&mut self) {
        while self.undo.len() > 1 && (self.undo.len() > self.max_depth || self.bytes > self.byte_budget) {
            if let Some(dropped) = self.undo.pop_front() {
                self.bytes = self.bytes.saturating_sub(dropped.bytes());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn buffers(n: usize) -> (Vec<[u8; 4]>, Vec<u32>) {
        (vec![[0, 0, 0, 255]; n], vec![0u32; n])
    }

    /// Paint one face, undo, redo. The entry is used in both directions.
    #[test]
    fn undo_then_redo_round_trips() {
        let (mut colors, mut labels) = buffers(4);
        let mut h = History::new();

        h.record(OpKind::Paint, None, &[(1, colors[1])], &[]);
        colors[1] = [9, 9, 9, 255];

        assert!(h.can_undo() && !h.can_redo());
        h.undo(&mut colors, &mut labels).unwrap();
        assert_eq!(colors[1], [0, 0, 0, 255]);
        assert!(!h.can_undo() && h.can_redo());

        h.redo(&mut colors, &mut labels).unwrap();
        assert_eq!(colors[1], [9, 9, 9, 255]);
        assert!(h.can_undo() && !h.can_redo());
    }

    /// The failure the swap trick invites: a redo entry that outlives the
    /// timeline it was recorded against would resurrect a colour from a branch
    /// the user abandoned.
    #[test]
    fn new_operation_discards_the_redo_branch() {
        let (mut colors, mut labels) = buffers(4);
        let mut h = History::new();

        h.record(OpKind::Paint, None, &[(1, colors[1])], &[]);
        colors[1] = [9, 9, 9, 255];
        h.undo(&mut colors, &mut labels).unwrap();
        assert!(h.can_redo());

        h.record(OpKind::Paint, None, &[(1, colors[1])], &[]);
        colors[1] = [5, 5, 5, 255];

        assert!(!h.can_redo(), "redo branch must not survive a new operation");
        assert!(h.redo(&mut colors, &mut labels).is_none());
        assert_eq!(colors[1], [5, 5, 5, 255], "redo must not overwrite new paint");
    }

    /// A drag paints the same face repeatedly. Undo must return it to the
    /// colour it had before the drag, not to some intermediate blend.
    #[test]
    fn a_stroke_keeps_only_the_colour_from_before_the_drag() {
        let (mut colors, mut labels) = buffers(4);
        colors[2] = [1, 1, 1, 255];
        let mut h = History::new();

        for shade in 1..=5u8 {
            h.record(OpKind::Paint, Some(7), &[(2, colors[2])], &[]);
            colors[2] = [shade * 10, 0, 0, 255];
        }

        assert_eq!(h.undo_depth(), 1, "one drag is one undo level");
        h.undo(&mut colors, &mut labels).unwrap();
        assert_eq!(colors[2], [1, 1, 1, 255]);
    }

    #[test]
    fn a_different_stroke_id_starts_a_new_entry() {
        let (mut colors, _labels) = buffers(4);
        let mut h = History::new();

        h.record(OpKind::Paint, Some(1), &[(0, colors[0])], &[]);
        colors[0] = [1, 0, 0, 255];
        h.record(OpKind::Paint, Some(2), &[(0, colors[0])], &[]);
        colors[0] = [2, 0, 0, 255];

        assert_eq!(h.undo_depth(), 2);
    }

    /// Two tools sharing a stroke id by accident must not share an entry: the
    /// label half of a segment-paint entry would be silently dropped into a
    /// paint entry that does not carry labels.
    #[test]
    fn a_different_op_kind_never_coalesces() {
        let (mut colors, mut labels) = buffers(4);
        let mut h = History::new();

        h.record(OpKind::Paint, Some(1), &[(0, colors[0])], &[]);
        colors[0] = [1, 0, 0, 255];
        h.record(
            OpKind::SegmentPaint,
            Some(1),
            &[(1, colors[1])],
            &[(1, labels[1])],
        );
        colors[1] = [2, 0, 0, 255];
        labels[1] = 100_000;

        assert_eq!(h.undo_depth(), 2);
    }

    /// Labels and colours travel together, so undoing a region restores both.
    #[test]
    fn label_bearing_entries_restore_labels_too() {
        let (mut colors, mut labels) = buffers(4);
        let mut h = History::new();

        let prev_colors: Vec<_> = (0..3u32).map(|f| (f, colors[f as usize])).collect();
        let prev_labels: Vec<_> = (0..3u32).map(|f| (f, labels[f as usize])).collect();
        h.record(OpKind::ManualRegion, None, &prev_colors, &prev_labels);
        for f in 0..3usize {
            colors[f] = [7, 7, 7, 255];
            labels[f] = 100_000;
        }

        let out = h.undo(&mut colors, &mut labels).unwrap();
        assert!(out.labels_changed);
        assert_eq!(labels, vec![0, 0, 0, 0]);
        assert_eq!(colors[0], [0, 0, 0, 255]);
    }

    #[test]
    fn depth_ceiling_drops_the_oldest_entry() {
        let (mut colors, _labels) = buffers(8);
        let mut h = History::with_limits(3, DEFAULT_BYTE_BUDGET);

        for i in 0..5u32 {
            h.record(OpKind::Paint, Some(i as u64), &[(i, colors[i as usize])], &[]);
            colors[i as usize] = [1, 0, 0, 255];
        }
        assert_eq!(h.undo_depth(), 3);
    }

    /// A single operation bigger than the entire budget still buys one undo.
    #[test]
    fn byte_ceiling_never_empties_the_stack() {
        let (mut colors, _labels) = buffers(8);
        let mut h = History::with_limits(DEFAULT_MAX_DEPTH, 1);

        for i in 0..4u32 {
            h.record(OpKind::Paint, Some(i as u64), &[(i, colors[i as usize])], &[]);
            colors[i as usize] = [1, 0, 0, 255];
        }
        assert_eq!(h.undo_depth(), 1);
        assert!(h.can_undo());
    }

    /// Byte accounting has to survive entries moving between the stacks and
    /// being discarded, or the budget drifts until it stops binding.
    #[test]
    fn byte_accounting_returns_to_zero() {
        let (mut colors, mut labels) = buffers(8);
        let mut h = History::new();

        h.record(OpKind::Paint, None, &[(0, colors[0])], &[]);
        colors[0] = [1, 0, 0, 255];
        let after_record = h.bytes();
        assert!(after_record > 0);

        h.undo(&mut colors, &mut labels).unwrap();
        assert_eq!(h.bytes(), after_record, "moving to redo must not change the total");

        h.clear();
        assert_eq!(h.bytes(), 0);
    }

    #[test]
    fn recording_nothing_creates_no_entry() {
        let mut h = History::new();
        h.record(OpKind::Paint, Some(1), &[], &[]);
        h.record(OpKind::Merge, None, &[], &[]);
        assert!(!h.can_undo(), "an operation that touched nothing is not an operation");
    }

    /// A merge re-labels faces without repainting them. `record` used to bail
    /// out on the empty colour vector, so the entire operation stayed out of
    /// the timeline: Ctrl+Z skipped straight past the merge to the brush stroke
    /// before it, and the regions could not be separated again.
    #[test]
    fn a_label_only_operation_is_undoable() {
        let (mut colors, mut labels) = buffers(4);
        labels[2] = 100_001;
        labels[3] = 100_001;
        let mut h = History::new();

        let prev: Vec<_> = [2u32, 3].iter().map(|&f| (f, labels[f as usize])).collect();
        h.record(OpKind::Merge, None, &[], &prev);
        labels[2] = 100_000;
        labels[3] = 100_000;

        assert!(h.can_undo(), "the merge must be on the timeline");
        let out = h.undo(&mut colors, &mut labels).unwrap();
        assert!(out.labels_changed, "must force the full-payload path");
        assert!(out.faces.is_empty(), "no colour patch to send");
        assert_eq!(labels, vec![0, 0, 100_001, 100_001]);

        h.redo(&mut colors, &mut labels).unwrap();
        assert_eq!(labels, vec![0, 0, 100_000, 100_000]);
        assert_eq!(colors, vec![[0, 0, 0, 255]; 4], "a merge never repaints");
    }
}
