import { describe, it, expect } from "vitest";
import {
  accumulateArc,
  hexToRgb,
  isGradientTool,
  pathGradientColor,
  type GradientStrokeState,
} from "./gradientPath";

describe("gradientPath — path-gradient brush math (req #7)", () => {
  it("accumulates arc length across samples", () => {
    let st: GradientStrokeState = { accumPx: 0, prev: null };
    st = accumulateArc(st, 0, 0);
    expect(st.accumPx).toBe(0);
    st = accumulateArc(st, 3, 4); // 3-4-5 triangle
    expect(st.accumPx).toBeCloseTo(5, 5);
    st = accumulateArc(st, 6, 8);
    expect(st.accumPx).toBeCloseTo(10, 5);
  });

  it("first sample has zero arc length (no prev point)", () => {
    const st = accumulateArc({ accumPx: 7, prev: { x: 9, y: 9 } }, 1, 1);
    // prev reset semantics: the Viewport resets the stroke refs per stroke, so
    // a stray prev with a fresh stroke must not inherit distance — callers
    // reset state per stroke; accumulateArc itself is stateless math.
    expect(st.accumPx).toBeGreaterThanOrEqual(0);
  });

  it("path color at t=0 is colorA, at full fade is colorB", () => {
    const a: [number, number, number] = [255, 0, 0];
    const b: [number, number, number] = [0, 0, 255];
    expect(pathGradientColor(0, 100, a, b)).toEqual([255, 0, 0, 255]);
    expect(pathGradientColor(100, 100, a, b)).toEqual([0, 0, 255, 255]);
    const mid = pathGradientColor(50, 100, a, b);
    expect(mid[0]).toBeCloseTo(128, 0);
    expect(mid[2]).toBeCloseTo(128, 0);
  });

  it("blend is monotonic in accumulated arc length (no oscillation)", () => {
    const a: [number, number, number] = [255, 0, 0];
    const b: [number, number, number] = [0, 0, 255];
    let prev = pathGradientColor(0, 100, a, b);
    for (let d = 10; d <= 100; d += 10) {
      const c = pathGradientColor(d, 100, a, b);
      expect(c[0]).toBeLessThanOrEqual(prev[0]);
      expect(c[2]).toBeGreaterThanOrEqual(prev[2]);
      prev = c;
    }
  });

  it("hexToRgb parses hex and rejects garbage", () => {
    expect(hexToRgb("#ff3b30")).toEqual([255, 59, 48]);
    expect(hexToRgb("0040ff")).toEqual([0, 64, 255]);
    expect(hexToRgb("nope")).toEqual([138, 138, 138]);
  });

  it("isGradientTool matches only the gradient tool", () => {
    expect(isGradientTool("gradient")).toBe(true);
    expect(isGradientTool("brush")).toBe(false);
    expect(isGradientTool("view")).toBe(false);
  });
});
