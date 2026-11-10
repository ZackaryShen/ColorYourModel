/**
 * Seed-erase hit testing — in ONE coordinate space, on purpose.
 *
 * Seeds are stored model-local (STL Z-up; `seedPoints` = face centroids from
 * `meshData.vertices` — see `src/types/mesh.ts`). The camera and the pointer
 * only speak world space and CSS pixels, and the model lives inside a group
 * rotated −π/2 about X, so local ≠ world: `world = (x, z, −y)`.
 *
 * Every failed round of this bug compared across that boundary:
 *   • R6/R7  `Vector3.project(seedLocal)`  → treated local as world
 *   • R8-fst `ray.distanceToPoint(seedLocal)` → treated local as world
 * and the error is `√2·ρ_yz` (the seed's distance from the local X axis) —
 * up to a whole model — so no threshold could ever compensate.
 *
 * The only fix is to convert explicitly BEFORE projecting: seed (local) →
 * `mesh.localToWorld` → world → `camera.project` → NDC → CSS px. These helpers
 * keep the two pieces that are easy to get wrong pure and testable:
 *
 *   1. how big the click target is (the drawn glyph, expressed in px), and
 *   2. which seed is nearest to the pointer (deterministic, thresholded).
 *
 * The projection itself is the caller's job because it needs `three`.
 */

export interface SeedLike {
  x: number;
  y: number;
  z: number;
}

export interface ScreenPoint {
  x: number;
  y: number;
}

/** Minimal DOM-rect surface — keeps this module testable in plain jsdom. */
export interface ScreenRect {
  left: number;
  top: number;
  width: number;
  height: number;
}

/**
 * Half-extent of the drawn "sapling" glyph in marker-size units:
 * sphere (1) + stem (2) + leaf radius (0.385) ≈ 3.4. The marker's world size is
 * `seedMarkerSize`, so the glyph covers roughly this many marker sizes around
 * (and above) the anchor. Using the glyph, not the surface hit point, is what
 * makes the target match what the user aims at.
 */
export const GLYPH_RADIUS_UNITS = 3.5;

/**
 * Floor for the click radius. At fit zoom on a large model the automatic
 * marker size is only ~1–3 px, and the old `2.5 × markerSize` world threshold
 * collapsed to the same 2–3 px — far below what a human can aim at. A 14 px
 * radius (28 px diameter) is a comfortable target at any zoom while still
 * being local enough not to grab an unrelated seed.
 */
export const MIN_ERASE_RADIUS_PX = 14;

/** Ceiling so a heavy zoom-in cannot turn one click into a multi-seed wipe. */
export const MAX_ERASE_RADIUS_PX = 48;

/**
 * Click radius in CSS px for the seed eraser.
 *
 * @param markerSize    `seedMarkerSize` (world/model-local units)
 * @param pxPerWorldUnit pixels per world unit = OrthographicCamera.zoom
 *                       (R3F sizes the ortho frustum to the canvas in CSS px,
 *                       so `zoom` is exactly px-per-world-unit)
 */
export function seedEraseRadiusPx(markerSize: number, pxPerWorldUnit: number): number {
  const raw = markerSize * GLYPH_RADIUS_UNITS * pxPerWorldUnit;
  if (!Number.isFinite(raw) || raw <= 0) return MIN_ERASE_RADIUS_PX;
  return Math.min(MAX_ERASE_RADIUS_PX, Math.max(MIN_ERASE_RADIUS_PX, raw));
}

/**
 * Pointer position in CSS pixels relative to the canvas.
 * `clientX/Y` and `getBoundingClientRect()` are both CSS-px (DPR-independent),
 * so this is a pure translation — no scaling, no devicePixelRatio.
 */
export function pointerToCanvasPx(
  clientX: number,
  clientY: number,
  rect: ScreenRect
): ScreenPoint {
  return { x: clientX - rect.left, y: clientY - rect.top };
}

/**
 * Nearest seed to the pointer within `radiusPx`, or `null` if none qualifies.
 *
 * `project` must return the seed's **rendered** on-screen position, i.e. it
 * owns the local→world conversion. Ties resolve to the lowest index so the
 * outcome is deterministic (stable ids make the click log readable).
 */
export function nearestSeedPx(
  seeds: readonly SeedLike[],
  project: (seed: SeedLike, index: number) => ScreenPoint,
  pointer: ScreenPoint,
  radiusPx: number
): { index: number; distancePx: number } | null {
  let best = -1;
  let bestD = Infinity;
  for (let i = 0; i < seeds.length; i++) {
    const p = project(seeds[i], i);
    if (!Number.isFinite(p.x) || !Number.isFinite(p.y)) continue;
    const d = Math.hypot(p.x - pointer.x, p.y - pointer.y);
    if (d < bestD) {
      bestD = d;
      best = i;
    }
  }
  if (best < 0 || bestD > radiusPx) return null;
  return { index: best, distancePx: bestD };
}
