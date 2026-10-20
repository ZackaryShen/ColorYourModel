import { beforeEach, describe, expect, it } from "vitest";
import { useAppStore } from "./appStore";

/**
 * Issue #16 merge multi-select. `mergePicked` is the ONE ordered picked list
 * shared by SegmentsPanel rows and the Viewport's Ctrl+click; `mergePicked[0]`
 * is the anchor (the region the others merge into) and `selectedSegment`
 * follows it so the viewport highlight keeps meaning "the surviving region".
 * These tests pin the toggle/anchor/prune semantics the UI leans on.
 */
const s = () => useAppStore.getState();

beforeEach(() => {
  useAppStore.setState({
    mergePicked: [],
    selectedSegment: null,
  } as Partial<ReturnType<typeof useAppStore.getState>>);
});

describe("toggleMergePicked (ordered merge multi-select)", () => {
  it("appends in pick order; first pick becomes the anchor and selectedSegment", () => {
    s().toggleMergePicked(7);
    s().toggleMergePicked(3);
    expect(s().mergePicked).toEqual([7, 3]);
    expect(s().selectedSegment).toBe(7);
  });

  it("toggling a picked label again removes it", () => {
    s().toggleMergePicked(7);
    s().toggleMergePicked(3);
    s().toggleMergePicked(7);
    expect(s().mergePicked).toEqual([3]);
  });

  it("re-adding keeps first-anchor order semantics", () => {
    // 7 anchored first, 3 joins, 7 removed (3 promotes), 7 re-added — it must
    // append BEHIND the promoted anchor, never reclaim the head.
    s().toggleMergePicked(7);
    s().toggleMergePicked(3);
    s().toggleMergePicked(7);
    s().toggleMergePicked(7);
    expect(s().mergePicked).toEqual([3, 7]);
    expect(s().selectedSegment).toBe(3);
  });

  it("selectedSegment follows the new anchor when the anchor is removed", () => {
    s().toggleMergePicked(7);
    s().toggleMergePicked(3);
    s().toggleMergePicked(7);
    expect(s().selectedSegment).toBe(3);
  });

  it("removing the last label empties the list without touching selectedSegment", () => {
    s().toggleMergePicked(7);
    s().setSelectedSegment(42);
    s().toggleMergePicked(7);
    expect(s().mergePicked).toEqual([]);
    // Clearing a pending merge is not a deselect (clear-not-deselect rule).
    expect(s().selectedSegment).toBe(42);
  });
});

describe("clearMergePicked", () => {
  it("empties the selection and leaves selectedSegment alone", () => {
    useAppStore.setState({
      mergePicked: [1, 2],
      selectedSegment: 1,
    } as Partial<ReturnType<typeof useAppStore.getState>>);
    s().clearMergePicked();
    expect(s().mergePicked).toEqual([]);
    expect(s().selectedSegment).toBe(1);
  });
});

describe("new-model path clears mergePicked", () => {
  it("setMeshData (import a fresh model) empties the merge selection", () => {
    useAppStore.setState({
      mergePicked: [4, 9],
      selectedSegment: 4,
    } as Partial<ReturnType<typeof useAppStore.getState>>);
    s().setMeshData({
      vertices: [0, 0, 0],
      faces: [0, 0, 0],
      faceColors: [1, 1, 1, 1],
      segmentLabels: [0],
      bbox: { min: [0, 0, 0], max: [1, 1, 1] },
      faceCount: 1,
      segments: [],
    });
    expect(s().mergePicked).toEqual([]);
  });
});
