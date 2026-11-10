import { describe, it, expect } from "vitest";
import {
  GLYPH_RADIUS_UNITS,
  MAX_ERASE_RADIUS_PX,
  MIN_ERASE_RADIUS_PX,
  nearestSeedPx,
  pointerToCanvasPx,
  seedEraseRadiusPx,
  type ScreenPoint,
} from "./seedErase";

// Acceptance round 9: the seed eraser's coordinate-space fix.
// Regression lock for the two pieces that actually decide a click:
//   • the click radius (world marker size -> CSS px), and
//   • nearest-seed selection in pixels.
// The local→world→NDC step lives in the Viewport (it needs three.js); it is
// exercised by tools/seed-erase-forensics/ (same math, real engine).

describe("seedEraseRadiusPx", () => {
  it("scales the drawn glyph with zoom (size 0.24wu @ zoom 10 = 8.4px)", () => {
    // 0.24 * 3.5 * 10 = 8.4 -> below the floor, so the floor wins.
    expect(seedEraseRadiusPx(0.24, 10)).toBe(MIN_ERASE_RADIUS_PX);
  });

  it("returns the glyph radius when it is between the bounds", () => {
    // 0.24 * 3.5 * 40 = 33.6px
    expect(seedEraseRadiusPx(0.24, 40)).toBeCloseTo(0.24 * GLYPH_RADIUS_UNITS * 40, 6);
    expect(seedEraseRadiusPx(0.24, 40)).toBeGreaterThan(MIN_ERASE_RADIUS_PX);
  });

  it("floors tiny markers so a 1-2px dot still gets a human-sized target", () => {
    // The pre-fix world threshold was 2.5*size == ~2.5px at fit zoom.
    expect(seedEraseRadiusPx(0.096, 1)).toBe(MIN_ERASE_RADIUS_PX);
    expect(MIN_ERASE_RADIUS_PX).toBeGreaterThanOrEqual(10);
  });

  it("caps heavy zoom-in so one click cannot wipe a cluster", () => {
    expect(seedEraseRadiusPx(1, 10_000)).toBe(MAX_ERASE_RADIUS_PX);
  });

  it("falls back to the floor for degenerate sizes (no NaN/0 radius)", () => {
    expect(seedEraseRadiusPx(0, 10)).toBe(MIN_ERASE_RADIUS_PX);
    expect(seedEraseRadiusPx(Number.NaN, 10)).toBe(MIN_ERASE_RADIUS_PX);
    expect(seedEraseRadiusPx(0.2, 0)).toBe(MIN_ERASE_RADIUS_PX);
  });
});

describe("pointerToCanvasPx", () => {
  it("translates window coordinates into canvas-local CSS pixels", () => {
    const rect = { left: 244, top: 60, width: 900, height: 700 };
    expect(pointerToCanvasPx(344, 160, rect)).toEqual({ x: 100, y: 100 });
  });

  it("keeps sub-pixel fractions (no rounding drift mid-smear)", () => {
    const rect = { left: 0.5, top: 0.25, width: 100, height: 100 };
    expect(pointerToCanvasPx(10.75, 20.5, rect)).toEqual({ x: 10.25, y: 20.25 });
  });
});

describe("nearestSeedPx", () => {
  const at = (x: number, y: number): ScreenPoint => ({ x, y });
  const fromTable = (table: ScreenPoint[]) => (seed: { x: number; y: number }) =>
    table[seed.x];

  it("returns null with no seeds", () => {
    expect(nearestSeedPx([], () => at(0, 0), at(0, 0), 32)).toBeNull();
  });

  it("picks the nearest seed when inside the radius", () => {
    const table = [at(100, 100), at(120, 100), at(400, 400)];
    const seeds = [{ x: 0, y: 0, z: 0 }, { x: 1, y: 0, z: 0 }, { x: 2, y: 0, z: 0 }];
    const hit = nearestSeedPx(seeds, fromTable(table), at(118, 101), 32);
    expect(hit?.index).toBe(1);
    expect(hit?.distancePx).toBeCloseTo(Math.hypot(2, 1), 6);
  });

  it("rejects everything outside the radius (no accidental far erases)", () => {
    const table = [at(100, 100)];
    expect(nearestSeedPx([{ x: 0, y: 0, z: 0 }], fromTable(table), at(100, 100 + 33), 32)).toBeNull();
    // ...and the same click passes at 31px.
    expect(nearestSeedPx([{ x: 0, y: 0, z: 0 }], fromTable(table), at(100, 100 + 31), 32)?.index).toBe(0);
  });

  it("is inclusive exactly at the radius", () => {
    const table = [at(100, 100)];
    expect(nearestSeedPx([{ x: 0, y: 0, z: 0 }], fromTable(table), at(132, 100), 32)?.index).toBe(0);
    expect(nearestSeedPx([{ x: 0, y: 0, z: 0 }], fromTable(table), at(132.5, 100), 32)).toBeNull();
  });

  it("resolves ties to the lowest index (deterministic click logs)", () => {
    const table = [at(110, 100), at(90, 100)];
    const seeds = [{ x: 0, y: 0, z: 0 }, { x: 1, y: 0, z: 0 }];
    expect(nearestSeedPx(seeds, fromTable(table), at(100, 100), 32)?.index).toBe(0);
  });

  it("skips non-finite projections instead of poisoning the search", () => {
    const table = [at(Number.NaN, 0), at(101, 100)];
    const seeds = [{ x: 0, y: 0, z: 0 }, { x: 1, y: 0, z: 0 }];
    const hit = nearestSeedPx(seeds, fromTable(table), at(100, 100), 32);
    expect(hit?.index).toBe(1);
  });

  it("uses the RENDERED projection (local->world already applied by caller)", () => {
    // The regression this whole round is about: a local seed at (x, z, -y)
    // projects ~hundreds of px away from where it is drawn. The helper must
    // be fed the rendered position; this test pins the contract.
    const seed = [{ x: 12, y: 40, z: 70 }];
    const rendered = at(500, 300);
    const phantom = at(272, 471); // what project() on the raw local coords gave
    expect(nearestSeedPx(seed, () => rendered, at(501, 299), 32)?.index).toBe(0);
    expect(nearestSeedPx(seed, () => phantom, at(501, 299), 32)).toBeNull();
  });
});
