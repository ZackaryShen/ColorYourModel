import { act, renderHook, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { invoke } from "@tauri-apps/api/core";
import { useAppStore } from "../store/appStore";
import type { MeshData } from "../types/mesh";
import type { TexturePreview } from "../types/texture";
import { useTextureImport } from "./useTextureImport";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
const invokeCommand = vi.mocked(invoke);

const oldMesh: MeshData = {
  tabId: 7,
  vertices: [0, 0, 0, 2, 0, 0, 0, 2, 0],
  faces: [0, 1, 2],
  faceColors: [0, 0, 255, 255],
  segmentLabels: [1],
  bbox: { min: [0, 0, 0], max: [2, 2, 0] },
  faceCount: 1,
  segments: [{ id: 1, name: "old paint", color: [0, 0, 255, 255], faceCount: 1 }],
};

const newMesh: MeshData = {
  ...oldMesh,
  tabId: 8,
  tabs: [
    { id: 7, name: "Existing plate", faceCount: 1, dirty: true },
    { id: 8, name: "Tide", faceCount: 1, dirty: true },
  ],
  faceColors: [255, 0, 0, 255],
  segmentLabels: [0],
  segments: [],
};

const preview: TexturePreview = {
  original: {
    vertices: newMesh.vertices, faces: newMesh.faces, cornerUvs: [0, 0, 1, 0, 0, 1],
    imageUrl: "data:image/png;base64,AA==", wrapS: "repeat", wrapT: "repeat",
    baseColorFactor: [1, 1, 1, 1],
  },
  converted: newMesh,
  palette: [[255, 0, 0]],
};

function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (reason: unknown) => void;
  const promise = new Promise<T>((done, fail) => { resolve = done; reject = fail; });
  return { promise, resolve, reject };
}

function binaryMesh(data: MeshData): ArrayBuffer {
  const { vertices, faces, segmentLabels, faceColors, ...metadata } = data;
  const json = new TextEncoder().encode(JSON.stringify(metadata));
  const bytes = new ArrayBuffer(20 + 4 * (vertices.length + faces.length + segmentLabels.length) + faceColors.length + json.length);
  const header = new DataView(bytes);
  [vertices.length, faces.length, segmentLabels.length, faceColors.length, json.length]
    .forEach((length, i) => header.setUint32(i * 4, length, true));
  let offset = 20;
  new Float32Array(bytes, offset, vertices.length).set(vertices);
  offset += vertices.length * 4;
  new Uint32Array(bytes, offset, faces.length).set(faces);
  offset += faces.length * 4;
  new Uint32Array(bytes, offset, segmentLabels.length).set(segmentLabels);
  offset += segmentLabels.length * 4;
  new Uint8Array(bytes, offset, faceColors.length).set(faceColors);
  new Uint8Array(bytes, offset + faceColors.length).set(json);
  return bytes;
}

beforeEach(() => {
  invokeCommand.mockReset();
  useAppStore.setState({
    meshData: oldMesh,
    tabs: [{ id: 7, name: "Existing plate", faceCount: 1, dirty: true }],
    activeTabId: 7,
    projectPath: "/models/saved.3mf",
    importBusy: false,
    saveBusy: false,
    pendingImportPath: null,
    pendingCloseTabId: null,
    isLoaded: true,
    paintDirty: true,
    canUndo: true,
    canRedo: true,
    isLoading: false,
  });
});

describe("GLB preview and apply", () => {
  it("keeps the active plate until apply and appends an unsaved plate without changing the project path", async () => {
    const apply = deferred<ArrayBuffer>();
    invokeCommand.mockImplementation(async (command) => {
      if (command === "prepare_texture") return preview;
      if (command === "apply_texture") return apply.promise;
      return undefined;
    });
    const { result } = renderHook(useTextureImport);
    act(() => { result.current.open("/models/tide.glb"); });
    await waitFor(() => expect(result.current.session?.phase).toBe("ready"));
    expect(useAppStore.getState().importBusy).toBe(true);
    expect(invokeCommand.mock.calls.map(([command]) => command)).toEqual(["prepare_texture"]);
    expect(useAppStore.getState().meshData).toBe(oldMesh);
    expect(useAppStore.getState().canUndo).toBe(true);

    act(() => { void result.current.apply(); });
    expect(result.current.session?.phase).toBe("applying");
    act(() => { result.current.cancel(); });
    expect(result.current.session?.phase).toBe("applying");
    expect(useAppStore.getState().meshData).toBe(oldMesh);

    await act(async () => { apply.resolve(binaryMesh(newMesh)); });
    expect(result.current.session).toBeNull();
    expect(useAppStore.getState().meshData).toMatchObject({
      tabId: 8, faces: Uint32Array.from(newMesh.faces), faceColors: Uint8Array.from(newMesh.faceColors),
    });
    expect(useAppStore.getState().tabs).toEqual(newMesh.tabs);
    expect(useAppStore.getState().activeTabId).toBe(8);
    expect(useAppStore.getState().projectPath).toBe("/models/saved.3mf");
    expect(useAppStore.getState().importBusy).toBe(false);
    expect(useAppStore.getState().paintDirty).toBe(true);
    expect(useAppStore.getState().canUndo).toBe(false);
    expect(useAppStore.getState().canRedo).toBe(false);
  });

  it("ignores a late response once a different palette limit is preparing", async () => {
    const first = deferred<TexturePreview>();
    const second = deferred<TexturePreview>();
    invokeCommand.mockImplementation(async (command, args) => {
      if (command !== "prepare_texture") return undefined;
      return (args as { colorLimit: number }).colorLimit === 8 ? first.promise : second.promise;
    });
    const { result } = renderHook(useTextureImport);
    act(() => { result.current.open("/models/tide.glb"); });
    act(() => { result.current.setColorLimit(16); });
    await act(async () => { first.resolve(preview); });
    expect(result.current.session?.phase).toBe("preparing");
    await act(async () => { await result.current.apply(); });
    expect(invokeCommand.mock.calls.some(([command]) => command === "apply_texture")).toBe(false);
    const latestPreview = { ...preview, palette: [[255, 0, 0], [0, 255, 0]] };
    await act(async () => { second.resolve(latestPreview); });
    await waitFor(() => expect(result.current.session?.preview).toBe(latestPreview));
    const prepareCalls = invokeCommand.mock.calls.filter(([command]) => command === "prepare_texture");
    expect(prepareCalls).toHaveLength(2);
    expect(Number(prepareCalls[1][1]?.jobId)).toBeGreaterThan(Number(prepareCalls[0][1]?.jobId));
    expect(useAppStore.getState().meshData).toBe(oldMesh);
  });

  it("keeps unsaved work and ignores a late response after cancel", async () => {
    const pending = deferred<TexturePreview>();
    invokeCommand.mockImplementation(async (command) => command === "prepare_texture" ? pending.promise : undefined);
    const { result } = renderHook(useTextureImport);
    act(() => { result.current.open("/models/tide.glb"); });
    act(() => { result.current.cancel(); });
    await act(async () => { pending.resolve(preview); });
    expect(result.current.session).toBeNull();
    expect(useAppStore.getState().meshData).toBe(oldMesh);
    expect(useAppStore.getState().paintDirty).toBe(true);
    expect(useAppStore.getState().canUndo).toBe(true);
    expect(useAppStore.getState().importBusy).toBe(false);
    expect(invokeCommand.mock.calls[1]).toEqual(["cancel_texture", { jobId: invokeCommand.mock.calls[0][1]?.jobId }]);
  });

  it("reports missing UVs while keeping the current mesh and undo stack", async () => {
    invokeCommand.mockImplementation(async (command) => {
      if (command === "prepare_texture") throw "Texture import requires TEXCOORD_0 UVs";
      return undefined;
    });
    const { result } = renderHook(useTextureImport);
    act(() => { result.current.open("/models/invalid.glb"); });
    await waitFor(() => expect(result.current.session?.phase).toBe("prepare-error"));
    expect(result.current.session?.error).toMatch(/TEXCOORD_0/);
    expect(useAppStore.getState().meshData).toBe(oldMesh);
    expect(useAppStore.getState().canUndo).toBe(true);
  });
});
