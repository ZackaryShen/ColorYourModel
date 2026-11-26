import { act, fireEvent, render, screen, within } from "@testing-library/react";
import { beforeEach, expect, it } from "vitest";
import { useAppStore } from "../../store/appStore";
import { PaintTool, type MeshData } from "../../types/mesh";
import { BrushSettings } from "./BrushSettings";

const mesh: MeshData = {
  vertices: [], faces: [], faceColors: [], segmentLabels: [], segments: [],
  faceCount: 0, bbox: { min: [0, 0, 0], max: [1, 1, 1] },
};

beforeEach(() => {
  useAppStore.setState({
    activeTool: PaintTool.Brush, language: "en", meshData: mesh, gradientEnabled: false,
    gradientStops: [[255, 0, 0, 255], [0, 0, 255, 255]],
    gradientContext: null, hasGradientPaint: false, showSavedGradientGuide: false,
  });
});

it("edits and reorders one stop list shared by Brush and Fill", () => {
  render(<BrushSettings />);
  fireEvent.click(screen.getByRole("checkbox", { name: "Gradient" }));
  expect(screen.getByRole("button", { name: "Remove colour stop 1" })).toHaveProperty("disabled", true);
  fireEvent.click(screen.getByRole("button", { name: "Add colour stop" }));
  fireEvent.change(screen.getByLabelText("Colour stop 3"), { target: { value: "#ff9900" } });
  fireEvent.click(screen.getByRole("button", { name: "Move colour stop 3 up" }));
  act(() => useAppStore.getState().setActiveTool(PaintTool.Fill));

  const stops = screen.getByRole("group", { name: "Colour stops" });
  expect(within(stops).getByLabelText("Colour stop 1")).toHaveProperty("value", "#ff0000");
  expect(within(stops).getByLabelText("Colour stop 2")).toHaveProperty("value", "#ff9900");
  expect(within(stops).getByLabelText("Colour stop 3")).toHaveProperty("value", "#0000ff");
  fireEvent.click(within(stops).getByRole("button", { name: "Remove colour stop 2" }));
  expect(within(stops).queryByLabelText("Colour stop 3")).toBeNull();
});

it("disables the gradient mode on a texture plate", () => {
  useAppStore.setState({ gradientEnabled: true, meshData: { ...mesh, texturePainting: true } });
  render(<BrushSettings />);
  expect(screen.getByRole("checkbox", { name: "Gradient" })).toHaveProperty("disabled", true);
  expect(screen.queryByRole("group", { name: "Colour stops" })).toBeNull();
});

it("restores the saved fill and ordered palette when the project opens", () => {
  render(<BrushSettings />);
  const project = {
    ...mesh,
    gradientContext: {
      tool: PaintTool.Fill,
      stops: [[235, 65, 75, 255], [0, 176, 240, 255]],
      radius: 2.5,
      points: [{ faceId: 0, point: [0, 0, 0] }, { faceId: 1, point: [2, 0, 0] }],
      wholeRegion: true,
    },
  } as MeshData;
  act(() => useAppStore.getState().setMeshData(project));

  expect(useAppStore.getState().activeTool).toBe(PaintTool.Fill);
  expect(screen.getByRole("checkbox", { name: "Gradient" })).toHaveProperty("checked", true);
  expect(screen.getByLabelText("Colour stop 1")).toHaveProperty("value", "#eb414b");
  expect(screen.getByLabelText("Colour stop 2")).toHaveProperty("value", "#00b0f0");
  const showDirection = screen.getByRole("button", { name: "Show saved direction" });
  fireEvent.click(showDirection);
  expect(showDirection.getAttribute("aria-pressed")).toBe("true");

  act(() => useAppStore.getState().setMeshData(mesh));
  expect(screen.getByRole("checkbox", { name: "Gradient" })).toHaveProperty("checked", false);
  expect(screen.queryByRole("button", { name: "Show saved direction" })).toBeNull();
});

it("identifies restored legacy paint without opening a guessed tool or direction", () => {
  useAppStore.setState({ activeTool: PaintTool.View });
  render(<BrushSettings />);
  act(() => useAppStore.getState().setMeshData({ ...mesh, hasGradientPaint: true }));

  expect(screen.getByRole("status").textContent).toContain("Gradient colours restored");
  expect(useAppStore.getState().activeTool).toBe(PaintTool.View);
  expect(screen.queryByRole("button", { name: "Show saved direction" })).toBeNull();
});
