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
}

export const DEFAULT_ALGORITHM_PARAMS: AlgorithmParams = {
  dihedral: { angleThreshold: DEFAULT_DIHEDRAL_ANGLE },
  shapeDiameter: { k: 0 },
  curvatureKMeans: { k: 6, smoothingIters: 2, useSdf: true, creaseThresholdDeg: 20 },
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
  }
}

const clamp = (v: number, lo: number, hi: number) => Math.min(hi, Math.max(lo, v));
const isNum = (v: unknown): v is number => typeof v === "number" && Number.isFinite(v);

export const ALGORITHM_KINDS: AlgorithmKind[] = [
  "curvatureKMeans",
  "shapeDiameter",
  "dihedral",
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
  };
  if (p.dihedral && isNum(p.dihedral.angleThreshold)) {
    out.dihedral.angleThreshold = clamp(p.dihedral.angleThreshold, 1, 179);
  }
  if (p.shapeDiameter && isNum(p.shapeDiameter.k)) {
    out.shapeDiameter.k = clamp(Math.round(p.shapeDiameter.k), 0, 24);
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
  return out;
}
