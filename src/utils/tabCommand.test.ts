import { beforeEach, expect, it, vi } from 'vitest';
import { useAppStore } from '../store/appStore';
import { invokeTab } from './tabCommand';
const api = vi.hoisted(() => ({ invoke: vi.fn() }));
vi.mock('@tauri-apps/api/core', () => api);
beforeEach(() => { vi.clearAllMocks(); useAppStore.setState({ activeTabId: 1, tabEpoch: 0, importBusy: false, saveBusy: false }); });
it('sends the initiating tab id', async () => {
  api.invoke.mockResolvedValue('ok');
  expect(await invokeTab('undo')).toBe('ok');
  expect(api.invoke).toHaveBeenCalledWith('undo', { tabId: 1 });
});
it('rejects an old response after the user switched tabs', async () => {
  let finish!: (v: string) => void;
  api.invoke.mockImplementation(() => new Promise(r => { finish = r; }));
  const result = invokeTab('brush_paint', { faceId: 3 });
  useAppStore.setState({ activeTabId: 2 });
  finish('old result');
  await expect(result).rejects.toThrow('stale result');
});

it('checks result ownership at application time after another microtask switches tabs', async () => {
  let finish!: (v: { faces: number[] }) => void;
  api.invoke.mockImplementation(() => new Promise(r => { finish = r; }));
  const { isCurrentTabResult } = await import('./tabCommand');
  const applied: number[][] = [];
  const pending = (async () => {
    const result = await invokeTab<{ faces: number[] }>('brush_paint');
    if (isCurrentTabResult(result)) applied.push(result.faces);
  })();
  finish({ faces: [9] });
  queueMicrotask(() => useAppStore.setState({ activeTabId: 2 }));
  await pending;
  expect(applied).toEqual([]);
});

it('does not revive an old response after leaving and returning to the same stable tab', async () => {
  api.invoke.mockResolvedValue({ faces: [9] });
  const { isCurrentTabResult } = await import('./tabCommand');
  const result = await invokeTab('brush_paint');
  useAppStore.setState({ tabEpoch: 1 });
  expect(isCurrentTabResult(result)).toBe(false);
});

it('carries ownership when a tool adds routing fields to an IPC result', async () => {
  api.invoke.mockResolvedValue({ faces: [9] });
  const { carryTabResult, isCurrentTabResult } = await import('./tabCommand');
  const result = await invokeTab<{ faces: number[] }>('brush_paint');
  const outcome = carryTabResult(result, { ...result, routing: 'segment' });
  useAppStore.setState({ activeTabId: 2 });
  expect(isCurrentTabResult(outcome)).toBe(false);
});
