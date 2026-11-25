import { render } from "@testing-library/react";
import * as THREE from "three";
import { afterEach, expect, it, vi } from "vitest";
import type { MeshData } from "../../types/mesh";
import type { OriginalTexturePreview } from "../../types/texture";
import { TexturePreviewCanvas } from "./TexturePreviewCanvas";

vi.mock("three", async (importOriginal) => ({
  ...(await importOriginal<typeof THREE>()),
  WebGLRenderer: class {
    domElement = document.createElement("canvas");
    setPixelRatio() {}
    setClearColor() {}
    dispose() {}
    forceContextLoss() {}
  },
}));

afterEach(() => {
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
});

it("applies the GLB minification and magnification filters to the original texture", () => {
  vi.stubGlobal("ResizeObserver", class {
    observe() {}
    disconnect() {}
  });
  const texture = new THREE.Texture();
  vi.spyOn(THREE.TextureLoader.prototype, "load").mockReturnValue(texture);
  const converted: MeshData = {
    vertices: [0, 0, 0, 1, 0, 0, 0, 1, 0],
    faces: [0, 1, 2],
    faceColors: [255, 0, 0, 255],
    segmentLabels: [0],
    bbox: { min: [0, 0, 0], max: [1, 1, 0] },
    faceCount: 1,
    segments: [],
  };
  const original: OriginalTexturePreview = {
    vertices: converted.vertices,
    faces: converted.faces,
    cornerUvs: [0, 0, 1, 0, 0, 1],
    imageUrl: "data:image/png;base64,AA==",
    wrapS: "repeat",
    wrapT: "clamp",
    magFilter: 9728,
    minFilter: 9985,
    baseColorFactor: [1, 1, 1, 1],
  };
  render(<TexturePreviewCanvas converted={converted} label="Texture preview" original={original} />);
  expect(texture.magFilter).toBe(THREE.NearestFilter);
  expect(texture.minFilter).toBe(THREE.LinearMipmapNearestFilter);
});
