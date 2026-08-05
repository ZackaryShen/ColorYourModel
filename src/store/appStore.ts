import { create, type StateCreator } from "zustand";
import { persist, createJSONStorage, type StateStorage } from "zustand/middleware";
import { MeshData, PaintTool, PaintSnapshot, Segment } from "../types/mesh";
import type { Lang } from "../i18n";

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

  // History (paint undo/redo)
  undoStack: PaintSnapshot[];
  redoStack: PaintSnapshot[];

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
  setSelectedSegment: (id: number | null) => void;
  setHoveredSegment: (id: number | null) => void;
  setSnapEnabled: (enabled: boolean) => void;
  setSegmentView: (enabled: boolean) => void;
  setToast: (msg: string | null) => void;
  pushUndo: (snapshot: PaintSnapshot) => void;
  popUndo: () => PaintSnapshot | null;
  pushRedo: (snapshot: PaintSnapshot) => void;
  popRedo: () => PaintSnapshot | null;
  // Atomic undo/redo of paint strokes. undoPaint pops the last pre-stroke
  // snapshot and pushes the CURRENT state to redo; redoPaint does the inverse
  // WITHOUT clearing the undo stack (redo may have further steps ahead).
  undoPaint: () => PaintSnapshot | null;
  redoPaint: () => PaintSnapshot | null;
  setStatusMessage: (msg: string) => void;
  setLoading: (loading: boolean) => void;
  setImportProgress: (progress: number, stage: string) => void;
  setLanguage: (lang: Lang) => void;
  setLastPaintDebug: (s: string | null) => void;
  setHoverProbe: (s: string | null) => void;
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

  undoStack: [],
  redoStack: [],

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

  setMeshData: (data) =>
    set({
      meshData: data,
      isLoaded: true,
      segments: data.segments.map((s) => ({
        id: s.id,
        name: s.name,
        color: s.color,
        faceCount: s.faceCount,
      })),
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
              segments: segments.map((s) => ({
                id: s.id,
                name: s.name,
                color: s.color,
                faceCount: s.faceCount,
              })),
              // Apply updated face colors when provided (length must match).
              ...(faceColors && faceColors.length === state.meshData.faceColors.length
                ? { faceColors }
                : {}),
            }
          : null,
      // Top-level segments slice kept for SegmentsPanel / other UI consumers
      // that read it independently of meshData.
      segments: segments.map((s) => ({
        id: s.id,
        name: s.name,
        color: s.color,
        faceCount: s.faceCount,
      })),
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
  pushUndo: (snapshot) =>
    set((state) => ({
      undoStack: [...state.undoStack.slice(-19), snapshot],
      redoStack: [],
    })),
  popUndo: () => {
    let popped: PaintSnapshot | null = null;
    set((state) => {
      if (state.undoStack.length === 0) return {};
      const idx = state.undoStack.length - 1;
      popped = state.undoStack[idx];
      return { undoStack: state.undoStack.slice(0, idx) };
    });
    return popped;
  },
  pushRedo: (snapshot) =>
    set((state) => ({
      redoStack: [...state.redoStack.slice(-19), snapshot],
    })),
  popRedo: () => {
    let popped: PaintSnapshot | null = null;
    set((state) => {
      if (state.redoStack.length === 0) return {};
      const idx = state.redoStack.length - 1;
      popped = state.redoStack[idx];
      return { redoStack: state.redoStack.slice(0, idx) };
    });
    return popped;
  },
  undoPaint: () => {
    let result: PaintSnapshot | null = null;
    set((state) => {
      if (state.undoStack.length === 0 || !state.meshData) return {};
      const idx = state.undoStack.length - 1;
      const prev = state.undoStack[idx];
      const current: PaintSnapshot = {
        faceColors: Uint8Array.from(state.meshData.faceColors),
        segmentLabels: Uint32Array.from(state.meshData.segmentLabels),
      };
      result = prev;
      return {
        undoStack: state.undoStack.slice(0, idx),
        redoStack: [...state.redoStack, current],
      };
    });
    return result;
  },
  redoPaint: () => {
    let result: PaintSnapshot | null = null;
    set((state) => {
      if (state.redoStack.length === 0 || !state.meshData) return {};
      const idx = state.redoStack.length - 1;
      const next = state.redoStack[idx];
      const current: PaintSnapshot = {
        faceColors: Uint8Array.from(state.meshData.faceColors),
        segmentLabels: Uint32Array.from(state.meshData.segmentLabels),
      };
      result = next;
      return {
        redoStack: state.redoStack.slice(0, idx),
        // Do NOT clear undo: redo may have more steps ahead.
        undoStack: [...state.undoStack, current],
      };
    });
    return result;
  },
  setStatusMessage: (msg) => set({ statusMessage: msg }),
  setLoading: (loading) => set({ isLoading: loading }),
  setImportProgress: (progress, stage) => set({ importProgress: progress, importStage: stage }),
  setLanguage: (lang) => set({ language: lang }),
  setLastPaintDebug: (s) => set({ lastPaintDebug: s }),
  setHoverProbe: (s) => set({ hoverProbe: s }),
});

export const useAppStore = create<AppStore>()(
  persist<AppStore, [], [], PersistedPrefs>(createAppState, {
    name: PREFS_KEY,
    version: 1,
    storage: createJSONStorage<PersistedPrefs>(pickStorage),
    // Whitelist — anything not listed here is intentionally session-scoped.
    // Excluded on purpose: meshData / isLoaded / segments / undoStack /
    // redoStack (large + stale on reload), activeTool / selectedSegment /
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
    }),
    merge: mergePrefs,
  })
);
