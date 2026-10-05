import * as THREE from "three";

/**
 * Lasso closure-intent helpers (0.2.0-P0).
 *
 * Problem: under a rotated camera the lasso's raycast can hit a different face
 * than it did when the loop started, and the three-tier vertex snap then
 * returns a vertex far from the loop start — the user clicks the visible start
 * dot, but the appended point is 3D-far from it, so the closeThreshold gate
 * never fires and the loop will not close.
 *
 * Fix shape (adversarial-review round): the 3D closeThreshold stays the ONLY
 * safety gate (iteration 42–44 guards untouched); screen space is used purely
 * as an INTENT signal. When a click/hover falls within the start dot's
 * *projected marker radius*, the click is treated as "return to start" and the
 * EXACT start point is appended instead of the mis-snapped vertex (the backend
 * `region_from_loop` skips the resulting a==b self-edge). The projected radius
 * scales with zoom by construction, so no fabricated pixel constant is needed.
 */

export interface ScreenPoint {
  x: number;
  y: number;
}

/** Pointer slop on top of the marker radius. 1.5× covers the dot's own edge
 *  plus typical click imprecision; anything looser starts eating legitimate
 *  corner clicks that merely project near the start (orthographic
 *  depth-coincidence). */
export const CLOSURE_INTENT_FACTOR = 1.5;

function worldToScreen(
  world: THREE.Vector3,
  camera: THREE.Camera,
  rect: { left: number; top: number; width: number; height: number }
): ScreenPoint | null {
  const ndc = world.clone().project(camera);
  if (!Number.isFinite(ndc.x) || !Number.isFinite(ndc.y)) return null;
  return {
    x: rect.left + ((ndc.x + 1) / 2) * rect.width,
    y: rect.top + ((1 - ndc.y) / 2) * rect.height,
  };
}

/** Project a model-local point to CSS pixels in the canvas' client space.
 *  Returns null when the projection degenerates (outside the camera depth
 *  range / non-finite NDC). `mesh.localToWorld` requires an updated
 *  matrixWorld — true at pointer-event time (transform groups update before
 *  render, and events fire between frames). */
export function localToScreenPx(
  local: THREE.Vector3,
  camera: THREE.Camera,
  domElement: HTMLElement,
  mesh: THREE.Object3D
): ScreenPoint | null {
  return worldToScreen(mesh.localToWorld(local.clone()), camera, domElement.getBoundingClientRect());
}

/** Screen-space radius (CSS px) of a model-local sphere at `centerLocal`.
 *  Measured by projecting the center and a point offset along camera-right —
 *  exact for the orthographic camera this app uses (scales with `camera.zoom`),
 *  and a good approximation for perspective. Null when the center cannot be
 *  projected. */
export function projectedRadiusPx(
  centerLocal: THREE.Vector3,
  radiusLocal: number,
  camera: THREE.Camera,
  domElement: HTMLElement,
  mesh: THREE.Object3D
): number | null {
  const rect = domElement.getBoundingClientRect();
  const worldCenter = mesh.localToWorld(centerLocal.clone());
  const camRight = new THREE.Vector3().setFromMatrixColumn(camera.matrixWorld, 0);
  const worldEdge = worldCenter.clone().add(camRight.multiplyScalar(radiusLocal));
  const a = worldToScreen(worldCenter, camera, rect);
  const b = worldToScreen(worldEdge, camera, rect);
  if (!a || !b) return null;
  return Math.hypot(a.x - b.x, a.y - b.y);
}

/** True when the pointer position sits within the start marker's projected
 *  footprint (radius × {@link CLOSURE_INTENT_FACTOR}), i.e. the user is aiming
 *  at the start dot rather than placing a far point. */
export function isClosureIntent(
  pointer: ScreenPoint,
  startPx: ScreenPoint,
  dotRadiusPx: number,
  factor: number = CLOSURE_INTENT_FACTOR
): boolean {
  return Math.hypot(pointer.x - startPx.x, pointer.y - startPx.y) <= dotRadiusPx * factor;
}
