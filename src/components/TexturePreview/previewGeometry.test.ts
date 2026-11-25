import { describe, expect, it } from "vitest";
import { createOriginalGeometry, createPreviewGeometry, writePreviewColors } from "./previewGeometry";

const vertices = [0, 0, 0, 2, 0, 0, 2, 1, 0, 0, 1, 0];
const faces = [0, 1, 2, 0, 2, 3];

describe("texture preview geometry", () => {
  it("retains per-corner glTF UVs across a shared position seam", () => {
    const geometry = createOriginalGeometry({
      vertices, faces,
      cornerUvs: [0, 0, 1, 0, 1, 1, 0.5, 0.25, 0, 1, 1, 1],
      imageUrl: "data:image/png;base64,AA==",
      wrapS: "repeat", wrapT: "mirror",
      baseColorFactor: [1, 1, 1, 1],
    });
    expect(Array.from(geometry.getAttribute("position").array)).toEqual([
      0, 0, 0, 2, 0, 0, 2, 1, 0, 0, 0, 0, 2, 1, 0, 0, 1, 0,
    ]);
    expect(Array.from(geometry.getAttribute("uv").array)).toEqual([
      0, 0, 1, 0, 1, 1, 0.5, 0.25, 0, 1, 1, 1,
    ]);
    geometry.dispose();
  });

  it("keeps each face color uniform and converts sRGB bytes to linear values", () => {
    const geometry = createPreviewGeometry(vertices, faces);
    writePreviewColors(geometry, [128, 0, 0, 255, 0, 255, 0, 255]);
    const colors = geometry.getAttribute("color");
    for (let vertex = 0; vertex < 3; vertex++) {
      expect(colors.getX(vertex)).toBeCloseTo(0.21586, 5);
      expect(colors.getY(vertex)).toBe(0);
      expect(colors.getX(vertex + 3)).toBe(0);
      expect(colors.getY(vertex + 3)).toBe(1);
    }
    geometry.dispose();
  });
});
