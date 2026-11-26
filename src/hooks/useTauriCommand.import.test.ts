import { act, renderHook } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { invoke } from "@tauri-apps/api/core";
import { useAppStore } from "../store/appStore";
import { useTauriCommand } from "./useTauriCommand";
import type { MeshData } from "../types/mesh";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));

const model: MeshData = {
  vertices: [0, 0, 0, 1, 0, 0, 0, 1, 0], faces: [0, 1, 2],
  faceColors: [200, 200, 200, 255], segmentLabels: [0],
  bbox: { min: [0, 0, 0], max: [1, 1, 0] }, faceCount: 1, segments: [],
  engineeringState: "restored",
};

describe("loading persisted history", () => {
  beforeEach(() => {
    vi.mocked(invoke).mockReset();
    useAppStore.setState({ meshData: null, canUndo: false, canRedo: false });
  });

  it.each([[true, false], [false, true], [true, true]])(
    "restores undo=%s redo=%s availability when reopening a saved project",
    async (canUndo, canRedo) => {
      vi.mocked(invoke).mockImplementation(async (command) => {
        if (command === "load_model") return model;
        if (command === "history_state") return { canUndo, canRedo };
        throw new Error(`Unexpected command: ${command}`);
      });
      const { result } = renderHook(() => useTauriCommand());
      await act(async () => { await result.current.loadModel("saved.3mf"); });
      expect(useAppStore.getState().canUndo).toBe(canUndo);
      expect(useAppStore.getState().canRedo).toBe(canRedo);
    }
  );

  it("still opens the model if reading history availability fails", async () => {
    vi.mocked(invoke).mockImplementation(async (command) => {
      if (command === "load_model") return model;
      throw new Error("history bridge unavailable");
    });
    const { result } = renderHook(() => useTauriCommand());
    await act(async () => { await result.current.loadModel("saved.3mf"); });
    expect(useAppStore.getState().meshData).toBe(model);
    expect(useAppStore.getState().canUndo).toBe(false);
    expect(useAppStore.getState().canRedo).toBe(false);
  });
});

describe("persisted region names", () => {
  it("marks a successful rename unsaved while keeping restored undo and redo available", async () => {
    const renamed = [{ id: 1, name: "New name", color: null, faceCount: 1 }];
    vi.mocked(invoke).mockResolvedValue(renamed);
    useAppStore.setState({ meshData: model, paintDirty: false, canUndo: true, canRedo: true });
    const { result } = renderHook(() => useTauriCommand());
    await act(async () => { expect(await result.current.renameSegment(1, "New name")).toBe(true); });
    expect(useAppStore.getState().paintDirty).toBe(true);
    expect(useAppStore.getState().canUndo).toBe(true);
    expect(useAppStore.getState().canRedo).toBe(true);
    expect(useAppStore.getState().segments[0].name).toBe("New name");
  });

  it("a rejected rename preserves saved state and history availability", async () => {
    vi.mocked(invoke).mockRejectedValue(new Error("Segment 1 does not exist"));
    useAppStore.setState({ meshData: model, paintDirty: false, canUndo: true, canRedo: false });
    const { result } = renderHook(() => useTauriCommand());
    await act(async () => { expect(await result.current.renameSegment(1, "New name")).toBe(false); });
    expect(useAppStore.getState().paintDirty).toBe(false);
    expect(useAppStore.getState().canUndo).toBe(true);
    expect(useAppStore.getState().canRedo).toBe(false);
  });
});

it('exporting only the current plate to OBJ does not mark the complete project saved', async () => {
  vi.mocked(invoke).mockResolvedValue('Exported to /tmp/plate.obj');
  useAppStore.setState({ meshData:model, paintDirty:true, tabs:[{id:1,name:'A',faceCount:1,dirty:true},{id:2,name:'B',faceCount:1,dirty:true}] });
  const { result } = renderHook(() => useTauriCommand());
  await act(() => result.current.exportObj('/tmp/plate.obj'));
  expect(useAppStore.getState().paintDirty).toBe(true);
  expect(useAppStore.getState().tabs.every(tab => tab.dirty)).toBe(true);
});
it('remembers the actual complete-project export path for subsequent Save', async () => {
  useAppStore.setState({ meshData:model, projectPath:'/tmp/A.3mf', paintDirty:true });
  vi.mocked(invoke).mockResolvedValue('Exported to /tmp/B.3mf');
  const { result } = renderHook(() => useTauriCommand());
  await act(() => result.current.export3mf('/tmp/B.3mf'));
  expect(useAppStore.getState().projectPath).toBe('/tmp/B.3mf');
});

it("shows the number of excess native recipes when a Chinese print export fails", async () => {
  useAppStore.setState({ meshData: model, activeTabId: 1, language: "zh", projectPath: "/tmp/original.3mf" });
  vi.mocked(invoke).mockRejectedValue("Snapmaker Orca supports 64 total filament slots. Reduce the number of gradient recipes by at least 2, or split the project.");
  const { result } = renderHook(() => useTauriCommand());
  await act(async () => {
    await expect(result.current.export3mf("/tmp/out.3mf")).rejects.toContain("至少减少 2 组");
  });
  expect(useAppStore.getState().projectPath).toBe("/tmp/original.3mf");
});

it("reports the native recipe limit on save while retaining the canonical project path", async () => {
  useAppStore.setState({ meshData: model, language: "en", activeTabId: 1, importBusy: false, saveBusy: false });
  vi.mocked(invoke).mockResolvedValue({ message: "Exported to /tmp/saved.3mf", omittedGradientRecipes: 3 });
  const { result } = renderHook(() => useTauriCommand());
  await act(async () => {
    expect(await result.current.export3mf("/tmp/link.3mf", undefined, true)).toBe("Exported to /tmp/saved.3mf");
  });
  expect(useAppStore.getState().projectPath).toBe("/tmp/saved.3mf");
  expect(useAppStore.getState().toast).toContain("3 gradient");
});

it("keeps the workspace save path and unrelated dirty plates after exporting a mapped texture plate", async () => {
  vi.mocked(invoke).mockClear();
  const active = { id: 1, name: "Texture", faceCount: 1, dirty: false };
  const other = { id: 2, name: "STL", faceCount: 1, dirty: true };
  useAppStore.setState({
    meshData: { ...model, texturePainting: true },
    activeTabId: 1, tabEpoch: 1, projectPath: "/tmp/Project.3mf",
    importBusy: false, saveBusy: false, paintDirty: false, tabs: [active, other],
  });
  vi.mocked(invoke).mockImplementation(async (command) => {
    if (command === "export_3mf_command") return "Exported to /tmp/Print.3mf";
    if (command === "project_tabs") return [{ ...active, dirty: true }, other];
    throw new Error(`Unexpected command: ${command}`);
  });
  const selection = {
    machineId: "snapmaker_u1", nozzleDiameter: "0.4", processName: "Standard",
    filamentNames: ["PLA"], targetSlicer: "snapmaker_orca",
  };
  const texture = { revision: "confirmed", mapping: { selection, nominalColors: [], assignments: [] } };
  const { result } = renderHook(() => useTauriCommand());
  await act(() => result.current.export3mf("/tmp/Print.3mf", selection, false, texture));
  expect(invoke).toHaveBeenCalledWith("export_3mf_command", {
    path: "/tmp/Print.3mf", selection, preserveSource: false, texture, tabId: 1,
  });
  expect(useAppStore.getState()).toMatchObject({
    projectPath: "/tmp/Project.3mf", paintDirty: true, tabs: [{ ...active, dirty: true }, other],
  });
});

it('a successful reset clears undo and redo availability and refreshes the timeline', async () => {
  useAppStore.setState({activeTabId:1,tabEpoch:1,importBusy:false,saveBusy:false,meshData:model,canUndo:true,canRedo:true});
  const previousVersion = useAppStore.getState().historyVersion;
  vi.mocked(invoke).mockResolvedValue({segmentLabels:[0],segments:[],faceColors:[200,200,200,255]});
  const { result } = renderHook(() => useTauriCommand());
  await act(() => result.current.resetSegmentation());
  expect(useAppStore.getState().canUndo).toBe(false);
  expect(useAppStore.getState().canRedo).toBe(false);
  expect(useAppStore.getState().historyVersion).toBeGreaterThan(previousVersion);
});

it("selects a newly closed lasso region and enables undo right away", async () => {
  useAppStore.setState({activeTabId: 1, tabEpoch: 1, importBusy: false, saveBusy: false,
    isLoading: false, meshData: model, canUndo: false, canRedo: true});
  vi.mocked(invoke).mockResolvedValueOnce({
    label: 100001, segmentLabels: [100001], faceColors: [1, 2, 3, 255],
    segments: [{id: 100001, name: "Manual", faceCount: 1, color: null}],
  });
  const { result } = renderHook(() => useTauriCommand());
  await act(() => result.current.finalizeManualRegion([[0,0,0], [1,0,0], [0,1,0]], [0,0,0]));
  expect(useAppStore.getState()).toMatchObject({selectedSegment: 100001, canUndo: true, canRedo: false});
});
