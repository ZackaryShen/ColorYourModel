import { describe, it, expect } from "vitest";
import * as THREE from "three";
import {
  CLOSURE_INTENT_FACTOR,
  isClosureIntent,
  localToScreenPx,
  projectedRadiusPx,
} from "./lassoClosure";

// Fake canvas: 800×600 CSS px, top-left at viewport origin.
const domElement = {
  getBoundingClientRect: () => ({ left: 0, top: 0, width: 800, height: 600, right: 800, bottom: 600, x: 0, y: 0, toJSON: () => {} }),
} as unknown as HTMLElement;

/** Orthographic camera at +Z looking at the origin, world +Y up. */
function orthoCamera(zoom = 1): THREE.OrthographicCamera {
  const cam = new THREE.OrthographicCamera(-1, 1, 1, -1, 0.1, 100);
  cam.position.set(0, 0, 10);
  cam.lookAt(0, 0, 0);
  cam.updateMatrixWorld(true);
  cam.zoom = zoom;
  cam.updateProjectionMatrix();
  return cam;
}

const mesh = new THREE.Object3D(); // identity local→world

describe("lassoClosure.localToScreenPx", () => {
  it("maps the world origin to the rect center (NDC→pixel, y flipped)", () => {
    const p = localToScreenPx(new THREE.Vector3(0, 0, 0), orthoCamera(), domElement, mesh)!;
    expect(p.x).toBeCloseTo(400, 5);
    expect(p.y).toBeCloseTo(300, 5);
  });

  it("world +Y maps upward on screen (smaller y)", () => {
    const up = localToScreenPx(new THREE.Vector3(0, 0.5, 0), orthoCamera(), domElement, mesh)!;
    expect(up.y).toBeLessThan(300);
    expect(up.x).toBeCloseTo(400, 5);
  });
});

describe("lassoClosure.projectedRadiusPx", () => {
  it("grows proportionally with ortho camera.zoom (zoom-invariant intent by construction)", () => {
    const r1 = projectedRadiusPx(new THREE.Vector3(0, 0, 0), 0.05, orthoCamera(1), domElement, mesh)!;
    const r2 = projectedRadiusPx(new THREE.Vector3(0, 0, 0), 0.05, orthoCamera(4), domElement, mesh)!;
    expect(r1).toBeGreaterThan(0);
    expect(r2 / r1).toBeCloseTo(4, 3);
  });
});

describe("lassoClosure.isClosureIntent", () => {
  const start = { x: 400, y: 300 };

  it("accepts a click inside the projected marker footprint", () => {
    expect(isClosureIntent({ x: 405, y: 302 }, start, 6)).toBe(true);
  });

  it("rejects a click outside marker radius × factor", () => {
    const outside = 6 * CLOSURE_INTENT_FACTOR + 1;
    expect(isClosureIntent({ x: 400 + outside, y: 300 }, start, 6)).toBe(false);
  });

  it("honours the factor at the boundary", () => {
    const edge = 6 * CLOSURE_INTENT_FACTOR;
    expect(isClosureIntent({ x: 400 + edge, y: 300 }, start, 6)).toBe(true);
    expect(isClosureIntent({ x: 400 + edge + 0.01, y: 300 }, start, 6)).toBe(false);
  });
});
