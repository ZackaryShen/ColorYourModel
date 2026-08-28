# Eye-Region Semantic Detection

> One-click eye detection for figure models. Status: **v1 written from verified sources**.
> Usage guide: [User Guide: Seed Tools](../user-guide/seed-tools.md#seed-recommendation).

## Why eyes

Eyes are small, high-curvature and visually critical: no other region ruins a figure print as fast when segmentation gets it wrong. CYM detects them and treats them as **protected** seeds for everything downstream.

## Two entry points

| Command | Scope | Since |
|---------|-------|-------|
| `detect_eye_regions(roi_faces)` | inside a face ROI; thresholds via `EyeParams` (parameterized in `6b63659`) | ROI era |
| `detect_eye_regions_auto()` | **whole model, no ROI needed** — template matching backend (`detect_eyes_global`, `TemplateParams`) | Stage 1, `8d0e063` |

```mermaid
flowchart LR
    M["full model scan<br/>(auto) or ROI scan"] --> C["geometry cues /<br/>template match"]
    C --> F["filter & validate"]
    F --> E["eye-region labels<br/>protected in fuse / merge"]
```

## Behaviour

- **Stale suggestions are cleared** before a new detection runs — old recommendations can't contaminate the new result (`3756321`).
- **Lone eye-only grows are guarded** — a seed grow that would end up eye-only without its body fails safe (`3756321`).
- **Protection** — detected eye regions survive internal dihedral cuts (`2fc242a`) and fuse/merge passes.
- Detection results land as regions/labels ready for [seed grow & fuse](seed-grow-fuse.md) — never as direct colour writes.

## Code pointers

- `src-tauri/src/segment/template.rs` — global template backend (Stage 1)
- `src-tauri/src/segment/eye.rs` — eye primitives + `EyeParams`
- `src-tauri/src/commands/segment.rs` — both IPC entries
- `src/components/SeedPanel.tsx` — the "detect eyes" button

## TODO

- [ ] Template/cue pipeline detail (what the template matches, acceptance thresholds with provenance)
- [ ] Failure gallery & manual recovery (re-seed by hand when detection misses)
- [ ] ROI vs. auto accuracy comparison on the sample corpus
