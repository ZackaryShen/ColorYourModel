import { act, renderHook } from "@testing-library/react";
import { beforeEach, expect, it, vi } from "vitest";
import { invoke } from "@tauri-apps/api/core";
import { open } from "@tauri-apps/plugin-dialog";
import { useImportStl } from "./useImportStl";
import { useDropImport } from "./useDropImport";
import { useAppStore } from "../store/appStore";
import type { MeshData } from "../types/mesh";

const dropBridge = vi.hoisted(() => ({ listener: null as null | ((e: { payload: { type: string; paths: string[] } }) => void) }));
vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
vi.mock("@tauri-apps/plugin-dialog", () => ({ open: vi.fn() }));
vi.mock("@tauri-apps/api/webview", () => ({ getCurrentWebview: () => ({
  onDragDropEvent: async (listener: typeof dropBridge.listener) => {
    dropBridge.listener = listener;
    return () => { dropBridge.listener = null; };
  },
}) }));
const oldModel: MeshData = {
  vertices: [0, 0, 0, 1, 0, 0, 0, 1, 0], faces: [0, 1, 2],
  faceColors: [200, 200, 200, 255], segmentLabels: [0],
  bbox: { min: [0, 0, 0], max: [1, 1, 0] }, faceCount: 1, segments: [],
};
const newModel = { ...oldModel, vertices: [0, 0, 1, 1, 0, 1, 0, 1, 1] };

beforeEach(() => {
  vi.mocked(invoke).mockReset();
  vi.mocked(open).mockReset();
  vi.mocked(invoke).mockImplementation(async (command) => {
    if (command === "load_model") return newModel;
    if (command === "history_state") return { canUndo: false, canRedo: false };
    throw new Error(`Unexpected: ${command}`);
  });
  useAppStore.setState({ meshData: oldModel, projectPath: "/old.3mf", paintDirty: true, isLoading: false, importBusy: false, saveBusy: false, pendingImportPath: null });
});

it("asks before a picker import discards unsaved work and cancellation preserves it", async () => {
  vi.mocked(open).mockResolvedValue("/new.3mf");
  const { result } = renderHook(() => useImportStl());
  await act(async () => { await result.current.importStl(); });
  expect(useAppStore.getState().pendingImportPath).toBe("/new.3mf");
  expect(useAppStore.getState().meshData).toBe(oldModel);
  act(() => result.current.cancelReplace());
  expect(useAppStore.getState().pendingImportPath).toBeNull();
  expect(useAppStore.getState().projectPath).toBe("/old.3mf");
  expect(useAppStore.getState().paintDirty).toBe(true);
});

it("routes picker and dropped GLBs to preview without offering to replace the project", async () => {
  const preview = vi.fn();
  vi.mocked(open).mockResolvedValue("/texture.glb");
  const { result } = renderHook(() => useImportStl(preview));
  await act(async () => { await result.current.importStl(); });
  await act(async () => { await result.current.importFromPath("/dropped.GLB", true); });
  expect(preview.mock.calls).toEqual([["/texture.glb"], ["/dropped.GLB"]]);
  expect(invoke).not.toHaveBeenCalled();
  expect(useAppStore.getState().pendingImportPath).toBeNull();
  expect(useAppStore.getState().projectPath).toBe("/old.3mf");
});

it("confirmation imports once without asking again across hook instances", async () => {
  const first = renderHook(() => useImportStl());
  const second = renderHook(() => useImportStl());
  await act(async () => { await first.result.current.importFromPath("/new.3mf"); });
  await act(async () => { await second.result.current.confirmReplace(); });
  expect(useAppStore.getState().pendingImportPath).toBeNull();
  expect(useAppStore.getState().meshData).toBe(newModel);
  expect(useAppStore.getState().projectPath).toBeNull();
  expect(useAppStore.getState().importSourcePath).toBe("/new.3mf");
  expect(useAppStore.getState().paintDirty).toBe(false);
});

it("a failed confirmed import preserves the old project and dirty flag", async () => {
  vi.mocked(invoke).mockRejectedValue(new Error("broken archive"));
  const { result } = renderHook(() => useImportStl());
  await act(async () => { await result.current.importFromPath("/broken.3mf"); });
  await act(async () => { await result.current.confirmReplace(); });
  expect(useAppStore.getState().meshData).toBe(oldModel);
  expect(useAppStore.getState().projectPath).toBe("/old.3mf");
  expect(useAppStore.getState().paintDirty).toBe(true);
  expect(useAppStore.getState().importBusy).toBe(false);
  expect(useAppStore.getState().isLoading).toBe(false);
});

it("rejects a second import while the first load is pending", async () => {
  const finishes: ((value: MeshData) => void)[] = [];
  vi.mocked(invoke).mockImplementation((command) => command === "load_model"
    ? new Promise((resolve) => { finishes.push(resolve as (value: MeshData) => void); })
    : Promise.resolve({ canUndo: false, canRedo: false }));
  useAppStore.setState({ paintDirty: false });
  const { result } = renderHook(() => useImportStl());
  let loading!: Promise<void>;
  act(() => { loading = result.current.importFromPath("/first.3mf"); });
  let second!: Promise<void>;
  act(() => { second = result.current.importFromPath("/second.3mf"); });
  await act(async () => { for (const finish of finishes) finish(newModel); await Promise.all([loading, second]); });
  expect(useAppStore.getState().importSourcePath).toBe("/first.3mf");
});

it("does not replace a pending confirmation with a later request", async () => {
  const { result } = renderHook(() => useImportStl());
  await act(async () => { await result.current.importFromPath("/first.3mf"); });
  await act(async () => { await result.current.importFromPath("/second.3mf"); });
  expect(useAppStore.getState().pendingImportPath).toBe("/first.3mf");
});

it("blocks both picker and path imports while a project save owns the workflow", async () => {
  useAppStore.setState({ saveBusy: true, paintDirty: false });
  vi.mocked(open).mockResolvedValue("/new.3mf");
  const { result } = renderHook(() => useImportStl());
  await act(async () => { await result.current.importFromPath("/new.3mf"); });
  await act(async () => { await result.current.importStl(); });
  expect(useAppStore.getState().meshData).toBe(oldModel);
  expect(useAppStore.getState().projectPath).toBe("/old.3mf");
  expect(useAppStore.getState().pendingImportPath).toBeNull();
  expect(useAppStore.getState().saveBusy).toBe(true);
});

it("keeps drag replacement to one shared confirmation even for a clean model", async () => {
  useAppStore.setState({ paintDirty: false });
  const { result } = renderHook(() => {
    const imports = useImportStl();
    useDropImport(imports.importFromPath);
    return imports;
  });
  await act(async () => {});
  act(() => dropBridge.listener!({ payload: { type: "drop", paths: ["/new.3mf"] } }));
  expect(useAppStore.getState().pendingImportPath).toBe("/new.3mf");
  await act(async () => { await result.current.confirmReplace(); });
  expect(useAppStore.getState().pendingImportPath).toBeNull();
  expect(useAppStore.getState().meshData).toBe(newModel);
});
it('does not stack a file replacement over a pending plate-close confirmation', async () => {
  useAppStore.setState({ pendingCloseTabId: 3 });
  const { result } = renderHook(() => useImportStl());
  await act(() => result.current.importFromPath('/new.3mf', true));
  expect(vi.mocked(invoke)).not.toHaveBeenCalled();
  expect(useAppStore.getState().pendingImportPath).toBeNull();
  useAppStore.setState({ pendingCloseTabId: null });
});

it("binds an already restored CYM project to its saved path", async () => {
  vi.mocked(invoke).mockImplementation(async (command) => command === "load_model"
    ? { ...newModel, engineeringState: "restored" }
    : { canUndo: false, canRedo: false });
  const { result } = renderHook(() => useImportStl());
  await act(() => result.current.importFromPath("/saved.3mf"));
  await act(() => result.current.confirmReplace());
  expect(useAppStore.getState().projectPath).toBe("/saved.3mf");
  expect(useAppStore.getState().importSourcePath).toBeNull();
});
