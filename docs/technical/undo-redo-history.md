# Unified Undo / Redo Timeline

> Status: **placeholder** — content pending. Fill freely in English or 中文.

## Scope

The single backend history timeline (stage 2 consolidation):

- `undo` / `redo` / `history_state` cover paint, fill, eraser and manual region edits
- The former `manual_region_undo` and `restore_face_colors` commands were deleted — no parallel history paths
- Frontend `useHistory` wiring and how state snapshots stay memory-bounded on 1.5M-face models

## Code pointers

- `src-tauri/src/commands/history.rs` · `src-tauri/src/mesh/history.rs`
- `src/hooks/useHistory.ts`

## TODO

- [ ] Memory model per snapshot (what is stored, coalescing rules)
- [ ] Interaction with segmentation resets
