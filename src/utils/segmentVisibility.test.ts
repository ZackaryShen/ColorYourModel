import { describe, it, expect } from "vitest";
import {
  toggleHiddenLabels,
  isolateLabel,
  pruneHiddenLabels,
  filterVisibleHits,
} from "./segmentVisibility";

// Issue #24: per-region visibility. The hidden set lives in the frontend view
// state only — display never touches labels/colours, the undo timeline, or
// exports. These are the rules the set must obey, pinned before the shader
// and panel wiring land on top.

describe("toggleHiddenLabels", () => {
  it("hides a visible label", () => {
    const next = toggleHiddenLabels(new Set(), [7]);
    expect([...next]).toEqual([7]);
  });

  it("shows a hidden label again (toggle off)", () => {
    const next = toggleHiddenLabels(new Set([7]), [7]);
    expect(next.size).toBe(0);
  });

  it("a multi-selection that is partly hidden hides all of it (net toggle)", () => {
    // Picked rows 1 (visible) and 2 (hidden): the user's intent when clicking
    // the eye on the selection is "hide these" — not a per-row coin flip.
    const next = toggleHiddenLabels(new Set([2]), [1, 2]);
    expect(next.has(1)).toBe(true);
    expect(next.has(2)).toBe(true);
  });

  it("a fully hidden multi-selection is shown again", () => {
    const next = toggleHiddenLabels(new Set([1, 2]), [1, 2]);
    expect(next.size).toBe(0);
  });

  it("leaves unrelated labels untouched", () => {
    const next = toggleHiddenLabels(new Set([9]), [1]);
    expect(next.has(9)).toBe(true);
    expect(next.has(1)).toBe(true);
  });
});

describe("isolateLabel", () => {
  it("hides every label except the isolated one", () => {
    const next = isolateLabel([1, 2, 3, 100_001], 2);
    expect(next.has(1)).toBe(true);
    expect(next.has(3)).toBe(true);
    expect(next.has(100_001)).toBe(true);
    expect(next.has(2)).toBe(false);
  });

  it("isolating one of two regions hides exactly the other", () => {
    const next = isolateLabel([1, 2], 1);
    expect([...next]).toEqual([2]);
  });
});

describe("pruneHiddenLabels", () => {
  it("drops labels that no longer exist (merge / reset / undo)", () => {
    const next = pruneHiddenLabels(new Set([1, 2, 100_001]), new Set([1, 3]));
    expect([...next]).toEqual([1]);
  });

  it("keeps nothing when no labels survive", () => {
    expect(pruneHiddenLabels(new Set([1, 2]), new Set()).size).toBe(0);
  });
});

// ─── Hidden faces are unselectable (issue #24, requirement: hidden regions
// must not be painted or filled) ────────────────────────────────────────

describe("filterVisibleHits", () => {
  const hits = (faceIndices: (number | null)[]) =>
    faceIndices.map((faceIndex) => ({ faceIndex }));
  const labels = [1, 2, 3, 4];

  it("drops hits on hidden regions, keeping the first visible one", () => {
    // faceIndex 1 carries label 2 (hidden); faceIndex 2 -> label 3, 3 -> 4.
    const visible = filterVisibleHits(hits([2, 1, 3]), new Set([2]), labels);
    expect(visible.map((h) => h.faceIndex)).toEqual([2, 3]);
  });

  it("returns nothing when every hit is hidden", () => {
    // faceIndex 1 -> label 2 (hidden)
    expect(filterVisibleHits(hits([1, 1]), new Set([2]), labels)).toEqual([]);
  });

  it("passes through hits without face info and faces without a label", () => {
    // faceIndex 0 -> label 1, which is hidden; the null-face hit survives.
    const visible = filterVisibleHits(hits([null, 0]), new Set([1, 2, 3, 4]), labels);
    expect(visible.map((h) => h.faceIndex)).toEqual([null]);
  });

  it("is a passthrough when nothing is hidden", () => {
    const raw = hits([3, 1]);
    expect(filterVisibleHits(raw, new Set(), labels)).toEqual(raw);
  });
});
