import { beforeEach, expect, it } from 'vitest';
import { useAppStore } from './appStore';
import type { MeshData } from '../types/mesh';
const mesh = (tabId: number): MeshData => ({ vertices: [], faces: [], faceColors: [], segmentLabels: [], segments: [], faceCount: 0, bbox: { min: [0,0,0], max: [0,0,0] }, tabId, tabs: [1,2].map(id => ({ id, name: `Plate ${id}`, faceCount: 0, dirty: false })) } as MeshData);
beforeEach(() => useAppStore.setState({ tabs: [], activeTabId: null, tabViews: {}, paintDirty: false, hiddenSegments: new Set(), selectedSegment: null }));
it('retains dirty and hidden metadata when returning to an edited tab', () => {
  useAppStore.getState().setMeshData(mesh(1));
  useAppStore.setState({ paintDirty: true, hiddenSegments: new Set([7]) });
  useAppStore.getState().setMeshData(mesh(2));
  expect(useAppStore.getState().tabs.find(t => t.id === 1)?.dirty).toBe(true);
  expect(useAppStore.getState().paintDirty).toBe(false);
  useAppStore.getState().setMeshData(mesh(1));
  expect(useAppStore.getState().paintDirty).toBe(true);
  expect([...useAppStore.getState().hiddenSegments]).toEqual([7]);
});
it('clears transient region selection when switching tabs', () => {
  useAppStore.setState({ selectedSegment: 999, pendingPaintResult: { faces:[1], colors:[[255,0,0,255]] } });
  useAppStore.getState().setMeshData(mesh(2));
  expect(useAppStore.getState().selectedSegment).toBeNull();
  expect(useAppStore.getState().pendingPaintResult).toBeNull();
});
it('keeps current visibility when another tab closes without changing the active tab', () => {
  useAppStore.getState().setMeshData(mesh(1));
  useAppStore.setState({ hiddenSegments: new Set([7]) });
  useAppStore.getState().setMeshData(mesh(1));
  expect([...useAppStore.getState().hiddenSegments]).toEqual([7]);
});
it('does not carry geometric detector overlays or source face ids into a new plate', () => {
  useAppStore.setState({ planarRegions: [{} as never], multiviewRegions: [{} as never], crossSectionRegions: [{} as never], eyeRegions: [{} as never] });
  useAppStore.getState().setMeshData(mesh(2));
  expect(useAppStore.getState().planarRegions).toEqual([]);
  expect(useAppStore.getState().multiviewRegions).toEqual([]);
  expect(useAppStore.getState().crossSectionRegions).toEqual([]);
  expect(useAppStore.getState().eyeRegions).toEqual([]);
});
it('releases hidden-region metadata belonging to a closed plate', () => {
  useAppStore.getState().setMeshData(mesh(1));
  useAppStore.setState({ tabViews:{1:[7],2:[8]}, hiddenSegments:new Set([7]) });
  const remaining = mesh(2);
  remaining.tabs = remaining.tabs?.filter(tab => tab.id === 2);
  useAppStore.getState().setMeshData(remaining);
  expect(Object.keys(useAppStore.getState().tabViews)).toEqual(['2']);
});
