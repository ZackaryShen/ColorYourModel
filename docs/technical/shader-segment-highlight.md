# Shader Segment Highlight

> Status: **placeholder** — content pending. Fill freely in English or 中文.

## Scope

The GPU highlight scheme (Plan B, iter. 2026-08-08) that replaced geometry-rebuilding overlays:

- Per-vertex `aSegLabel` attribute + `uHighlightLabel`/`uHighlightColor` uniforms injected via `onBeforeCompile`
- O(1) hover switching (was O(F) geometry rebuild — the giant-segment freeze root cause)
- `flat` qualifier on the varying; `preserveDrawingBuffer` removed
- Conditional `frameloop="demand"` (`IdleFrameloop`: sleep after 1.5 s idle, wake on interaction)
- Tool-gated highlight: only fill / picker / segment tools compute highlight

## Code pointers

- `src/components/Viewport/Viewport.tsx` — material patch, uniform wiring, gating
- `src/segmentStages.ts`

## TODO

- [ ] Shader snippet walkthrough
- [ ] Why highlight is display-only (never a paint/fill input — see fill-routing.md)
