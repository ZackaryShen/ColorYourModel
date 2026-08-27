# Fill Routing: Label Authority

> Status: **placeholder** — content pending. Fill freely in English or 中文.

## Scope

The correctness rule that fixed the "fill C same colour → A changes" bug (`a7700df`, 2026-08-27):

- **Labels are truth; colour is output.** Fill targets strictly `segmentLabels[clickedFace]` — the region the clicked face belongs to. Colour never participates in region identity.
- History: iter29 hover preference + the v5 stale cache (`lastValidHoveredSegmentRef`) used to hijack fills to wrong regions; both retired. Hover snapshot is HUD diagnostic only.
- Regression guarantees in `commands::paint::fill_tests`: same-colour double-region fill must not modify the first region; Shift+click flood stops at label boundaries even when both sides share a colour.
- Where the rule generalizes: fuse/grow/lasso must also propagate labels, never overwrite `face_colors` with palette colours (open backlog item — see tracker `../04`, §8).

## Code pointers

- `src-tauri/src/paint/fill.rs` — core fill + tests
- `src-tauri/src/commands/paint.rs` — `fill_paint` / `fill_segment_paint`
- `src/hooks/usePaintTool.ts` — frontend target routing

## TODO

- [ ] Decision record: why clicked-face label beats hover/memory heuristics
- [ ] Fuse/grow repaint ruling & the 3MF colour-strategy decision it depends on
