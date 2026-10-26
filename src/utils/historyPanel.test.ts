import { describe, it, expect } from "vitest";
import { buildHistoryRows, formatHeaderBytes, type HistoryPanelEntry } from "./historyPanel";

// Issue #22: the history panel's row model. One row per applied operation
// PLUS a leading pristine-state row (Photoshop's "Open"), a current-state
// marker, and a redo branch below it. Row depth IS the jump target the
// backend's history_jump takes.

const entry = (over: Partial<HistoryPanelEntry> = {}): HistoryPanelEntry => ({
  kind: "paint",
  strokeId: null,
  faceCount: 12,
  bytes: 480,
  ...over,
});

describe("buildHistoryRows", () => {
  it("shows only the pristine row on a fresh timeline, marked current", () => {
    const rows = buildHistoryRows([], 0);
    expect(rows).toHaveLength(1);
    expect(rows[0]).toMatchObject({ depth: 0, kind: null, isCurrent: true, isRedo: false });
  });

  it("builds one row per applied op after the pristine row, depths in order", () => {
    const rows = buildHistoryRows(
      [entry({ kind: "paint" }), entry({ kind: "merge", faceCount: 0 }), entry({ kind: "transform" })],
      3
    );
    expect(rows.map((r) => r.depth)).toEqual([0, 1, 2, 3]);
    expect(rows.map((r) => r.kind)).toEqual([null, "paint", "merge", "transform"]);
    // Metadata rides through verbatim — the panel must not reshape it.
    expect(rows[3]).toMatchObject({ kind: "transform", faceCount: 12, bytes: 480, strokeId: null });
  });

  it("marks exactly the current state; rows above applied, below greyed as redo", () => {
    const rows = buildHistoryRows([entry(), entry(), entry()], 2);
    expect(rows.map((r) => r.isCurrent)).toEqual([false, false, true, false]);
    expect(rows.map((r) => r.isRedo)).toEqual([false, false, false, true]);
    expect(rows[3].depth).toBe(3);
  });

  it("marks the pristine row current when sitting at depth 0", () => {
    const rows = buildHistoryRows([entry(), entry()], 0);
    expect(rows[0].isCurrent).toBe(true);
    expect(rows.map((r) => r.isRedo)).toEqual([false, true, true]);
  });

  it("has no redo rows when sitting at the tip", () => {
    const rows = buildHistoryRows([entry(), entry()], 2);
    expect(rows.some((r) => r.isRedo)).toBe(false);
    expect(rows[2].isCurrent).toBe(true);
  });
});

describe("formatHeaderBytes", () => {
  it("formats the panel header's footprint readably", () => {
    expect(formatHeaderBytes(0)).toBe("0 B");
    expect(formatHeaderBytes(999)).toBe("999 B");
    expect(formatHeaderBytes(512 * 1024)).toBe("512 KB");
    expect(formatHeaderBytes(Math.round(3.4 * 1024 * 1024))).toBe("3.4 MB");
    expect(formatHeaderBytes(256 * 1024 * 1024)).toBe("256.0 MB");
  });
});
