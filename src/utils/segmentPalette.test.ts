import { describe, it, expect } from "vitest";
import { SEGMENT_PALETTE_SIZE, SEGMENT_COLORS_RGB } from "./segmentPalette";

// The fuse's adjacency-aware label allocation places each region on a colour
// slot `label % SEGMENT_PALETTE_SIZE`; the Rust side pins the same number via
// SEGMENT_PALETTE_SIZE in src-tauri/src/mesh/model.rs. If either side drifts,
// adjacent regions render in the same colour again.
describe("segmentPalette", () => {
  it("keeps 30 slots in sync with the backend's SEGMENT_PALETTE_SIZE", () => {
    expect(SEGMENT_PALETTE_SIZE).toBe(30);
    expect(SEGMENT_COLORS_RGB).toHaveLength(30);
  });

  it("has no duplicate colours", () => {
    const keys = new Set(SEGMENT_COLORS_RGB.map((c) => c.join(",")));
    expect(keys.size).toBe(SEGMENT_COLORS_RGB.length);
  });
});
