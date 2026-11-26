import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, expect, it, vi } from "vitest";
import { useAppStore } from "../../store/appStore";
import { PaintTool } from "../../types/mesh";
import { ColorPanel } from "./ColorPanel";

const { invoke } = vi.hoisted(() => ({ invoke: vi.fn() }));
vi.mock("@tauri-apps/api/core", () => ({ invoke }));

const stops = [[235, 65, 75, 255], [0, 176, 240, 255], [255, 255, 0, 255]];

beforeEach(() => {
  useAppStore.setState({
    language: "zh",
    activeTabId: 7,
    activeTool: PaintTool.View,
    gradientEnabled: false,
    paintSourceHoverFaces: null,
    meshData: {
      vertices: [], faces: [],
      faceColors: [235, 65, 75, 255, 20, 129, 204, 255],
      faceCount: 2, segmentLabels: [0, 0], segments: [],
      bbox: { min: [0, 0, 0], max: [1, 1, 1] },
    },
  });
  invoke.mockReset();
  invoke.mockImplementation(async (command) => {
    if (command === "paint_source_groups") return [{ stops, faces: 2 }];
    if (command === "paint_source_faces") return [0, 1];
    return [];
  });
});

it("shows a multi-stop ramp under painted colours and reuses it as a gradient", async () => {
  render(<ColorPanel />);
  const swatch = await screen.findByRole("button", { name: /渐变绘制的 2 面/ });
  expect(screen.getByText("绘制颜色")).toBeTruthy();
  fireEvent.mouseEnter(swatch);
  await waitFor(() => expect(useAppStore.getState().paintSourceHoverFaces).toEqual([0, 1]));
  expect(invoke).toHaveBeenCalledWith("paint_source_faces", { stops, tabId: 7 });
  fireEvent.click(swatch);
  expect(useAppStore.getState()).toMatchObject({
    activeTool: PaintTool.Brush,
    gradientEnabled: true,
    gradientStops: stops,
  });
});

it("cancels hover results that arrive after an undo changed paint ownership", async () => {
  let finishHover: ((faces: number[]) => void) | undefined;
  invoke.mockImplementation(async (command) => {
    if (command === "paint_source_groups") return [{ stops, faces: 2 }];
    if (command === "paint_source_faces") return new Promise(resolve => { finishHover = resolve; });
    return [];
  });
  render(<ColorPanel />);
  fireEvent.mouseEnter(await screen.findByRole("button", { name: /渐变绘制的 2 面/ }));
  await waitFor(() => expect(finishHover).toBeDefined());
  act(() => useAppStore.getState().setHistoryFlags(false, true));
  await act(async () => finishHover?.([0, 1]));
  expect(useAppStore.getState().paintSourceHoverFaces).toBeNull();
});
