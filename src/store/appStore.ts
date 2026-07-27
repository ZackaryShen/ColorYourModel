import { create } from "zustand";
import { MeshData, PaintTool, Segment } from "../types/mesh";
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

  // Color
  currentColor: [number, number, number, number];

  // Segmentation
  segments: Segment[];
  selectedSegment: number | null;
  snapEnabled: boolean;
  segmentView: boolean;

  // History
  undoStack: Uint8Array[];
  redoStack: Uint8Array[];

  // i18n
  language: Lang;

  // Status
  statusMessage: string;
  isLoading: boolean;
  importProgress: number;
  importStage: string;

  // Actions
  setMeshData: (data: MeshData) => void;
  updateSegmentLabels: (labels: number[], segments: Segment[], faceColors?: number[]) => void;
  setActiveTool: (tool: PaintTool) => void;
  setBrushRadius: (r: number) => void;
  setBrushStrength: (s: number) => void;
  setBrushFalloff: (f: "linear" | "smooth" | "step") => void;
  setCurrentColor: (c: [number, number, number, number]) => void;
  setSegments: (segments: Segment[]) => void;
  setSelectedSegment: (id: number | null) => void;
  setSnapEnabled: (enabled: boolean) => void;
  setSegmentView: (enabled: boolean) => void;
  pushUndo: (snapshot: Uint8Array) => void;
  setStatusMessage: (msg: string) => void;
  setLoading: (loading: boolean) => void;
  setImportProgress: (progress: number, stage: string) => void;
  setLanguage: (lang: Lang) => void;
}

export const useAppStore = create<AppStore>((set) => ({
  meshData: null,
  isLoaded: false,

  activeTool: PaintTool.Brush,
  brushRadius: 5.0,
  brushStrength: 0.8,
  brushFalloff: "smooth",

  currentColor: [255, 0, 0, 255],

  segments: [],
  selectedSegment: null,
  snapEnabled: false,
  segmentView: false,

  undoStack: [],
  redoStack: [],

  language: "zh",

  statusMessage: "就绪",
  isLoading: false,
  importProgress: 0,
  importStage: "",

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
              // Apply updated face colors when provided (length must match).
              ...(faceColors && faceColors.length === state.meshData.faceColors.length
                ? { faceColors }
                : {}),
            }
          : null,
      segments: segments.map((s) => ({
        id: s.id,
        name: s.name,
        color: s.color,
        faceCount: s.faceCount,
      })),
    })),

  setActiveTool: (tool) => set({ activeTool: tool }),
  setBrushRadius: (r) => set({ brushRadius: r }),
  setBrushStrength: (s) => set({ brushStrength: s }),
  setBrushFalloff: (f) => set({ brushFalloff: f }),
  setCurrentColor: (c) => set({ currentColor: c }),
  setSegments: (segments) => set({ segments }),
  setSelectedSegment: (id) => set({ selectedSegment: id }),
  setSnapEnabled: (enabled) => set({ snapEnabled: enabled }),
  setSegmentView: (enabled) => set({ segmentView: enabled }),
  pushUndo: (snapshot) =>
    set((state) => ({
      undoStack: [...state.undoStack.slice(-19), snapshot],
      redoStack: [],
    })),
  setStatusMessage: (msg) => set({ statusMessage: msg }),
  setLoading: (loading) => set({ isLoading: loading }),
  setImportProgress: (progress, stage) => set({ importProgress: progress, importStage: stage }),
  setLanguage: (lang) => set({ language: lang }),
}));
