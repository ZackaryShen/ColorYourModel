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

    /// Stable wire key for the history panel's row label (issue #22). The
    /// frontend maps this to icon + localized text; the mapping is part of
    /// the IPC contract and must not be renamed casually.
    pub fn key(self) -> &'static str {
        match self {
            OpKind::Paint => "paint",
            OpKind::SegmentPaint => "segmentPaint",
            OpKind::ManualRegion => "region",
            OpKind::Merge => "merge",
            OpKind::Split => "split",
        }
    }

    /// Stable one-byte encoding, persisted inside the project file.
    ///
    /// These numbers are a file-format contract: once a project has been
    /// written they must not be reassigned. Byte 5 is reserved for a future
    /// model-transform operation and is deliberately left unused here.
    pub fn to_u8(self) -> u8 {
        match self {
            OpKind::Paint => 0,
            OpKind::SegmentPaint => 1,
            OpKind::ManualRegion => 2,
            OpKind::Merge => 3,
            OpKind::Split => 4,
        }
    }

    /// Decode a byte written by [`OpKind::to_u8`].
    ///
    /// Returns `None` for every byte this build does not know, including the
    /// reserved byte 5: a project carrying an operation from a newer build must
    /// fail to load rather than silently undo it as the wrong kind.
    pub fn from_u8(b: u8) -> Option<OpKind> {
        match b {
            0 => Some(OpKind::Paint),
            1 => Some(OpKind::SegmentPaint),
            2 => Some(OpKind::ManualRegion),
            3 => Some(OpKind::Merge),
            4 => Some(OpKind::Split),
            _ => None,
        }
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

    /// Heap footprint measured by `len`, not `capacity`.
    ///
    /// [`History::from_raw`] charges loaded entries this way: a rebuilt entry
    /// owns `Vec`s whose capacity is an artefact of how the snapshot was
    /// filled, so charging capacity would make a loaded project's byte budget
    /// depend on the load path rather than on what it stored.
    fn bytes_by_len(&self) -> usize {
        std::mem::size_of::<Self>()
            + self.colors.len() * std::mem::size_of::<(u32, [u8; 4])>()
            + self.labels.len() * std::mem::size_of::<(u32, u32)>()
    }

    /// Project this entry into the owned form the project file persists.
    fn as_raw(&self) -> RawEntry {
        RawEntry {
            kind: self.kind,
            stroke_id: self.stroke_id,
            colors: self.colors.clone(),
            labels: self.labels.clone(),
        }
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

/// One history entry projected into owned, payload-only form.
///
/// This is the serialization seam: it mirrors [`HistoryEntry`]'s fields without
/// the stack or accounting state the `History` owns. The derives are part of
/// that contract — the project-file tests compare snapshots for equality.
#[derive(Debug, Clone, PartialEq)]
pub struct RawEntry {
    pub kind: OpKind,
    pub stroke_id: Option<u64>,
    pub colors: Vec<(u32, [u8; 4])>,
    pub labels: Vec<(u32, u32)>,
}

impl RawEntry {
    fn into_entry(self) -> HistoryEntry {
        HistoryEntry {
            kind: self.kind,
            stroke_id: self.stroke_id,
            colors: self.colors,
            labels: self.labels,
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
    /// drag must keep the colour it had before the drag started, not the one
    /// it had midway through, so only the first sighting is recorded.
    open_faces: HashSet<u32>,
    /// Live total across both stacks.
    bytes: usize,
    max_depth: usize,
    byte_budget: usize,
    persisted_truncated: bool,
}

/// One row of the history panel (issue #22): a read-only projection of an
/// applied entry — what the operation was and how much it touched. Entries
/// are incremental diffs, so this is cheap to produce on every refresh.
pub struct HistoryEntryInfo {
    pub kind: OpKind,
    pub stroke_id: Option<u64>,
    pub face_count: usize,
    pub bytes: usize,
}

/// What a [`History::jump_to`] actually changed, for the command layer to
/// turn into a patch. Faces is the UNION of everything the skipped steps
/// touched and colors holds each face's FINAL value — intermediate steps
/// must not leak into the patch the frontend applies.
pub struct JumpOutcome {
    pub kind: OpKind,
    pub faces: Vec<u32>,
    pub colors: Vec<[u8; 4]>,
    pub labels_changed: bool,
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
            persisted_truncated: false,
        }
    }

    pub fn persisted_truncated(&self) -> bool {
        self.persisted_truncated
    }

    pub fn set_persisted_truncated(&mut self, truncated: bool) {
        self.persisted_truncated = truncated;
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

    /// Panel listing (issue #22): every APPLIED entry, oldest first. Undone
    /// entries reappear when redone. `undo_depth()` is the current state's
    /// jump index — depth k means ops `0..k` are applied.
    pub fn entries(&self) -> Vec<HistoryEntryInfo> {
        self.undo
            .iter()
            .map(|e| HistoryEntryInfo {
                kind: e.kind,
                stroke_id: e.stroke_id,
                face_count: e.colors.len(),
                bytes: e.bytes(),
            })
            .collect()
    }

    /// Jump to state `depth` in ONE call (issue #22): `depth` entries of the
    /// timeline applied, `0` = pristine. Going back pops entries onto the
    /// redo branch (still listed, greyed, jump-forward-able); going forward
    /// re-applies redo entries, clamped to how many actually exist. Returns
    /// `None` when already at `depth`. Ends any open stroke, like undo/redo.
    pub fn jump_to(
        &mut self,
        depth: usize,
        colors: &mut [[u8; 4]],
        labels: &mut [u32],
    ) -> Option<JumpOutcome> {
        let current = self.undo.len();
        if depth == current {
            return None;
        }

        let mut faces: Vec<u32> = Vec::new();
        let mut seen: HashSet<u32> = HashSet::new();
        let mut labels_changed = false;
        let mut last_kind: Option<OpKind> = None;

        // Going back moves entries onto the redo branch; going forward moves
        // them back onto the applied stack. Direction is the only difference
        // between the two traversals.
        let going_back = depth < current;
        let steps = if going_back {
            current - depth
        } else {
            (depth - current).min(self.redo.len())
        };
        for _ in 0..steps {
            let mut entry = if going_back {
                self.undo.pop_back()?
            } else {
                self.redo.pop()?
            };
            let out = Self::apply(&mut entry, colors, labels);
            if going_back {
                self.redo.push(entry);
            } else {
                self.undo.push_back(entry);
            }
            union_into(&out.faces, &mut faces, &mut seen);
            labels_changed |= out.labels_changed;
            last_kind = Some(out.kind);
        }

        // A jump is a timeline move: whatever stroke was accepting appends is
        // now closed, exactly as a plain undo would have closed it.
        self.open_stroke = None;
        self.open_faces.clear();

        Some(JumpOutcome {
            kind: last_kind?,
            colors: faces.iter().map(|&f| colors[f as usize]).collect(),
            faces,
            labels_changed,
        })
    }

    pub fn bytes(&self) -> usize {
        self.bytes
    }

    /// Eviction ceiling, for the history panel's "step x / y" header (issue
    /// #22): the depth limit stops being invisible once it is on display.
    pub fn max_depth(&self) -> usize {
        self.max_depth
    }

    pub(crate) fn applied_entries(&self) -> impl DoubleEndedIterator<Item = &HistoryEntry> {
        self.undo.iter()
    }

    /// Project both stacks into owned payloads, for the project file.
    ///
    /// `.0` is the undo stack oldest→newest; `.1` is the redo stack with its
    /// top (the next redo) last — the same order the stacks themselves hold.
    /// The result is the timeline only: no open stroke, no byte account.
    pub fn to_raw(&self) -> (Vec<RawEntry>, Vec<RawEntry>) {
        (
            self.undo.iter().map(HistoryEntry::as_raw).collect(),
            self.redo.iter().map(HistoryEntry::as_raw).collect(),
        )
    }

    /// Rebuild a history from [`History::to_raw`] output.
    ///
    /// Stack order is preserved, including the redo top-last convention. No
    /// stroke survives the trip: a just-loaded history must not coalesce the
    /// next paint call into an entry from the previous session, and the
    /// first-sighting set that bounds a stroke starts empty.
    ///
    /// `bytes` is recomputed from the loaded payload lengths (not `capacity`,
    /// which is an artefact of how each `Vec` was filled), so the eviction
    /// budget reflects what was actually loaded. The default depth and byte
    /// limits apply, and entries are restored verbatim rather than evicted
    /// here — the snapshot is the state the caller asked for.
    pub fn from_raw(undo: Vec<RawEntry>, redo: Vec<RawEntry>) -> Self {
        let mut history = Self::new();
        history.undo = undo.into_iter().map(RawEntry::into_entry).collect();
        history.redo = redo.into_iter().map(RawEntry::into_entry).collect();
        history.open_stroke = None;
        history.open_faces.clear();
        history.bytes = history
            .undo
            .iter()
            .chain(history.redo.iter())
            .map(HistoryEntry::bytes_by_len)
            .sum();
        history
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
        self.persisted_truncated = false;
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
    pub fn undo(&mut self, colors: &mut [[u8; 4]], labels: &mut [u32]) -> Option<HistoryOutcome> {
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
    pub fn redo(&mut self, colors: &mut [[u8; 4]], labels: &mut [u32]) -> Option<HistoryOutcome> {
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
        while self.undo.len() > 1
            && (self.undo.len() > self.max_depth || self.bytes > self.byte_budget)
        {
            if let Some(dropped) = self.undo.pop_front() {
                self.bytes = self.bytes.saturating_sub(dropped.bytes());
            }
        }
    }
}

/// First-sighting union for [`History::jump_to`]'s patch: a face touched by
/// several skipped steps appears once, with its final colour read after the
/// whole jump has been applied.
fn union_into(faces: &[u32], out: &mut Vec<u32>, seen: &mut HashSet<u32>) {
    for f in faces {
        if seen.insert(*f) {
            out.push(*f);
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

        assert!(
            !h.can_redo(),
            "redo branch must not survive a new operation"
        );
        assert!(h.redo(&mut colors, &mut labels).is_none());
        assert_eq!(
            colors[1],
            [5, 5, 5, 255],
            "redo must not overwrite new paint"
        );
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
            h.record(
                OpKind::Paint,
                Some(i as u64),
                &[(i, colors[i as usize])],
                &[],
            );
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
            h.record(
                OpKind::Paint,
                Some(i as u64),
                &[(i, colors[i as usize])],
                &[],
            );
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
        assert_eq!(
            h.bytes(),
            after_record,
            "moving to redo must not change the total"
        );

        h.clear();
        assert_eq!(h.bytes(), 0);
    }

    #[test]
    fn recording_nothing_creates_no_entry() {
        let mut h = History::new();
        h.record(OpKind::Paint, Some(1), &[], &[]);
        h.record(OpKind::Merge, None, &[], &[]);
        assert!(
            !h.can_undo(),
            "an operation that touched nothing is not an operation"
        );
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

    /// A three-operation timeline on a three-face buffer: two paint strokes
    /// with distinct ids, then a label-only merge. The returned buffers hold
    /// the state *after* all three operations.
    fn three_op_timeline() -> (History, Vec<[u8; 4]>, Vec<u32>) {
        let (mut colors, mut labels) = buffers(3);
        let mut h = History::new();

        h.record(OpKind::Paint, Some(1), &[(0, colors[0])], &[]);
        colors[0] = [10, 0, 0, 255];

        h.record(OpKind::Paint, Some(2), &[(1, colors[1])], &[]);
        colors[1] = [20, 0, 0, 255];

        h.record(OpKind::Merge, None, &[], &[(2, labels[2])]);
        labels[2] = 100_000;

        (h, colors, labels)
    }

    /// The byte formula `from_raw` must use: payload measured by `len`.
    fn len_bytes(e: &RawEntry) -> usize {
        std::mem::size_of::<HistoryEntry>()
            + e.colors.len() * std::mem::size_of::<(u32, [u8; 4])>()
            + e.labels.len() * std::mem::size_of::<(u32, u32)>()
    }

    /// A history saved and reloaded must undo and redo identically, including
    /// when the snapshot is taken while an entry sits on the redo branch.
    #[test]
    fn raw_round_trip_preserves_undo_and_redo_effects() {
        let (mut h, colors_after_all, labels_after_all) = three_op_timeline();
        assert_eq!(h.undo_depth(), 3, "two paints + one merge");

        // Undo the merge before the snapshot, so the round trip has to carry a
        // populated redo branch as well as both stacks' ordering.
        let mut live_colors = colors_after_all.clone();
        let mut live_labels = labels_after_all.clone();
        h.undo(&mut live_colors, &mut live_labels).unwrap();
        let colors_after_undo = live_colors.clone();
        let labels_after_undo = live_labels.clone();
        assert_eq!(
            labels_after_undo,
            vec![0, 0, 0],
            "the merge is undone again"
        );
        assert_eq!(
            colors_after_undo, colors_after_all,
            "a merge never repaints"
        );

        let (undo, redo) = h.to_raw();
        assert_eq!(undo.len(), 2);
        assert_eq!(redo.len(), 1);
        let mut restored = History::from_raw(undo, redo);
        assert_eq!(restored.undo_depth(), 2);
        assert_eq!(restored.redo_depth(), 1);

        let mut restored_colors = colors_after_undo.clone();
        let mut restored_labels = labels_after_undo.clone();

        // Redo the merge on the reloaded history: same effect.
        let out = restored
            .redo(&mut restored_colors, &mut restored_labels)
            .unwrap();
        assert!(out.labels_changed);
        assert_eq!(restored_colors, colors_after_all, "redo colours");
        assert_eq!(restored_labels, labels_after_all, "redo labels");

        // Undo it again: back to the recorded state.
        restored
            .undo(&mut restored_colors, &mut restored_labels)
            .unwrap();
        assert_eq!(restored_colors, colors_after_undo);
        assert_eq!(restored_labels, labels_after_undo);

        // Walk the rest of both timelines in lockstep: every undo and redo must
        // leave the two histories in the same colour/label state.
        for step in 0..2 {
            let live = h.undo(&mut live_colors, &mut live_labels);
            let reloaded = restored.undo(&mut restored_colors, &mut restored_labels);
            assert_eq!(
                live.is_some(),
                reloaded.is_some(),
                "undo {step} availability"
            );
            assert_eq!(live_colors, restored_colors, "undo {step} colours");
            assert_eq!(live_labels, restored_labels, "undo {step} labels");
        }
        assert!(!h.can_undo() && !restored.can_undo());
        assert_eq!(h.redo_depth(), 3);
        assert_eq!(restored.redo_depth(), 3);
        assert_eq!(live_colors, vec![[0, 0, 0, 255]; 3], "pristine colours");
        assert_eq!(live_labels, vec![0, 0, 0], "pristine labels");

        // At pristine every operation sits on the redo branch. Snapshot again
        // and replay: the top-last order must survive, because replaying the
        // entries out of order restores intermediate states out of sequence.
        let (pristine_undo, pristine_redo) = h.to_raw();
        assert!(pristine_undo.is_empty());
        assert_eq!(pristine_redo.len(), 3);
        let mut redo_only = History::from_raw(pristine_undo, pristine_redo);
        let mut redo_colors = live_colors.clone();
        let mut redo_labels = live_labels.clone();
        for step in 0..3 {
            let live = h.redo(&mut live_colors, &mut live_labels);
            let reloaded = redo_only.redo(&mut redo_colors, &mut redo_labels);
            assert_eq!(
                live.is_some(),
                reloaded.is_some(),
                "redo {step} availability"
            );
            assert_eq!(live_colors, redo_colors, "redo {step} colours");
            assert_eq!(live_labels, redo_labels, "redo {step} labels");
        }
        assert_eq!(live_colors, colors_after_all);
        assert_eq!(live_labels, labels_after_all);
    }

    /// `from_raw` must charge the loaded payload by `len`, not `capacity`, or
    /// the byte budget of a loaded project depends on how the vectors were
    /// filled.
    #[test]
    fn from_raw_recomputes_the_byte_accounting() {
        let (h, _colors, _labels) = three_op_timeline();
        let (undo, redo) = h.to_raw();
        let expected: usize = undo.iter().chain(redo.iter()).map(len_bytes).sum();
        assert!(expected > 0, "the fixture must store a real payload");

        let restored = History::from_raw(undo, redo);
        assert_eq!(restored.bytes(), expected, "bytes = sum of stored payloads");
        assert_eq!(restored.max_depth(), DEFAULT_MAX_DEPTH);
        assert_eq!(restored.undo_depth(), 3);
        assert_eq!(restored.redo_depth(), 0);

        // Spare capacity must not be charged: a caller-built Vec with room for
        // 64 colours still stores one.
        let mut colors = Vec::with_capacity(64);
        colors.push((0, [1, 2, 3, 4]));
        let mut labels = Vec::with_capacity(64);
        labels.push((1, 7));
        let sparse = History::from_raw(
            vec![RawEntry {
                kind: OpKind::SegmentPaint,
                stroke_id: None,
                colors,
                labels,
            }],
            Vec::new(),
        );
        let one_entry = std::mem::size_of::<HistoryEntry>()
            + std::mem::size_of::<(u32, [u8; 4])>()
            + std::mem::size_of::<(u32, u32)>();
        assert_eq!(sparse.bytes(), one_entry, "len, not capacity");
    }

    /// A freshly loaded history must not merge the next paint call into a
    /// stroke that belonged to the previous session.
    #[test]
    fn an_open_stroke_never_survives_a_round_trip() {
        let (mut colors, mut labels) = buffers(2);
        let mut h = History::new();

        // Two appends under one id coalesce into a single entry.
        h.record(OpKind::Paint, Some(7), &[(0, colors[0])], &[]);
        colors[0] = [1, 0, 0, 255];
        h.record(OpKind::Paint, Some(7), &[(1, colors[1])], &[]);
        colors[1] = [1, 0, 0, 255];
        assert_eq!(h.undo_depth(), 1, "same stroke id coalesces");

        let (undo, redo) = h.to_raw();
        let mut restored = History::from_raw(undo, redo);

        // Recording under the same id must open a new entry...
        restored.record(OpKind::Paint, Some(7), &[(0, colors[0])], &[]);
        assert_eq!(restored.undo_depth(), 2, "the open stroke did not survive");

        // ...and its first-sighting set must be empty too: face 0 must be
        // recorded even though the old open stroke had already seen it.
        colors[0] = [9, 9, 9, 255];
        restored.undo(&mut colors, &mut labels).unwrap();
        assert_eq!(
            colors[0],
            [1, 0, 0, 255],
            "the open-face dedup must not leak either"
        );
    }

    /// Kind bytes are a persisted contract; byte 5 is reserved for a future
    /// model-transform op and must not be reused before that op exists.
    #[test]
    fn kind_bytes_are_stable() {
        assert_eq!(OpKind::Paint.to_u8(), 0);
        assert_eq!(OpKind::SegmentPaint.to_u8(), 1);
        assert_eq!(OpKind::ManualRegion.to_u8(), 2);
        assert_eq!(OpKind::Merge.to_u8(), 3);
        assert_eq!(OpKind::Split.to_u8(), 4);

        for kind in [
            OpKind::Paint,
            OpKind::SegmentPaint,
            OpKind::ManualRegion,
            OpKind::Merge,
            OpKind::Split,
        ] {
            assert_eq!(
                OpKind::from_u8(kind.to_u8()),
                Some(kind),
                "{} must round trip",
                kind.key()
            );
        }

        assert_eq!(
            OpKind::from_u8(5),
            None,
            "kind 5 预留给模型变换，本版本不可加载"
        );
        for b in 6..=u8::MAX {
            assert_eq!(OpKind::from_u8(b), None, "byte {b} is not a known op kind");
        }
    }
}
