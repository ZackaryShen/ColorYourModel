export interface BoundingBox {
  min: [number, number, number];
  max: [number, number, number];
}

export interface Segment {
  id: number;
  name: string;
  color: [number, number, number, number] | null;
  faceCount: number;
}

export interface MeshData {
  vertices: number[];
  faces: number[];
  faceColors: number[];
  segmentLabels: number[];
  bbox: BoundingBox;
  faceCount: number;
  segments: Segment[];
}

export interface PaintResult {
  updatedFaces: number[];
  updatedColors: number[][];
}

export enum PaintTool {
  View = "view",
  Fill = "fill",
  Brush = "brush",
  Spray = "spray",
  SmartBrush = "smart",
  Eyedropper = "picker",
  Eraser = "eraser",
  Segment = "segment",
  Lasso = "lasso",
  Seed = "seed",
}

/// A user-placed seed for seeded watershed segmentation (iteration 50).
/// `point` is the snapped 3D position (model-local); `faceIndex` is the face the
/// raycaster hit, so the backend can re-snap exactly.
export interface SeedPoint {
  x: number;
  y: number;
  z: number;
  faceIndex: number;
}

/// Layer 1 planar-region detection result (`detect_planar_regions`). One entry
/// per detected continuous flat patch. `seed` is a representative interior face
/// that the UI drops into the ghost-suggestion set (same path as `recommend`),
/// `boundaryEdges` outlines the patch as 3D line segments (each `[[x,y,z],[x,y,z]]`).
export interface PlanarRegion {
  plane: [number, number, number, number];
  faceCount: number;
  seed: SeedPoint;
  boundaryEdges: number[][][];
}

/// Layer 3 (MultiView 3→2→3) region detection result (`detect_multiview_regions`).
/// Same shape as `PlanarRegion` so the UI can reuse the ghost-seed path and the
/// boundary-outline renderer. One entry per multi-view-consensus cluster.
export interface MultiViewRegion {
  faceCount: number;
  seed: SeedPoint;
  boundaryEdges: number[][][];
}

/// Layer 2 (cross-section / ray marching, docs/09) feature detection result
/// (`detect_cross_section_features`). A *visual-only* evidence overlay: each
/// entry is one feature cross-section (where the cross-sectional profile
/// changes sharply) carrying its actual 3D contour (`boundaryEdges`) plus a
/// generalized-winding-number inside/outside confidence. It is NOT a partition
/// seed (a slice is a plane, not a face) so it never feeds `seed_grow`.
export interface CrossSectionRegion {
  plane: number[];
  axis: number;
  position: number;
  areaMetric: number;
  winding: number;
  boundaryEdges: number[][][];
}

export interface SegmentResult {
  segments: Segment[];
  segmentLabels: number[];
  faceColors: number[];
}

export interface ManualPointResult {
  vertexIndex: number;
  faceId: number;
  snapped: [number, number, number];
}

/// Result of an undo/redo step from the unified backend history.
///
/// Two shapes, selected by `full` (backend `HistoryResult`, camelCased):
///   - `full === false` — colour-only patch: write `faces`+`colors` incrementally.
///   - `full === true`  — replace `segments`/`segmentLabels`/`faceColors` wholesale.
/// `canUndo`/`canRedo` let the toolbar converge even if a prior response dropped.
export interface HistoryResult {
  applied: boolean;
  full: boolean;
  faces: number[];
  colors: number[];
  segments: Segment[] | null;
  segmentLabels: number[] | null;
  faceColors: number[] | null;
  canUndo: boolean;
  canRedo: boolean;
}

/// Stack state for enabling toolbar buttons on load / after a stroke.
export interface HistoryState {
  canUndo: boolean;
  canRedo: boolean;
  undoDepth: number;
  redoDepth: number;
  bytes: number;
}

export interface ColorEntry {
  name: string;
  color: [number, number, number];
}

// Default AMS palette from Bambu Lab
export const AMS_PALETTE: ColorEntry[] = [
  { name: "White", color: [255, 255, 255] },
  { name: "Black", color: [0, 0, 0] },
  { name: "Red", color: [255, 0, 0] },
  { name: "Orange", color: [255, 128, 0] },
  { name: "Yellow", color: [255, 255, 0] },
  { name: "Green", color: [0, 176, 80] },
  { name: "Cyan", color: [0, 176, 240] },
  { name: "Blue", color: [0, 32, 255] },
  { name: "Purple", color: [112, 48, 160] },
  { name: "Pink", color: [255, 192, 203] },
  { name: "Brown", color: [139, 69, 19] },
  { name: "Gray", color: [128, 128, 128] },
  { name: "Silver", color: [192, 192, 192] },
  { name: "Gold", color: [255, 215, 0] },
  { name: "Desert Tan", color: [210, 180, 140] },
];
