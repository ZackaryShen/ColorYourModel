# IPC Reference

> Part of the CYM wiki — every Tauri command the frontend can invoke.
> Source of truth: `src-tauri/src/lib.rs` (`generate_handler!`) and `src-tauri/src/commands/`.

All commands live in the Rust backend and are invoked from TypeScript with `invoke("<name>", { args })`. Async commands run on worker threads so long operations don't block the UI.

## Mesh

| Command | Signature | Purpose |
|---------|-----------|---------|
| `load_model` | `(path: String) -> MeshDataDto` | load binary/ASCII STL, build adjacency + kdtree, emit progress events |
| `get_face_color` | `(face_id: u32) -> [u8; 4]` | RGBA of one face |

## Segmentation

| Command | Signature | Purpose |
|---------|-----------|---------|
| `auto_segment` | `(angle_threshold: f32) -> SegmentResult` | legacy three-phase dihedral segmentation |
| `auto_segment_smart` | `(k: u32) -> SegmentResult` | legacy curvature k-means segmentation |
| `auto_segment_v2` | `(algorithm: SegmentationAlgorithm, preserve_manual?: bool) -> SegmentResult` | **unified entry** — dispatches one of the 8 algorithms; keeps manual regions by default; runs async |
| `resegment_region` | `(label: u32, algorithm: SegmentationAlgorithm) -> SegmentResult` | re-run segmentation inside one region, async |
| `reset_segmentation` | `() -> SegmentResult` | back to a single default region |
| `merge_segments` | `(target_id: u32, source_ids: Vec<u32>) -> Vec<Segment>` | merge regions into target |
| `split_segment` | `(label: u32, method: SplitMethod) -> SplitResult` | split a region (plane or crease method) |
| `rename_segment` | `(segment_id: u32, name: String) -> Vec<Segment>` | rename a region |
| `paint_segment_face` | `(face_id: u32, segment_label?: u32, stroke_id?: u64) -> PaintSegmentFaceResult` | segment-brush: mark one face during a drag |
| `finalize_segment` | `(segment_label: u32) -> FinalizeSegmentResult` | finish a segment-brush drag (labels ≥ 100,000 only) |
| `manual_region_add_point` | `(point: [f32; 3], face_index: u32) -> ManualPointResult` | lasso: add a snapped point |
| `finalize_manual_region` | `(points: Vec<[f32; 3]>, face_indices: Vec<u32>) -> SegmentResult` | lasso: close the loop, create the region |
| `seed_grow` | `(seeds: Vec<SeedInput>, barrier_deg: f32, optimizer: bool) -> SegmentResult` | seeded watershed growth |
| `recommend_seeds` | `(count: usize, curvature: f32, concavity: f32) -> Vec<SeedSuggestion>` | saliency-based seed suggestions |
| `detect_planar_regions` | `(angle_threshold_deg: f32, dist_thr_factor: f32, min_region_faces: u32) -> Vec<PlanarRegion>` | flat-region detection |
| `detect_eye_regions` | `(roi_faces: Vec<u32>) -> Vec<EyeRegion>` | eye detection inside a ROI |
| `detect_eye_regions_auto` | `() -> Vec<EyeRegion>` | eye detection, whole model (Stage 1) |
| `detect_multiview_regions` | `(view_count: u32, angle_threshold_deg: f32, min_region_faces: u32, match_threshold: u32) -> Vec<MultiViewRegion>` | regions consistent across views |
| `detect_cross_section_features` | `(planes_per_axis: u32, feature_threshold: f32) -> Vec<CrossSectionRegion>` | features on slicing planes |
| `fuse_segmentation` | async | fuse seed-grown regions + tiny-region merge |

## Painting

| Command | Signature | Purpose |
|---------|-----------|---------|
| `brush_paint` | `(center_face: u32, radius: f32, strength: f32, falloff_mode: String, color: [u8; 4], stroke_id?: u64) -> PaintResult` | soft brush stroke |
| `spray_paint` | `(center_face: u32, radius: f32, strength: f32, color: [u8; 4], density: u32, stroke_id?: u64) -> PaintResult` | scattered spray dots |
| `smart_brush_paint` | `(center_face: u32, radius: f32, strength: f32, falloff_mode: String, color: [u8; 4], stroke_id?: u64) -> PaintResult` | brush that respects region boundaries |
| `fill_paint` | `(face_id: u32, color: [u8; 4], radius: f32) -> PaintResult` | fill the region under the clicked face (label-authoritative) |
| `fill_segment_paint` | `(segment_id: u32, color: [u8; 4]) -> PaintResult` | fill an explicit region id |
| `erase_paint` | `(center_face: u32, radius: f32, stroke_id?: u64) -> PaintResult` | remove colour |
| `pick_color` | `(face_id: u32) -> [u8; 4]` | eyedropper |

`stroke_id` deduplicates faces within one drag so overlapping stamps don't darken colour twice.

## History

| Command | Signature | Purpose |
|---------|-----------|---------|
| `undo` / `redo` | `() -> HistoryResult` | single timeline over paint / fill / erase / manual edits |
| `history_state` | `() -> HistoryState` | can-undo / can-redo + position for UI gating |

## Export

| Command | Signature | Purpose |
|---------|-----------|---------|
| `list_export_presets` | `() -> Vec<MachineSummary>` | vendor/machine/nozzle/process/filament catalogue (embedded presets) |
| `export_palette_preview` | `() -> Vec<String>` | palette slots as filament-slot preview |
| `export_3mf_command` | `(path: String, selection?: ExportSelection) -> String` | write 3MF with colours + machine settings; verifies the file landed non-empty; async |
| `export_obj_command` | `(path: String) -> String` | write `.obj` + `.mtl` (per-face colour via `usemtl`, ≤256 colours); async |

`ExportSelection` = `{ machineId, nozzleDiameter, processName, filamentNames: string[], targetSlicer: "snapmaker_orca" | "orcaslicer" }`.

## Diagnostics

| Command | Signature | Purpose |
|---------|-----------|---------|
| `report_js_error` | `(msg: String)` | frontend → backend error capture (in-app DebugLogViewer) |
| `report_app_ready` | `()` | readiness handshake |

## Notes for contributors

- Manual region labels start at `MANUAL_SEGMENT_OFFSET = 100_000` — anything below belongs to automatic segmentation.
- `SegmentationAlgorithm` is a tagged union (`type` + params) with 8 variants: `dihedral`, `shapeDiameter`, `curvatureKMeans`, `sdfGraphCut`, `concavity`, `convexDecomposition`, `curveSkeleton`, `fhGraph` — see [Mesh Segmentation](../algorithms/segmentation.md).
- Adding a command: implement in `commands/`, register in `lib.rs` `generate_handler!`, keep the UI-language i18n keys symmetric.
