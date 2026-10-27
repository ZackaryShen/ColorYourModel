import { describe, it, expect } from "vitest";
import { TOOL_GROUPS, TOOL_KEYS } from "./Toolbar";

// Regression test for the collapsible-toolbar-groups off-by-one: the segment
// group used `slice(7, 10)`, which double-booked the Eraser (also in the
// paint group) and dropped the Seed tool (the array's tail) entirely — the
// 🌱 button vanished from the bar, and with it the seed panel's auto/manual
// tabs. Every tool must render in exactly one group.

describe("toolbar groups", () => {
  it("places every non-View tool in exactly one group", () => {
    const grouped = TOOL_GROUPS.flatMap((g) => g.tools);
    const counts = new Map(grouped.map((t) => [t.tool, grouped.filter((x) => x.tool === t.tool).length]));

    for (const item of TOOL_KEYS) {
      if (item.tool === "view") {
        expect(counts.has(item.tool), "View renders above the groups, never inside one").toBe(false);
        continue;
      }
      expect(counts.get(item.tool), `${item.tool} must appear exactly once`).toBe(1);
    }
  });

  it("keeps the seed tool reachable — it opens the auto/manual segmentation panel", () => {
    const segment = TOOL_GROUPS.find((g) => g.id === "segment");
    expect(segment?.tools.some((t) => t.tool === "seed")).toBe(true);
  });

  it("keeps the eraser a paint tool — not duplicated into the segment group", () => {
    const paint = TOOL_GROUPS.find((g) => g.id === "paint");
    const segment = TOOL_GROUPS.find((g) => g.id === "segment");
    expect(paint?.tools.some((t) => t.tool === "eraser")).toBe(true);
    expect(segment?.tools.some((t) => t.tool === "eraser")).toBe(false);
  });
});
