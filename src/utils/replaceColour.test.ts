import { describe, it, expect } from "vitest";
import {
  planReplaceColour,
  replaceGuard,
  DEFAULT_FACE_COLOR_RGBA,
} from "./replaceColour";

// ── Issue #29: colour find & replace ───────────────────────────────
// One semantic only, per the owner: one click replaces EVERY region that
// used the source colour. The dry-run count and the guard decisions run
// over the frontend faceColors mirror (meshData.faceColors, flat RGBA
// bytes per face), so the preview updates instantly while the user picks
// — no IPC round trip. The APPLY is backend-authoritative; these numbers
// are advisory.

// 3 faces red (labels 1, 2, 2), 2 faces blue (labels 3, 3), 1 face
// default grey (label 4). faceColors is flat [r,g,b,a] * faces.
const RED: [number, number, number, number] = [139, 69, 19, 255];
const BLUE: [number, number, number, number] = [30, 144, 255, 255];

function fixture(): { faceColors: number[]; segmentLabels: number[] } {
  const one = (c: [number, number, number, number]) => [c[0], c[1], c[2], c[3]];
  const faceColors = [
    ...one(RED),
    ...one(RED),
    ...one(RED),
    ...one(BLUE),
    ...one(BLUE),
    ...one(DEFAULT_FACE_COLOR_RGBA),
  ];
  return { faceColors, segmentLabels: [1, 2, 2, 3, 3, 4] };
}

describe("planReplaceColour — dry-run count (#29)", () => {
  it("counts faces and DISTINCT regions carrying the colour", () => {
    const { faceColors, segmentLabels } = fixture();
    const plan = planReplaceColour(faceColors, segmentLabels, RED);
    expect(plan.faces).toBe(3);
    expect(plan.regions).toBe(2); // labels 1 and 2
  });

  it("alpha is part of the identity — a truncated tuple never matches", () => {
    const { faceColors, segmentLabels } = fixture();
    const translucent: [number, number, number, number] = [RED[0], RED[1], RED[2], 128];
    const plan = planReplaceColour(faceColors, segmentLabels, translucent);
    expect(plan.faces).toBe(0);
  });

  it("no face carries the colour → zeros (apply button disabled)", () => {
    const { faceColors, segmentLabels } = fixture();
    const plan = planReplaceColour(faceColors, segmentLabels, [1, 2, 3, 255]);
    expect(plan).toEqual({ faces: 0, regions: 0 });
  });

  it("default grey IS countable (the guard blocks the apply, not the math)", () => {
    const { faceColors, segmentLabels } = fixture();
    const plan = planReplaceColour(faceColors, segmentLabels, DEFAULT_FACE_COLOR_RGBA);
    expect(plan.faces).toBe(1);
    expect(plan.regions).toBe(1);
  });
});

describe("replaceGuard — the safety valves (#29)", () => {
  it("from == to → 'same' (a disabled no-op)", () => {
    expect(replaceGuard(RED, [...RED] as [number, number, number, number])).toBe("same");
  });

  it("replacing the unpainted default grey is refused — it would repaint every untouched face", () => {
    expect(replaceGuard(DEFAULT_FACE_COLOR_RGBA, BLUE)).toBe("defaultColour");
  });

  it("replacing TO the default grey is allowed (that is what the eraser writes)", () => {
    expect(replaceGuard(RED, DEFAULT_FACE_COLOR_RGBA)).toBeNull();
  });

  it("a normal replace passes", () => {
    expect(replaceGuard(RED, BLUE)).toBeNull();
  });
});
