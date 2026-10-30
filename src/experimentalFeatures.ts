/**
 * Registry of experimental features gated behind the Preferences dialog.
 *
 * Single machine-readable source of truth: the Preferences dialog renders one
 * checkbox per entry, the store persists one boolean per id, and UI code reads
 * the record to show/hide experimental controls. Nothing else may hard-code an
 * id — add a feature by adding one entry here, one i18n pair, and one gate.
 *
 * Everything defaults to false (hidden): the default UI only exposes mature
 * features. Enabling is opt-in per feature, persisted across sessions via
 * cym-prefs (see PersistedPrefs / partialize / mergePrefs in appStore.ts —
 * the custom merge silently drops unclaimed fields, so registry, store field,
 * initial state, partialize and merge must move together).
 */

export type ExperimentalFeatureId =
  | "seedDetectPlanar"
  | "seedDetectMultiview"
  | "seedDetectCrossSection"
  | "seedWeightSliders";

export interface ExperimentalFeatureDef {
  id: ExperimentalFeatureId;
  /** i18n KEY for the checkbox label (M8: store keys, not literals). */
  labelKey: string;
  /** i18n KEY for the one-line explanation under the label. */
  hintKey: string;
}

export const EXPERIMENTAL_FEATURES: readonly ExperimentalFeatureDef[] = [
  {
    id: "seedDetectPlanar",
    labelKey: "exp.seedDetectPlanar",
    hintKey: "exp.seedDetectPlanarHint",
  },
  {
    id: "seedDetectMultiview",
    labelKey: "exp.seedDetectMultiview",
    hintKey: "exp.seedDetectMultiviewHint",
  },
  {
    id: "seedDetectCrossSection",
    labelKey: "exp.seedDetectCrossSection",
    hintKey: "exp.seedDetectCrossSectionHint",
  },
  {
    id: "seedWeightSliders",
    labelKey: "exp.seedWeightSliders",
    hintKey: "exp.seedWeightSlidersHint",
  },
];

/** In-code defaults: every experimental feature starts hidden. */
export const DEFAULT_EXPERIMENTAL_FEATURES: Record<ExperimentalFeatureId, boolean> = {
  seedDetectPlanar: false,
  seedDetectMultiview: false,
  seedDetectCrossSection: false,
  seedWeightSliders: false,
};

export function isExperimentalFeatureId(v: unknown): v is ExperimentalFeatureId {
  return (
    v === "seedDetectPlanar" ||
    v === "seedDetectMultiview" ||
    v === "seedDetectCrossSection" ||
    v === "seedWeightSliders"
  );
}

/**
 * Whitelist + type validation applied on every rehydrate (same discipline as
 * mergePrefs for the scalar prefs): a corrupted or hand-edited payload can
 * only degrade to the in-code defaults, never produce an unknown id or a
 * non-boolean override. Unknown ids are dropped (a payload from a newer build
 * must not smuggle state into an older one); known ids must be booleans.
 */
export function sanitizeExperimentalFeatures(
  v: unknown,
): Record<ExperimentalFeatureId, boolean> {
  const next: Record<ExperimentalFeatureId, boolean> = {
    ...DEFAULT_EXPERIMENTAL_FEATURES,
  };
  if (v && typeof v === "object") {
    for (const [key, value] of Object.entries(v as Record<string, unknown>)) {
      if (isExperimentalFeatureId(key) && typeof value === "boolean") {
        next[key] = value;
      }
    }
  }
  return next;
}
