import { expect, it, vi } from 'vitest';
import { showTab } from './useProjectTabs';
import { useAppStore } from '../store/appStore';
vi.mock('@tauri-apps/api/core', () => ({ invoke:vi.fn() }));
it('closing the last plate clears the old save destination and all transient paint', async () => {
  useAppStore.setState({ projectPath:'/old-project.3mf', segments:[{id:1,name:'Old region',faceCount:1,color:null}], activeTabId:2, tabs:[{id:2,name:'Old',dirty:true,faceCount:1}], pendingPaintResult:{faces:[0],colors:[[1,2,3,255]]} });
  await showTab(null);
  expect(useAppStore.getState().projectPath).toBeNull();
  expect(useAppStore.getState().activeTabId).toBeNull();
  expect(useAppStore.getState().pendingPaintResult).toBeNull();
  expect(useAppStore.getState().segments).toEqual([]);
});
it('keeps engineering loss warnings visible when lazily switching to a plate', async () => {
  const { invoke } = await import('@tauri-apps/api/core');
  vi.mocked(invoke).mockResolvedValue({canUndo:false,canRedo:false});
  useAppStore.setState({language:'en'});
  await showTab({vertices:[],faces:[],faceColors:[],segmentLabels:[],segments:[],faceCount:0,bbox:{min:[0,0,0],max:[0,0,0]},tabId:2,tabs:[{id:2,name:'Bad state',faceCount:0,dirty:false}],engineeringState:'dropped',plateIndex:2,plateCount:3});
  expect(useAppStore.getState().statusMessage).toContain('Engineering data does not match');
});
