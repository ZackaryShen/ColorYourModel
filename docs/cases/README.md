# Cases

Real models taken through the full CYM loop — segmentation, painting, export, sliced print — with evidence.

Status: **empty by design for now**; content is being written. Duplicate [`TEMPLATE.md`](TEMPLATE.md) per case.

```mermaid
flowchart LR
    M["pick a model<br/>(document provenance!)"] --> S["run pipeline<br/>record every setting"]
    S --> SL["slice in target slicer"]
    SL --> E["collect evidence<br/>CYM + slicer screenshots"]
    E --> W["write the case from TEMPLATE"]
```

Suggested first entries:

- [ ] 3MF end-to-end verification (2026-08-27, Snapmaker Orca / U1) — evidence exists in `samples/`; needs the written protocol (⚠️ before publishing: document the provenance/license of the model shown in `samples/`, or replace with a self-made model)
- [ ] A figure model with eye detection + seed fuse
- [ ] A mechanical/planar model with auto segmentation only

## Why write cases down?

A case = a reproducible claim: which model, which settings, which slicer, what came out, where the evidence lives. It doubles as regression evidence when the pipeline changes.
