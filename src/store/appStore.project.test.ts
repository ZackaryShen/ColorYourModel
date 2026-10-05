import { beforeEach, describe, expect, it } from "vitest";
import { useAppStore } from "./appStore";

/**
 * 0.2.0-P1 open-project semantics. Opening a .cym project replaces the mesh
 * via setMeshData, and the adversarial review flagged that setMeshData alone
 * leaves the previous model's seed/overlay/selection state behind — stale
 * selectedSegment can collide with a real region id, and stale seedPoints /
 * planarRegions carry the OLD model's face indices into new-model commands.
 * resetProjectTransientState is the guard; these tests pin it plus the
 * setMeshData reset semantics it leans on.
 */
const seedDirtyState = () => {
  useAppStore.setState({
    isLoaded: true,
    canUndo: true,
    canRedo: true,
    paintDirty: true,
    selectedSegment: 100000,
    hoveredSegment: 3,
    seedPoints: [{ x: 1, y: 2, z: 3, faceIndex: 9 }],
    seedEraseMode: true,
    seedPickMode: true,
    suggestedSeeds: [{ x: 4, y: 5, z: 6, faceIndex: 8 }],
    planarRegions: [
      { plane: [0, 0, 0, 0], faceCount: 4, seed: { x: 0, y: 0, z: 0, faceIndex: 0 }, boundaryEdges: [], faceIndices: [0, 1, 2] },
    ],
    multiviewRegions: [
      { seed: { x: 0, y: 0, z: 0, faceIndex: 0 }, boundaryEdges: [], faceIndices: [0, 1] },
    ],
    crossSectionRegions: [],
    eyeRegions: [],
    segmentView: true,
  } as Partial<ReturnType<typeof useAppStore.getState>>);
};

beforeEach(() => {
  seedDirtyState();
});

describe("resetProjectTransientState (open-project guard)", () => {
  it("clears seeds, overlay regions and selection", () => {
    useAppStore.getState().resetProjectTransientState();
    const s = useAppStore.getState();
    expect(s.seedPoints).toHaveLength(0);
    expect(s.seedEraseMode).toBe(false);
    expect(s.seedPickMode).toBe(false);
    expect(s.suggestedSeeds).toHaveLength(0);
    expect(s.planarRegions).toHaveLength(0);
    expect(s.multiviewRegions).toHaveLength(0);
    expect(s.crossSectionRegions).toHaveLength(0);
    expect(s.eyeRegions).toHaveLength(0);
    expect(s.selectedSegment).toBeNull();
    expect(s.hoveredSegment).toBeNull();
  });

  it("preserves segmentView (a view preference, not project data)", () => {
    useAppStore.getState().resetProjectTransientState();
    expect(useAppStore.getState().segmentView).toBe(true);
  });
});

describe("setMeshData reset semantics leaned on by open-project", () => {
  it("resets history flags and paintDirty, keeps segmentView untouched", () => {
    const data = {
      vertices: new Float32Array([0, 0, 0, 1, 0, 0, 0, 1, 0]),
      faces: new Uint32Array([0, 1, 2]),
      faceColors: new Array(12).fill(138),
      segmentLabels: [0],
      bbox: { min: [0, 0, 0], max: [1, 1, 0] },
      faceCount: 1,
      segments: [{ id: 0, name: "r0", color: null, faceCount: 1 }],
    };
    useAppStore.getState().setMeshData(data as Parameters<typeof useAppStore.getState.setMeshData>[0]);
    const s = useAppStore.getState();
    expect(s.canUndo).toBe(false);
    expect(s.canRedo).toBe(false);
    expect(s.paintDirty).toBe(false);
    expect(s.isLoaded).toBe(true);
    // segmentView untouched by setMeshData — asserted so a future "helpful"
    // reset here gets a deliberate second look instead of slipping in.
    expect(s.segmentView).toBe(true);
  });
});
