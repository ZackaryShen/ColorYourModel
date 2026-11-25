import { act, cleanup, render, screen } from '@testing-library/react';
import { afterEach, expect, it, vi } from 'vitest';
import { HistoryPanel } from './HistoryPanel';
import { useAppStore } from '../store/appStore';
import type { MeshData } from '../types/mesh';
const api = vi.hoisted(() => ({ historyEntries: vi.fn(), historyJump: vi.fn() }));
vi.mock('../hooks/useTauriCommand', () => ({ useTauriCommand: () => api }));
afterEach(cleanup);
it('removes old-tab history while the new tab timeline is still loading', async () => {
  useAppStore.setState({ activeTabId: 1, tabEpoch: 1, meshData: { tabId: 1 } as MeshData, language: 'en' });
  api.historyEntries.mockResolvedValueOnce({ current: 1, maxDepth: 50, bytes: 100, entries: [{kind:'paint',strokeId:null,faceCount:42,bytes:100}] });
  render(<HistoryPanel/>);
  await act(async () => {});
  expect(screen.getByText('42 faces')).toBeTruthy();
  api.historyEntries.mockImplementationOnce(() => new Promise(() => {}));
  await act(async () => { useAppStore.setState({ activeTabId: 2, tabEpoch: 2, meshData: {tabId:2} as MeshData }); });
  expect(screen.queryByText('42 faces')).toBeNull();
});
