import { create, type StateCreator } from "zustand";
import { persist, createJSONStorage, type StateStorage } from "zustand/middleware";
import { MeshData, PaintTool, Segment, SeedPoint, PlanarRegion, MultiViewRegion, CrossSectionRegion, EyeRegion } from "../types/mesh";
import type { PersistedExportSelection } from "../types/export";
import type { Lang } from "../i18n";
import { log } from "../utils/logger";
import {
  sanitizeAlgorithmParams,
  isAlgorithmKind,
  type AlgorithmParams,
  type AlgorithmKind,
} from "../types/segment";

interface AppStore {
  // Mesh
  meshData: MeshData | null;
  isLoaded: boolean;

  // Tool
  activeTool: PaintTool;
  brushRadius: number;
  brushStrength: number;
  brushFalloff: "linear" | "smooth" | "step";
  /** Material shading mode (iteration 18 hotfix + iteration 19). "flat" =
   *  meshBasicMaterial — exact per-face color, no lighting, ideal for final
   *  colour verification. "shaded" = meshLambertMaterial — Lambert diffuse,
   *  enough 3D shape readability without PBR complexity. */
  shadingMode: "flat" | "shaded";
  /** App theme (iteration 20; extended in 21). Drives `<html data-theme>` and
   *  therefore every CSS variable. Since iteration 21 the 3D canvas follows it
   *  too (`--bg-canvas`), together with the 3D overlay colours in Viewport. */
  theme: "dark" | "light";

  // Color
  currentColor: [number, number, number, number];

  // Segmentation
  segments: Segment[];
  selectedSegment: number | null;
  /** Segment currently under the cursor (paint view near-highlight / segment
   *  view hover-outline). Null when not hovering a segment. */
  hoveredSegment: number | null;
  snapEnabled: boolean;
  segmentView: boolean;

  // Transient toast (e.g. "添加分区成功"). Auto-cleared by the Toast component.
  toast: string | null;

  // History (backend-owned undo/redo). `canUndo`/`canRedo` drive the toolbar
  // buttons; `markHistoryDirty` is an optimistic hint after a paint stroke;
  // `resetHistory` clears both flags on model (re)load.
  canUndo: boolean;
  canRedo: boolean;

  // i18n
  language: Lang;

  // Status
  statusMessage: string;
  isLoading: boolean;
  importProgress: number;
  importStage: string;

  // In-app debug HUD (visible paint/pick diagnostics — replaces the F12 console
  // which is unavailable in Tauri release builds, iteration 14).
  lastPaintDebug: string | null;
  // Live hover-state probe (iteration 30): updated on EVERY pointermove so the
  // user can watch hoveredSegment change in real time inside a release build.
  hoverProbe: string | null;

  // Last export dialog selection (persisted so re-exports don't re-pick).
  lastExportSelection: PersistedExportSelection | null;

  // Intelligent-segmentation parameters (persisted so the panel opens where the
  // user left it, and so model import can auto-segment with the user's last
  // choice instead of a hard-coded 30° dihedral). `lastAlgorithmParams` keeps a
  // per-kind record so switching algorithms in the dialog and back does not lose
  // tuned sliders; `lastSegmentKind` remembers which algorithm was last run.
  lastAlgorithmParams: AlgorithmParams | null;
  lastSegmentKind: AlgorithmKind | null;

  // Transient loading-status discriminator. The shared ProgressBar consults
  // this to decide whether to render the import UI or the segment UI. Both
  // listener events flip isLoading, but they want different stage plans —
  // the segment UI needs a canonical stage key + the active algorithm kind
  // to render "Stage X/Y: …", while the import UI just shows the raw loader
  // stage string.
  loadingKind: "import" | "segment";
  segmentProgress: number;
  segmentStage: string;
  // The algorithm kind the segmentation is currently running for. Set by
  // both the panel (IntelligentSegmentPanel.run) and the import auto-segment
  // (Toolbar.handleImport) before invoking, so the ProgressBar can pick the
  // correct stage plan regardless of which flow triggered the work.
  segmentStageKind: AlgorithmKind | null;

  // Actions
  setMeshData: (data: MeshData) => void;
  updateSegmentLabels: (labels: number[], segments: Segment[], faceColors?: number[]) => void;
  /** Write back the colors a paint command just produced into the CANONICAL
   *  `meshData.faceColors`. Mutates in place and deliberately does NOT call
   *  `set()`: the `meshData` object reference must stay identical so CameraFit /
   *  buildGeometry / paintColorArray never re-run (iteration 18, B3). Before
   *  this existed, paint only touched the GPU color attribute, so every undo
   *  snapshot was stale, redo restored nothing, and `finalizeSegment` recomputed
   *  from stale colors — wiping freshly painted faces (iteration 18, B2). */
  applyFaceColors: (updatedFaces: number[], updatedColors: number[][]) => void;
  setActiveTool: (tool: PaintTool) => void;
  setBrushRadius: (r: number) => void;
  setBrushStrength: (s: number) => void;
  setBrushFalloff: (f: "linear" | "smooth" | "step") => void;
  setShadingMode: (m: "flat" | "shaded") => void;
  setTheme: (t: "dark" | "light") => void;
  setCurrentColor: (c: [number, number, number, number]) => void;
  setSegments: (segments: Segment[]) => void;
  /** Replace segment *metadata* only, leaving `segmentLabels` untouched.
   *  Rename is the first operation that changes what a region is called
   *  without changing which faces belong to it, and `updateSegmentLabels`
   *  would force the caller to hand back a labels array it never received. */
  setSegmentMetadata: (segments: Segment[]) => void;
  setSelectedSegment: (id: number | null) => void;
  setHoveredSegment: (id: number | null) => void;
  setSnapEnabled: (enabled: boolean) => void;
  setSegmentView: (enabled: boolean) => void;
  setToast: (msg: string | null) => void;
  // History flags + actions (frontend owns no timeline; see useHistory bridge).
  setHistoryFlags: (canUndo: boolean, canRedo: boolean) => void;
  markHistoryDirty: () => void;
  resetHistory: () => void;
  setStatusMessage: (msg: string) => void;

  // Seeded watershed tool (iteration 50): points the user clicks on the mesh.
  // Each seed grows into one region; the backend fills the rest by geometry.
  seedPoints: SeedPoint[];
  addSeedPoint: (p: SeedPoint) => void;
  clearSeedPoints: () => void;
  /** Iteration 51: eraser-mode toggle. While ON, clicking the mesh removes the
   *  seed nearest the click (within a tolerance radius) instead of adding a new
   *  one — so a misplaced seed can be fixed without clearing all of them.
   *  Transient on purpose: NOT persisted (see partialize whitelist). */
  seedEraseMode: boolean;
  removeSeedPoint: (index: number) => void;
  setSeedEraseMode: (v: boolean) => void;
  /** Pick-for-Eye sub-mode of the Seed tool. While ON, LEFT-click on the mesh
   *  sets `selectedSegment` to the hit face's partition label (and auto-exits
   *  pick mode) instead of placing a seed. Empty-space click clears selection.
   *  Replaces the previous "open SegmentsPanel and click a row" path for users
   *  who want to run eye-detect. Transient: NOT persisted (see partialize). */
  seedPickMode: boolean;
  setSeedPickMode: (v: boolean) => void;
  /** Iteration 52: advisory seed suggestions from the backend (weighted FPS over
   *  face centroids, biased to region interiors). Shown as ghost markers; the
   *  user accepts one by clicking it (moves to `seedPoints`) or ignores it. They
   *  never reach `seed_grow` until accepted, so a bad suggestion is harmless.
   *  Transient: NOT persisted. */
  suggestedSeeds: SeedPoint[];
  setSuggestedSeeds: (seeds: SeedPoint[]) => void;
  acceptSuggestedSeed: (index: number) => void;
  clearSuggestedSeeds: () => void;
  /** Layer 1 planar-region detection (`docs/09`): the flat patches the backend
   *  found, kept so the Viewport can outline each one with boundary lines. The
   *  region seeds are ALSO pushed into `suggestedSeeds` (ghost markers) so the
   *  user can accept them into `seed_grow` via the existing iter58-64 path.
   *  Transient: NOT persisted. */
  planarRegions: PlanarRegion[];
  setPlanarRegions: (regions: PlanarRegion[]) => void;
  clearPlanarRegions: () => void;
  /** 是否在 Viewport 中绘制 Layer 1 平面边界（默认 true）。切换只是显隐，数据保留。 */
  planarRegionsVisible: boolean;
  togglePlanarRegionsVisible: () => void;
  multiviewRegions: MultiViewRegion[];
  setMultiviewRegions: (regions: MultiViewRegion[]) => void;
  clearMultiviewRegions: () => void;
  multiviewRegionsVisible: boolean;
  toggleMultiviewRegionsVisible: () => void;
  crossSectionRegions: CrossSectionRegion[];
  setCrossSectionRegions: (regions: CrossSectionRegion[]) => void;
  clearCrossSectionRegions: () => void;
  crossSectionRegionsVisible: boolean;
  toggleCrossSectionRegionsVisible: () => void;
  /** Layer 5 (docs/10) eye-region semantic detection. The four sub-regions
   *  (globe / sclera / eyelid / socket) the backend found inside the user's eye
   *  ROI, kept so the Viewport can outline each semantic class with its own
   *  colour. Transient: NOT persisted. */
  eyeRegions: EyeRegion[];
  setEyeRegions: (regions: EyeRegion[]) => void;
  clearEyeRegions: () => void;
  eyeRegionsVisible: boolean;
  toggleEyeRegionsVisible: () => void;
  /** 只显当前层：传入要保留下来的层名，其余关闭。null 表示全开。 */
  setOnlyVisible: (which: "planar" | "multiview" | "crosssection" | "eye" | null) => void;
  setLoading: (loading: boolean) => void;
  setImportProgress: (progress: number, stage: string) => void;
  setLanguage: (lang: Lang) => void;
  setLastPaintDebug: (s: string | null) => void;
  setHoverProbe: (s: string | null) => void;
  setLastExportSelection: (s: PersistedExportSelection | null) => void;
  setLastAlgorithmParams: (p: AlgorithmParams) => void;
  setLastSegmentKind: (k: AlgorithmKind) => void;
  setSegmentProgress: (progress: number, stage: string) => void;
  setSegmentStageKind: (kind: AlgorithmKind) => void;
  setLoadingKind: (kind: "import" | "segment") => void;
}

// ── Preference persistence (iteration 21) ─────────────────────────────────
// Iteration 20 only persisted `theme` through a hand-rolled localStorage pair.
// Every other preference (language, shading, brush params, current color) was
// reset on each launch. This block replaces that with zustand's `persist`
// middleware over an explicit whitelist.
//
// Design constraints established by the adversarial REFUTE pass:
//   - B1: do NOT use `migrate` for the legacy `cym-theme` key. `migrate` only
//     runs when the NEW key already exists, so on a first launch after upgrade
//     it never fires and the user's dark/light choice would be silently
//     overwritten by `prefers-color-scheme`. The legacy key is therefore read
//     in the *initializer* (see `theme:` below), which persist keeps whenever
//     no new payload exists.
//   - M3: brushStrength is clamped to [0.1, 1.0] to match the slider range in
//     BrushSettings (`min=0.1 max=1.0`); an out-of-range persisted value would
//     otherwise render a slider thumb that cannot be reproduced by dragging.
//   - M4: never let a disabled/partitioned localStorage short-circuit persist.
//     Falling back to an in-memory StateStorage keeps the whole rehydrate +
//     validation pipeline running, so behaviour is identical minus durability.
//   - M5: value-range validation lives in `merge`, NOT in `onRehydrateStorage`.
//     The latter runs synchronously *during* `create()`, when `useAppStore` is
//     still in its temporal dead zone — touching the store there throws.
//   - M7: `setTheme` keeps mirroring the legacy `cym-theme` key so downgrading
//     to an older build does not lose the theme choice.
const PREFS_KEY = "cym-prefs";
const LEGACY_THEME_KEY = "cym-theme";

/** Subset of the store that is actually written to disk. Everything else
 *  (mesh data, undo history, transient UI state) is deliberately excluded. */
interface PersistedPrefs {
  theme: "dark" | "light";
  language: Lang;
  shadingMode: "flat" | "shaded";
  brushRadius: number;
  brushStrength: number;
  brushFalloff: "linear" | "smooth" | "step";
  currentColor: [number, number, number, number];
  snapEnabled: boolean;
  lastExportSelection: PersistedExportSelection | null;
  // Intelligent-segmentation prefs (see the AppStore field comments above).
  lastAlgorithmParams: AlgorithmParams | null;
  lastSegmentKind: AlgorithmKind | null;
}

/** Initial theme when no persisted payload exists: legacy key → OS preference
 *  → dark. All storage/matchMedia access is guarded; this project has no
 *  ErrorBoundary, so a throw here would white-screen the app. */
function initialTheme(): "dark" | "light" {
  try {
    const stored = localStorage.getItem(LEGACY_THEME_KEY);
    if (stored === "light" || stored === "dark") return stored;
  } catch {
    /* storage unavailable — fall through */
  }
  try {
    if (
      typeof window !== "undefined" &&
      window.matchMedia &&
      window.matchMedia("(prefers-color-scheme: light)").matches
    ) {
      return "light";
    }
  } catch {
    /* matchMedia unavailable */
  }
  return "dark";
}

function mirrorLegacyTheme(t: "dark" | "light") {
  try {
    localStorage.setItem(LEGACY_THEME_KEY, t);
  } catch {
    /* ignore persist failure */
  }
}

// M4: in-memory fallback so persist always completes even without localStorage.
const memoryBacking = new Map<string, string>();
const memoryStorage: StateStorage = {
  getItem: (name) => memoryBacking.get(name) ?? null,
  setItem: (name, value) => {
    memoryBacking.set(name, value);
  },
  removeItem: (name) => {
    memoryBacking.delete(name);
  },
};

function pickStorage(): StateStorage {
  try {
    const probe = "__cym_probe__";
    localStorage.setItem(probe, "1");
    localStorage.removeItem(probe);
    return localStorage;
  } catch {
    return memoryStorage;
  }
}

const clamp = (v: number, lo: number, hi: number) => Math.min(hi, Math.max(lo, v));
const isNum = (v: unknown): v is number => typeof v === "number" && Number.isFinite(v);

/**
 * Narrow backend segment metadata to the fields the store keeps.
 *
 * This projection existed verbatim in three places, which made it three chances
 * to forget one when the Rust DTO grows a field — the copy is a *filter*, not a
 * spread, so anything new is dropped in silence and only shows up as a value
 * that is always undefined in the UI. One function means adding a field is one
 * edit, and the omission is at least visible in a single place.
 */
const projectSegments = (segments: Segment[]): Segment[] =>
  segments.map((s) => ({
    id: s.id,
    name: s.name,
    color: s.color,
    faceCount: s.faceCount,
  }));

/** M5: whitelist + range validation applied on every rehydrate. A corrupted or
 *  hand-edited payload can only ever degrade to the in-code defaults, never
 *  produce NaN sliders, unknown enum values or a broken color tuple. */
function mergePrefs(persisted: unknown, current: AppStore): AppStore {
  const p = (persisted ?? {}) as Partial<PersistedPrefs>;
  const next: AppStore = { ...current };

  if (p.theme === "dark" || p.theme === "light") next.theme = p.theme;
  if (p.language === "zh" || p.language === "en") next.language = p.language;
  if (p.shadingMode === "flat" || p.shadingMode === "shaded") next.shadingMode = p.shadingMode;
  if (p.brushFalloff === "linear" || p.brushFalloff === "smooth" || p.brushFalloff === "step") {
    next.brushFalloff = p.brushFalloff;
  }
  if (isNum(p.brushRadius)) next.brushRadius = clamp(p.brushRadius, 0.5, 200);
  if (isNum(p.brushStrength)) next.brushStrength = clamp(p.brushStrength, 0.1, 1.0);
  if (typeof p.snapEnabled === "boolean") next.snapEnabled = p.snapEnabled;
  if (Array.isArray(p.currentColor) && p.currentColor.length === 4 && p.currentColor.every(isNum)) {
    const c = p.currentColor.map((v) => clamp(Math.round(v), 0, 255));
    next.currentColor = [c[0], c[1], c[2], c[3]];
  }
  // Value-range validation for the persisted export selection. A hand-edited
  // payload must never produce a dialog stuck on an unknown machine id, so
  // only structurally sound selections survive rehydration; anything else is
  // dropped and the dialog starts from defaults.
  if (p.lastExportSelection && typeof p.lastExportSelection === "object") {
    const s = p.lastExportSelection;
    if (
      typeof s.machineId === "string" &&
      s.machineId.length > 0 &&
      typeof s.nozzleDiameter === "string" &&
      s.nozzleDiameter.length > 0 &&
      typeof s.processName === "string" &&
      s.processName.length > 0 &&
      Array.isArray(s.filamentNames) &&
      s.filamentNames.every((f) => typeof f === "string") &&
      typeof s.targetSlicer === "string" &&
      (s.targetSlicer === "snapmaker_orca" || s.targetSlicer === "orcaslicer")
    ) {
      next.lastExportSelection = {
        machineId: s.machineId,
        nozzleDiameter: s.nozzleDiameter,
        processName: s.processName,
        filamentNames: s.filamentNames.slice(0, 8),
        targetSlicer: s.targetSlicer,
      };
    }
  }
  // Intelligent-segmentation prefs: a corrupted payload must never produce NaN
  // sliders or an unknown algorithm kind. sanitizeAlgorithmParams already clamps
  // every field to its valid range and falls back to in-code defaults; an
  // unknown lastSegmentKind is simply dropped so the panel falls back to its own
  // default. Either way the stored value only ever degrades, never breaks.
  if (p.lastAlgorithmParams && typeof p.lastAlgorithmParams === "object") {
    next.lastAlgorithmParams = sanitizeAlgorithmParams(p.lastAlgorithmParams);
  }
  if (isAlgorithmKind(p.lastSegmentKind)) {
    next.lastSegmentKind = p.lastSegmentKind;
  }
  return next;
}

const createAppState: StateCreator<AppStore, [], []> = (set, get) => ({
  meshData: null,
  isLoaded: false,

  activeTool: PaintTool.View,
  brushRadius: 2.0,
  brushStrength: 0.8,
  brushFalloff: "smooth",
  shadingMode: "shaded",
  theme: initialTheme(),

  currentColor: [255, 0, 0, 255],

  segments: [],
  selectedSegment: null,
  hoveredSegment: null,
  snapEnabled: false,
  segmentView: false,
  toast: null,

  canUndo: false,
  canRedo: false,

  language: "zh",

  // M8: stored as an i18n KEY, not a literal. Once `language` is persisted the
  // app can boot in English, and a hard-coded 中文 default would leak through.
  // StatusBar renders it via `t()`, which returns unknown keys verbatim, so
  // runtime messages pushed by setStatusMessage still display unchanged.
  statusMessage: "status.ready",
  isLoading: false,
  importProgress: 0,
  importStage: "",

  lastPaintDebug: null,
  hoverProbe: null,
  lastExportSelection: null,
  lastAlgorithmParams: null,
  lastSegmentKind: null,

  loadingKind: "import",
  segmentProgress: 0,
  segmentStage: "",
  segmentStageKind: null,

  setMeshData: (data) =>
    set({
      meshData: data,
      isLoaded: true,
      canUndo: false,
      canRedo: false,
      segments: projectSegments(data.segments),
      statusMessage: `已加载 ${data.faceCount.toLocaleString()} 个面`,
    }),

  updateSegmentLabels: (labels, segments, faceColors) =>
    set((state) => ({
      meshData:
        state.meshData
          ? {
              ...state.meshData,
              segmentLabels: labels,
              // segments metadata must live inside meshData so that Fill routing
              // (usePaintTool), segment hover highlight (Viewport.tsx), and any
              // future consumer reading meshData can resolve segment info.
              // Previously only written to top-level state slice — always []
              // there (iteration 22 fix: B6 root cause of "Fill acts like brush").
              segments: projectSegments(segments),
              // Apply updated face colors when provided (length must match).
              ...(faceColors && faceColors.length === state.meshData.faceColors.length
                ? { faceColors }
                : {}),
            }
          : null,
      // Top-level segments slice kept for SegmentsPanel / other UI consumers
      // that read it independently of meshData.
      segments: projectSegments(segments),
    })),

  // Metadata-only refresh. Both slices are written for the same reason
  // `updateSegmentLabels` writes both: Fill routing and the hover highlight
  // read `meshData.segments`, while the region panel reads the top-level one,
  // and a rename that reached only one of them would show two different names
  // for the same region depending on where you looked.
  setSegmentMetadata: (segments) =>
    set((state) => ({
      meshData: state.meshData
        ? { ...state.meshData, segments: projectSegments(segments) }
        : null,
      segments: projectSegments(segments),
    })),

  applyFaceColors: (updatedFaces, updatedColors) => {
    const md = get().meshData;
    if (!md) return;
    const fc = md.faceColors;
    const n = Math.min(updatedFaces.length, updatedColors.length);
    for (let i = 0; i < n; i++) {
      const o = updatedFaces[i] * 4;
      if (o < 0 || o + 3 >= fc.length) continue;
      const c = updatedColors[i];
      fc[o] = c[0];
      fc[o + 1] = c[1];
      fc[o + 2] = c[2];
      fc[o + 3] = c[3] ?? 255;
    }
  },

  setActiveTool: (tool) => set({ activeTool: tool }),
  setBrushRadius: (r) => set({ brushRadius: clamp(r, 0.5, 200) }),
  // M3: clamp to the BrushSettings slider range so a persisted/scripted value
  // can never produce a thumb position the user cannot reproduce by dragging.
  setBrushStrength: (s) => set({ brushStrength: clamp(s, 0.1, 1.0) }),
  setBrushFalloff: (f) => set({ brushFalloff: f }),
  setShadingMode: (m) => set({ shadingMode: m }),
  setTheme: (t) => {
    mirrorLegacyTheme(t); // M7: keep the legacy key readable by older builds
    set({ theme: t });
  },
  setCurrentColor: (c) => set({ currentColor: c }),
  setSegments: (segments) => set({ segments }),
  setSelectedSegment: (id) => set({ selectedSegment: id }),
  setHoveredSegment: (id) => set({ hoveredSegment: id }),
  setSnapEnabled: (enabled) => set({ snapEnabled: enabled }),
  setSegmentView: (enabled) => set({ segmentView: enabled }),
  setToast: (msg) => set({ toast: msg }),
  setHistoryFlags: (canUndo, canRedo) => set({ canUndo, canRedo }),
  // Optimistic: a successful paint stroke always creates a history entry, so undo
  // becomes available; meanwhile it invalidates any redo branch on the backend.
  markHistoryDirty: () => set({ canUndo: true, canRedo: false }),
  resetHistory: () => set({ canUndo: false, canRedo: false }),
  setStatusMessage: (msg) => set({ statusMessage: msg }),

  seedPoints: [],
  addSeedPoint: (p) => set((s) => ({ seedPoints: [...s.seedPoints, p] })),
  clearSeedPoints: () => set({ seedPoints: [] }),
  seedEraseMode: false,
  removeSeedPoint: (index) =>
    set((s) => ({ seedPoints: s.seedPoints.filter((_, i) => i !== index) })),
  setSeedEraseMode: (v) => set({ seedEraseMode: v }),
  seedPickMode: false,
  setSeedPickMode: (v) => set({ seedPickMode: v }),
  suggestedSeeds: [],
  setSuggestedSeeds: (seeds) => {
    log.info("store", "setSuggestedSeeds", { count: seeds.length, first: seeds[0] });
    set({ suggestedSeeds: seeds });
  },
  acceptSuggestedSeed: (index) => {
    log.info("store", "acceptSuggestedSeed enter", { index });
    set((s) => {
      const picked = s.suggestedSeeds[index];
      if (!picked) {
        log.warn("store", "acceptSuggestedSeed: index out of range", {
          index,
          count: s.suggestedSeeds.length,
        });
        return {};
      }
      log.info("store", "acceptSuggestedSeed: promoting suggestion to real seed", {
        x: picked.x.toFixed(3),
        y: picked.y.toFixed(3),
        z: picked.z.toFixed(3),
      });
      return {
        seedPoints: [...s.seedPoints, picked],
        suggestedSeeds: s.suggestedSeeds.filter((_, i) => i !== index),
      };
    });
  },
  clearSuggestedSeeds: () => set({ suggestedSeeds: [] }),
  planarRegions: [],
  planarRegionsVisible: true,
  togglePlanarRegionsVisible: () =>
    set((s) => ({ planarRegionsVisible: !s.planarRegionsVisible })),
  setPlanarRegions: (regions) => {
    log.info("store", "setPlanarRegions", { count: regions.length });
    set({ planarRegions: regions });
  },
  clearPlanarRegions: () => set({ planarRegions: [] }),
  multiviewRegions: [],
  multiviewRegionsVisible: true,
  toggleMultiviewRegionsVisible: () =>
    set((s) => ({ multiviewRegionsVisible: !s.multiviewRegionsVisible })),
  setMultiviewRegions: (regions) => {
    log.info("store", "setMultiviewRegions", { count: regions.length });
    set({ multiviewRegions: regions });
  },
  clearMultiviewRegions: () => set({ multiviewRegions: [] }),
  crossSectionRegions: [],
  crossSectionRegionsVisible: true,
  toggleCrossSectionRegionsVisible: () =>
    set((s) => ({ crossSectionRegionsVisible: !s.crossSectionRegionsVisible })),
  setCrossSectionRegions: (regions) => {
    log.info("store", "setCrossSectionRegions", { count: regions.length });
    set({ crossSectionRegions: regions });
  },
  clearCrossSectionRegions: () => set({ crossSectionRegions: [] }),
  eyeRegions: [],
  eyeRegionsVisible: true,
  toggleEyeRegionsVisible: () =>
    set((s) => ({ eyeRegionsVisible: !s.eyeRegionsVisible })),
  setEyeRegions: (regions) => {
    log.info("store", "setEyeRegions", { count: regions.length });
    set({ eyeRegions: regions });
  },
  clearEyeRegions: () => set({ eyeRegions: [] }),
  setOnlyVisible: (which) => {
    // null = 全部恢复；指定层 = 只保留它。
    set({
      planarRegionsVisible: which === null || which === "planar",
      multiviewRegionsVisible: which === null || which === "multiview",
      crossSectionRegionsVisible: which === null || which === "crosssection",
      eyeRegionsVisible: which === null || which === "eye",
    });
  },
  setLoading: (loading) => set({ isLoading: loading }),
  setImportProgress: (progress, stage) => set({ importProgress: progress, importStage: stage }),
  setLanguage: (lang) => set({ language: lang }),
  setLastPaintDebug: (s) => set({ lastPaintDebug: s }),
  setHoverProbe: (s) => set({ hoverProbe: s }),
  setLastExportSelection: (s) => set({ lastExportSelection: s }),
  setLastAlgorithmParams: (p) => set({ lastAlgorithmParams: p }),
  setLastSegmentKind: (k) => set({ lastSegmentKind: k }),
  setSegmentProgress: (progress, stage) =>
    set({ segmentProgress: progress, segmentStage: stage }),
  setSegmentStageKind: (kind) => set({ segmentStageKind: kind }),
  setLoadingKind: (kind) => set({ loadingKind: kind }),
});

export const useAppStore = create<AppStore>()(
  persist<AppStore, [], [], PersistedPrefs>(createAppState, {
    name: PREFS_KEY,
    version: 1,
    storage: createJSONStorage<PersistedPrefs>(pickStorage),
    // Whitelist — anything not listed here is intentionally session-scoped.
    // Excluded on purpose: meshData / isLoaded / segments / canUndo / canRedo
    // (runtime history flags, not durable prefs), activeTool / selectedSegment /
    // hoveredSegment / segmentView / toast / statusMessage / isLoading /
    // importProgress / importStage / lastPaintDebug (transient UI state).
    partialize: (s) => ({
      theme: s.theme,
      language: s.language,
      shadingMode: s.shadingMode,
      brushRadius: s.brushRadius,
      brushStrength: s.brushStrength,
      brushFalloff: s.brushFalloff,
      currentColor: s.currentColor,
      snapEnabled: s.snapEnabled,
      lastExportSelection: s.lastExportSelection,
      lastAlgorithmParams: s.lastAlgorithmParams,
      lastSegmentKind: s.lastSegmentKind,
    }),
    merge: mergePrefs,
  })
);
