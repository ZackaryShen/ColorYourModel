import * as THREE from "three";
import type { OriginalTexturePreview } from "../../types/texture";

export function createPreviewGeometry(vertices: ArrayLike<number>, faces: ArrayLike<number>) {
  // Independent corners keep UV seams and distinct colors on adjacent faces.
  const positions = new Float32Array(faces.length * 3);
  for (let corner = 0; corner < faces.length; corner++) {
    const source = faces[corner] * 3;
    const target = corner * 3;
    positions[target] = vertices[source];
    positions[target + 1] = vertices[source + 1];
    positions[target + 2] = vertices[source + 2];
  }

  const geometry = new THREE.BufferGeometry();
  geometry.setAttribute("position", new THREE.BufferAttribute(positions, 3));
  geometry.computeVertexNormals();
  geometry.computeBoundingSphere();
  return geometry;
}

export function createOriginalGeometry(model: OriginalTexturePreview) {
  const geometry = createPreviewGeometry(model.vertices, model.faces);
  geometry.setAttribute("uv", new THREE.Float32BufferAttribute(model.cornerUvs, 2));
  return geometry;
}

const LINEAR_FROM_BYTE = Float32Array.from({ length: 256 }, (_, byte) =>
  THREE.ColorManagement.toWorkingColorSpace(new THREE.Color(byte / 255, 0, 0), THREE.SRGBColorSpace).r,
);

export function writePreviewColors(geometry: THREE.BufferGeometry, faceColors: ArrayLike<number>) {
  const count = geometry.getAttribute("position").count;
  let colors = geometry.getAttribute("color");
  if (!colors) {
    colors = new THREE.BufferAttribute(new Float32Array(count * 3), 3);
    geometry.setAttribute("color", colors);
  }

  for (let corner = 0; corner < count; corner += 3) {
    const source = (corner / 3) * 4;
    const r = LINEAR_FROM_BYTE[faceColors[source]];
    const g = LINEAR_FROM_BYTE[faceColors[source + 1]];
    const b = LINEAR_FROM_BYTE[faceColors[source + 2]];
    colors.setXYZ(corner, r, g, b);
    colors.setXYZ(corner + 1, r, g, b);
    colors.setXYZ(corner + 2, r, g, b);
  }
  colors.needsUpdate = true;
}
