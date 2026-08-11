/**
 * Per-algorithm segmentation stage plans + canonical-key resolver.
 *
 * The backend's `segment-progress` event carries a stable stage key that
 * identifies *which* phase of the algorithm is running; this module is the
 * single source of truth for how those keys map to a localised "Stage X/Y"
 * line for the ProgressBar.
 *
 * Why the plan lives on the frontend (not the backend):
 *   - The total number of stages depends on the algorithm kind, which the
 *     frontend already knows (the panel's selected kind, or the persisted
 *     `lastSegmentKind` for import auto-segment).
 *   - Localisation belongs here — backend stage strings are stable machine
 *     keys, not user-facing copy.
 *   - The `indeterminate` flag is a UX concern (which phases warrant a
 *     spinner because the algorithm emits no sub-fraction for them) that
 *     does not belong in the Rust pipeline.
 */

import type { AlgorithmKind } from "./types/segment";

export interface StageDef {
  /** Canonical stage key emitted by the backend. */
  key: string;
  /** i18n key for the localised stage label. */
  labelKey: string;
  /**
   * True if the algorithm does not emit sub-fraction progress during this
   * phase, so the ProgressBar should show the indeterminate bar + spinner
   * instead of a frozen percentage. Use this for stages whose only progress
   * event is the entry point (k-means loop, merge loop, etc.).
   */
  indeterminate: boolean;
}

export const SEGMENT_STAGE_PLANS: Record<AlgorithmKind, StageDef[]> = {
  dihedral: [
    { key: "dihedral:edges", labelKey: "segStage.dihedral.edges", indeterminate: false },
    { key: "dihedral:regions", labelKey: "segStage.dihedral.regions", indeterminate: false },
    { key: "dihedral:merge", labelKey: "segStage.dihedral.merge", indeterminate: false },
    { key: "dihedral:finalize", labelKey: "segStage.dihedral.finalize", indeterminate: false },
  ],
  shapeDiameter: [
    { key: "sdf:sample", labelKey: "segStage.sdf.sample", indeterminate: false },
    { key: "sdf:cluster", labelKey: "segStage.sdf.cluster", indeterminate: true },
    { key: "sdf:split", labelKey: "segStage.sdf.split", indeterminate: true },
  ],
  curvatureKMeans: [
    { key: "curv:features", labelKey: "segStage.curv.features", indeterminate: false },
    { key: "curv:kmeans", labelKey: "segStage.curv.kmeans", indeterminate: true },
    { key: "curv:refine", labelKey: "segStage.curv.refine", indeterminate: true },
  ],
  sdfGraphCut: [
    { key: "sdf:sample", labelKey: "segStage.sdf.sample", indeterminate: false },
    { key: "sdf:gmm", labelKey: "segStage.sdf.gmm", indeterminate: true },
    { key: "sdf:graphcut", labelKey: "segStage.sdf.graphcut", indeterminate: true },
    { key: "sdf:connectivity", labelKey: "segStage.sdf.connectivity", indeterminate: true },
  ],
  concavity: [
    { key: "concav:topology", labelKey: "segStage.concav.topology", indeterminate: true },
    { key: "concav:laplacian", labelKey: "segStage.concav.laplacian", indeterminate: true },
    { key: "concav:field", labelKey: "segStage.concav.field", indeterminate: true },
    { key: "concav:refine", labelKey: "segStage.concav.refine", indeterminate: true },
  ],
  // V-HACD convex decomposition: voxelize + hierarchical ACD, then assign each
  // triangle to the nearest hull centroid. Both phases are indeterminate (VHACD
  // emits no sub-fraction).
  convexDecomposition: [
    { key: "vhacd:decompose", labelKey: "segStage.vhacd.decompose", indeterminate: true },
    { key: "vhacd:assign", labelKey: "segStage.vhacd.assign", indeterminate: true },
    { key: "finalize:merge", labelKey: "segStage.finalize.merge", indeterminate: true },
  ],
  // Curve skeleton: same V-HACD pass, then group each chain between joints into
  // one limb-level region.
  curveSkeleton: [
    { key: "skeleton:decompose", labelKey: "segStage.skeleton.decompose", indeterminate: true },
    { key: "skeleton:limbs", labelKey: "segStage.skeleton.limbs", indeterminate: true },
    { key: "finalize:merge", labelKey: "segStage.finalize.merge", indeterminate: true },
  ],
};

/**
 * When curvatureKMeans runs with `useSdf: true`, the feature-building phase
 * internally calls `compute_sdf`, which emits the key `"sdf:sample"`. That
 * key is the same one ShapeDiameter uses for its own sampling phase, but in
 * the curvature context it should be displayed as the features stage. This
 * alias map keeps the backend contract simple (one physical stage = one
 * canonical key) while letting the renderer treat SDF sampling as part of
 * feature construction when appropriate.
 */
const STAGE_ALIASES: Record<string, string> = {
  "sdf:sample": "curv:features",
};

export interface ResolvedStage {
  /** 1-based index of the current stage within the active algorithm's plan. */
  index: number;
  /** Total number of stages in the active algorithm's plan. */
  total: number;
  /** i18n key for the localised stage label. */
  labelKey: string;
  /** Whether the algorithm emits no sub-fraction progress during this stage. */
  indeterminate: boolean;
  /** True if the algorithm has reported completion (`"done"` key). */
  done: boolean;
}

/**
 * Resolve a raw `stage` key from the `segment-progress` event into a
 * displayable stage description. The caller is expected to translate
 * `labelKey` via `useT()` and format `"Stage {index}/{total} · {label}"`.
 */
export function resolveSegmentStage(
  kind: AlgorithmKind,
  rawKey: string,
): ResolvedStage {
  const plan = SEGMENT_STAGE_PLANS[kind];
  if (rawKey === "done") {
    return {
      index: plan.length,
      total: plan.length,
      labelKey: "segStage.done",
      indeterminate: false,
      done: true,
    };
  }
  const normalised =
    kind === "curvatureKMeans" ? (STAGE_ALIASES[rawKey] ?? rawKey) : rawKey;
  const idx = plan.findIndex((s) => s.key === normalised);
  if (idx >= 0) {
    const s = plan[idx];
    return {
      index: idx + 1,
      total: plan.length,
      labelKey: s.labelKey,
      indeterminate: s.indeterminate,
      done: false,
    };
  }
  // Unknown key — fall back to a single generic "working" stage so the
  // spinner still shows something rather than a blank line. This is the
  // expected behaviour for the legacy `auto_segment` / `auto_segment_smart`
  // commands that are no longer wired to the new UI but still emit
  // `segment-progress` events if invoked.
  return {
    index: 1,
    total: 1,
    labelKey: "segStage.working",
    indeterminate: true,
    done: false,
  };
}
