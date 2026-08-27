# Seed-Based Segmentation: Grow & Fuse

> Status: **placeholder** — content pending. Fill freely in English or 中文.

## Scope

The seed-driven segmentation workflow exposed in `SeedPanel` (Auto = fuse / Manual = grow):

- **Seed recommendation** — planar regions, multiview, cross-section features, saliency (curvature / concavity)
- **Manual grow** — point-by-point seeded region growing (`seed_grow`, `manual_region_add_point`, `finalize_manual_region`)
- **Auto fuse** — fusing seed-grown regions with aggressive tiny-region merging (`fuse_segmentation`), including the reverse-split behaviour and the live region-size breakdown
- Known protection: eye regions are kept intact across internal dihedral cuts

## Code pointers

- `src-tauri/src/segment/seeded.rs` · `fuse.rs` · `manual.rs` · `recommend.rs`
- `src-tauri/src/segment/{planar,multiview,cross_section}.rs`
- `src/components/SeedPanel.tsx` (Auto/Manual modes)

## Related research notes (中文, historical)

- `../09-连续平面与边界-算法融合研究.md` — plane/boundary fusion research

## TODO

- [ ] Grow barrier-angle semantics (what the slider really does)
- [ ] Fuse/tiny-merge thresholds with provenance
- [ ] Region-authority note: algorithms propagate labels, never colours (see [../technical/fill-routing.md](../technical/fill-routing.md))
