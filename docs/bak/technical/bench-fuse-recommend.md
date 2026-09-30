# Fuse / Recommend Stage Benchmark

Release-mode stage timings for the one-click fuse pipeline (`fuse_segmentation`)
and seed recommendation (`recommend_seeds`), measured by the
`bench_fuse_and_recommend_stages` test in `src-tauri/src/segment/mod.rs`.

## What it measures

The bench mirrors the `fuse_segmentation` command stage by stage —
planar (15/M30) → multiview (12 views) → dihedral fold → edge vote with the
SeedPanel's real cut=2/min=0 — plus `recommend_seeds` at panel defaults
(count 12, curvature 1.0, concavity 1.0). A fresh load per `CYM_BENCH_DIHEDRAL`
setting matches the command path. It reports per-stage timings, region counts,
and max/tiny-region shares.

## How to run

```bash
# Release mode; the test is #[ignore] because it reads large STL files.
CYM_BENCH_STL=samples/_src/YourModel.stl CYM_BENCH_DIHEDRAL=15,2 \
  cargo test --release --lib bench_fuse_and_recommend_stages -- --ignored --nocapture
```

- `CYM_BENCH_STL` selects the model (default: `samples/_src/Sanji+Diorama+Detailed_U1.stl`).
- `CYM_BENCH_DIHEDRAL` is a comma list of fold-angle settings (default `15,2`).

**Models are NOT shipped with the repository.** Drop your own STLs under
`samples/_src/` (gitignored) or point `CYM_BENCH_STL` anywhere on disk.

## Results (2026-09-21 session, 17 STLs, release build)

Per-model details were recorded for three representative models:

| Model | Faces | Fold angle | planar | multiview | dihedral | vote | Total | Regions |
|-------|-------|-----------|--------|-----------|----------|------|-------|---------|
| godzilla | 1.5M | 2° | 0.4s | 9.4s | 2.2s | 0.5s | ~12.7s | 70 |
| luffy | 500K | 2° | — | — | — | — | 8.0s | 87 (max region share 8.0%) |
| house2 | 4.7M | — | — | — | — | timeout | >30min | — |

### Scalability finding

On house2 (4.7M faces) the vote stage ran over the 30-minute bench timeout
while iterating 4.73M dihedral sets: `MAX_AUTO_REGIONS` (`postprocess.rs`,
4096) is **not wired into the fuse path**, so nothing caps the work on huge
meshes. Wiring the cap into `fuse_region_sets` is recorded as backlog.
