import { invoke } from '@tauri-apps/api/core';
import { useAppStore } from '../store/appStore';

type TabOwnership = { tabId: number | null; epoch: number };
const owners = new WeakMap<object, TabOwnership>();
const reads = new Set(['history_state', 'history_entries', 'get_face_color', 'used_colors', 'paint_source_groups', 'paint_source_faces', 'export_palette_preview', 'set_hidden_labels']);

export function captureTabOwnership(): TabOwnership {
  const state = useAppStore.getState();
  return { tabId: state.activeTabId, epoch: state.tabEpoch };
}

export function isCurrentTabOwnership(owner: TabOwnership): boolean {
  const state = useAppStore.getState();
  return owner.tabId === state.activeTabId && owner.epoch === state.tabEpoch;
}

export function isCurrentTabResult(value: unknown): boolean {
  if (!value || typeof value !== 'object') return true;
  const owner = owners.get(value);
  return !owner || isCurrentTabOwnership(owner);
}

/** Preserve ownership when a consumer enriches an IPC response. */
export function carryTabResult<T extends object>(source: unknown, target: T): T {
  if (source && typeof source === 'object') {
    const owner = owners.get(source);
    if (owner) owners.set(target, owner);
  }
  return target;
}

/** Bind both request and final application to the initiating tab generation. */
export async function invokeTab<T>(command: string, args: Record<string, unknown> = {}, tabId = useAppStore.getState().activeTabId): Promise<T> {
  const state = useAppStore.getState();
  const owner = { tabId, epoch: state.tabEpoch };
  if ((state.importBusy || state.saveBusy) && !reads.has(command) && !command.startsWith('export_')) {
    throw new Error('Project operation in progress');
  }
  const value = await invoke<T>(command, { ...args, ...(tabId == null ? {} : { tabId }) });
  if (value && typeof value === 'object') owners.set(value, owner);
  if (!isCurrentTabOwnership(owner)) throw new Error('Tab changed; stale result discarded');
  return value;
}
