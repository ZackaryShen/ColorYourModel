/**
 * Intelligent-segmentation algorithm selection.
 *
 * Mirrors the Rust `SegmentationAlgorithm` enum (src-tauri/src/segment/mod.rs),
 * which is serialized **internally tagged** with camelCased fields. The exact
 * JSON is pinned by `serde_wire_format_is_stable` on the Rust side — if that
 * test fails, this file is the other half that must move with it.
 */
export type SegmentationAlgorithm =
  | { type: "dihedral"; angleThreshold: number }
  | { type: "shapeDiameter"; k: number }
  | {
      type: "curvatureKMeans";
      k: number;
      smoothingIters: number;
      useSdf: boolean;
      creaseThresholdDeg: number;
    }
  | { type: "sdfGraphCut"; k: number }
  | { type: "concavity"; k: number }
  | { type: "convexDecomposition"; maxHulls: number; concavity: number }
  | { type: "curveSkeleton"; maxHulls: number; concavity: number }
  | {
      type: "fhGraph";
      scale: number;
      curvature: number;
      concavity: number;
    };

import type { Segment } from "./mesh";

/**
 * How a region is split. Mirrors the Rust `SplitMethod` enum
 * (src-tauri/src/segment/split.rs), serialized internally tagged with camelCased
 * fields; pinned by `split_method_wire_format_is_stable` on the Rust side.
 * `plane` is reserved for a future viewport cut gesture and is not yet accepted
 * by the backend command.
 */
export type SplitMethod =
  | { type: "plane"; point: [number, number, number]; normal: [number, number, number] }
  | { type: "crease"; thresholdDeg: number };

/** Result of a split, mirroring the Rust `SplitResult`. */
export type SplitResult = {
  segments: Segment[];
  segmentLabels: number[];
  movedFaces: number;
  keptLabel: number;
  newLabel: number;
};

export type AlgorithmKind = SegmentationAlgorithm["type"];

/** Default auto-segmentation applied right after import. */
export const DEFAULT_DIHEDRAL_ANGLE = 30.0;

/**
 * One entry per algorithm, holding the *last used* parameters. Keeping a full
 * record (instead of a single active object) means switching algorithms in the
 * dialog and switching back does not reset the sliders the user just tuned.
 */
export interface AlgorithmParams {
  dihedral: { angleThreshold: number };
  shapeDiameter: { k: number };
  curvatureKMeans: {
    k: number;
    smoothingIters: number;
    useSdf: boolean;
    creaseThresholdDeg: number;
  };
  sdfGraphCut: { k: number };
  concavity: { k: number };
  convexDecomposition: { maxHulls: number; concavity: number };
  curveSkeleton: { maxHulls: number; concavity: number };
  fhGraph: { scale: number; curvature: number; concavity: number };
}

export const DEFAULT_ALGORITHM_PARAMS: AlgorithmParams = {
  dihedral: { angleThreshold: DEFAULT_DIHEDRAL_ANGLE },
  shapeDiameter: { k: 0 },
  curvatureKMeans: { k: 6, smoothingIters: 2, useSdf: true, creaseThresholdDeg: 45 },
  sdfGraphCut: { k: 0 },
  concavity: { k: 0 },
  convexDecomposition: { maxHulls: 0, concavity: 5 },
  curveSkeleton: { maxHulls: 0, concavity: 5 },
  fhGraph: { scale: 0.3, curvature: 1.0, concavity: 1.0 },
};

/** Assemble the IPC payload for the currently selected algorithm. */
export function buildAlgorithm(
  kind: AlgorithmKind,
  params: AlgorithmParams
): SegmentationAlgorithm {
  switch (kind) {
    case "dihedral":
      return { type: "dihedral", angleThreshold: params.dihedral.angleThreshold };
    case "shapeDiameter":
      return { type: "shapeDiameter", k: params.shapeDiameter.k };
    case "curvatureKMeans":
      return {
        type: "curvatureKMeans",
        k: params.curvatureKMeans.k,
        smoothingIters: params.curvatureKMeans.smoothingIters,
        useSdf: params.curvatureKMeans.useSdf,
        creaseThresholdDeg: params.curvatureKMeans.creaseThresholdDeg,
      };
    case "sdfGraphCut":
      return { type: "sdfGraphCut", k: params.sdfGraphCut.k };
    case "concavity":
      return { type: "concavity", k: params.concavity.k };
    case "convexDecomposition":
      return {
        type: "convexDecomposition",
        maxHulls: params.convexDecomposition.maxHulls,
        // `concavity` is authored as a percent (1–20) on the slider; the backend
        // expects the 0..1 fraction VHACD uses.
        concavity: params.convexDecomposition.concavity / 100,
      };
    case "curveSkeleton":
      return {
        type: "curveSkeleton",
        maxHulls: params.curveSkeleton.maxHulls,
        concavity: params.curveSkeleton.concavity / 100,
      };
    case "fhGraph":
      return {
        type: "fhGraph",
        scale: params.fhGraph.scale,
        curvature: params.fhGraph.curvature,
        concavity: params.fhGraph.concavity,
      };
  }
}

const clamp = (v: number, lo: number, hi: number) => Math.min(hi, Math.max(lo, v));
const isNum = (v: unknown): v is number => typeof v === "number" && Number.isFinite(v);

export const ALGORITHM_KINDS: AlgorithmKind[] = [
  "curvatureKMeans",
  "shapeDiameter",
  "dihedral",
  // Iter-45 experimental algorithms (validated head-to-head in the Rust harness:
  // they collapse hundreds of fragments into 4–7 semantic parts). Exposed for
  // the user to A/B visually; the default stays curvatureKMeans until the user
  // picks one, which persists as lastSegmentKind.
  "sdfGraphCut",
  "concavity",
  // Iter-48: V-HACD approximate convex decomposition (pure Rust, parry3d) and the
  // curve-skeleton derived from it. Target the failure mode concavity could not
  // fix on armoured characters — cutting at narrow joints instead of concave seams.
  "convexDecomposition",
  "curveSkeleton",
  // Iter-56: Felzenszwalb-Huttenlocher graph segmentation, ported from SAM3D's
  // final stage. Takes a scale (granularity) instead of a preset k, and reuses
  // the curvature/concavity significance field — a pure-geometric, no-ML cousin
  // of the other algorithms that emerges the part count from the mesh.
  "fhGraph",
];

export function isAlgorithmKind(v: unknown): v is AlgorithmKind {
  return typeof v === "string" && (ALGORITHM_KINDS as string[]).includes(v);
}

/**
 * Range-validate a rehydrated parameter record. Same contract as `mergePrefs`
 * in appStore: a corrupted payload may only ever degrade to the in-code
 * defaults, never produce a NaN slider or a k that makes the backend allocate
 * an absurd number of clusters.
 */
export function sanitizeAlgorithmParams(raw: unknown): AlgorithmParams {
  const d = DEFAULT_ALGORITHM_PARAMS;
  if (!raw || typeof raw !== "object") return d;
  const p = raw as Partial<AlgorithmParams>;
  const out: AlgorithmParams = {
    dihedral: { ...d.dihedral },
    shapeDiameter: { ...d.shapeDiameter },
    curvatureKMeans: { ...d.curvatureKMeans },
    sdfGraphCut: { ...d.sdfGraphCut },
    concavity: { ...d.concavity },
    convexDecomposition: { ...d.convexDecomposition },
    curveSkeleton: { ...d.curveSkeleton },
    fhGraph: { ...d.fhGraph },
  };
  if (p.dihedral && isNum(p.dihedral.angleThreshold)) {
    out.dihedral.angleThreshold = clamp(p.dihedral.angleThreshold, 1, 179);
  }
  if (p.shapeDiameter && isNum(p.shapeDiameter.k)) {
    out.shapeDiameter.k = clamp(Math.round(p.shapeDiameter.k), 0, 24);
  }
  if (p.sdfGraphCut && isNum(p.sdfGraphCut.k)) {
    out.sdfGraphCut.k = clamp(Math.round(p.sdfGraphCut.k), 0, 24);
  }
  if (p.concavity && isNum(p.concavity.k)) {
    out.concavity.k = clamp(Math.round(p.concavity.k), 0, 48);
  }
  if (p.convexDecomposition) {
    const c = p.convexDecomposition;
    if (isNum(c.maxHulls)) {
      out.convexDecomposition.maxHulls = clamp(Math.round(c.maxHulls), 0, 64);
    }
    if (isNum(c.concavity)) {
      out.convexDecomposition.concavity = clamp(c.concavity, 1, 20);
    }
  }
  if (p.curveSkeleton) {
    const c = p.curveSkeleton;
    if (isNum(c.maxHulls)) {
      out.curveSkeleton.maxHulls = clamp(Math.round(c.maxHulls), 0, 64);
    }
    if (isNum(c.concavity)) {
      out.curveSkeleton.concavity = clamp(c.concavity, 1, 20);
    }
  }
  if (p.curvatureKMeans) {
    const c = p.curvatureKMeans;
    if (isNum(c.k)) out.curvatureKMeans.k = clamp(Math.round(c.k), 0, 24);
    if (isNum(c.smoothingIters)) {
      out.curvatureKMeans.smoothingIters = clamp(Math.round(c.smoothingIters), 0, 10);
    }
    if (typeof c.useSdf === "boolean") out.curvatureKMeans.useSdf = c.useSdf;
    if (isNum(c.creaseThresholdDeg)) {
      out.curvatureKMeans.creaseThresholdDeg = clamp(c.creaseThresholdDeg, 1, 90);
    }
  }
  if (p.fhGraph) {
    const c = p.fhGraph;
    if (isNum(c.scale)) out.fhGraph.scale = clamp(c.scale, 0.05, 1);
    if (isNum(c.curvature)) out.fhGraph.curvature = clamp(c.curvature, 0, 2);
    if (isNum(c.concavity)) out.fhGraph.concavity = clamp(c.concavity, 0, 2);
  }
  return out;
}
