import { describe, expect, it } from "vitest";
import { PaintTool } from "../../types/mesh";
import { gradientGuideAppearance } from "./gradientGesture";

describe("gradient preview width", () => {
  it("uses the live brush diameter at the current view scale", () => {
    const live = { tool: PaintTool.Brush, radius: 3.5 } as const;
    expect(gradientGuideAppearance(live, 8)).toEqual({ linewidth: 56, outline: false });
    expect(gradientGuideAppearance({ ...live, radius: 7 }, 8).linewidth).toBe(112);
    expect(gradientGuideAppearance(live, 12).linewidth).toBe(84);
  });

  it("uses a small screen-space guide for fill direction and saved paths", () => {
    for (const live of [{ tool: PaintTool.Fill, radius: 7 } as const, null]) {
      expect(gradientGuideAppearance(live, 12)).toEqual({ linewidth: 3, outline: true });
    }
  });
});
