import { decodeMeshResponse } from "../utils/meshResponse";
import { importStatusMessage } from "./useImportStl";
import { translate } from "../i18nDict";
import { invoke } from '@tauri-apps/api/core';
import { useAppStore } from '../store/appStore';
import type { MeshData, HistoryState, TabInfo } from '../types/mesh';

export async function showTab(data: MeshData | null) {
  const state = useAppStore.getState();
  const path = state.projectPath;
  if (!data) {
    useAppStore.setState({ meshData: null, segments: [], statusMessage: "status.ready", projectPath: null, importSourcePath: null, tabEpoch: state.tabEpoch + 1, isLoaded: false, tabs: [], activeTabId: null, paintDirty: false, canUndo: false, canRedo: false, hiddenSegments: new Set(), tabViews: {}, pendingPaintResult: null, selectedSegment: null, hoveredSegment: null, mergePicked: [], paintSourceHoverFaces: null, seedPoints: [], suggestedSeeds: [], planarRegions: [], multiviewRegions: [], crossSectionRegions: [], eyeRegions: [] });
    return;
  }
  state.setMeshData(data);
  state.setProjectPath(path);
  const history = await invoke<HistoryState>('history_state', { tabId: data.tabId }).catch(() => null);
  if (useAppStore.getState().activeTabId === data.tabId) {
    if (history) state.setHistoryFlags(history.canUndo, history.canRedo);
    state.setStatusMessage(importStatusMessage(data, (key, ...args) => translate(key, useAppStore.getState().language, ...args)));
  }
}

export function workspaceDirty() {
  const state = useAppStore.getState();
  return state.paintDirty || state.tabs.some(tab => tab.dirty);
}

export function useProjectTabs() {
  const run = async (command: string, tabId: number) => {
    const state = useAppStore.getState();
    if (state.isLoading || state.importBusy || state.saveBusy || state.pendingImportPath || state.pendingCloseTabId != null) return;
    useAppStore.setState({ importBusy: true, loadingKind: "import", importProgress: 0, importStage: translate("tabs.loading", state.language) });
    state.setLoading(true);
    try { await showTab(decodeMeshResponse(await invoke<ArrayBuffer | MeshData | null>(command, { tabId }))); }
    catch (e) { state.setStatusMessage(String(e)); }
    finally { state.setLoading(false); useAppStore.setState({ importBusy: false }); }
  };
  return {
    switchTab: (tabId: number) => run('switch_tab', tabId),
    closeTab: (tabId: number) => run('close_tab', tabId),
    renameTab: async (tabId: number, name: string) => {
      const state = useAppStore.getState();
      if (state.importBusy || state.saveBusy || state.isLoading || state.pendingImportPath || state.pendingCloseTabId != null) return;
      useAppStore.setState({ importBusy: true });
      try {
        const tabs = await invoke<TabInfo[]>('rename_tab', { tabId, name });
        useAppStore.setState({ tabs, paintDirty: state.paintDirty || Boolean(tabs.find(tab => tab.id === state.activeTabId)?.dirty) });
      } catch (e) { state.setStatusMessage(String(e)); }
      finally { useAppStore.setState({ importBusy: false }); }
    },
  };
}
