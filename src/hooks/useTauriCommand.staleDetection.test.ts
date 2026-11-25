import { act, renderHook } from '@testing-library/react';
import { beforeEach, expect, it, vi } from 'vitest';
import { invoke } from '@tauri-apps/api/core';
import { useAppStore } from '../store/appStore';
import { useTauriCommand } from './useTauriCommand';

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }));
beforeEach(() => {
  vi.mocked(invoke).mockReset();
  useAppStore.setState({activeTabId:1, tabEpoch:1, importBusy:false, saveBusy:false});
});

it.each(['planar', 'multiview', 'crossSection', 'eye', 'eyeAuto'] as const)(
  'rejects a stale %s detection instead of returning an unowned empty overlay', async kind => {
    let finish!: (value: unknown[]) => void;
    vi.mocked(invoke).mockImplementation(() => new Promise(resolve => { finish = resolve; }));
    const { result } = renderHook(() => useTauriCommand());
    const commands = result.current;
    const pending = kind === 'planar' ? commands.detectPlanarRegions(15, 1/30, 2)
      : kind === 'multiview' ? commands.detectMultiViewRegions(12,20,2,1)
      : kind === 'crossSection' ? commands.detectCrossSectionRegions(24,0.5)
      : kind === 'eye' ? commands.detectEyeRegions([0]) : commands.detectEyeRegionsAuto();
    const outcome = pending.then(value => ({value}), error => ({error}));
    act(() => useAppStore.setState({activeTabId:2, tabEpoch:2, statusMessage:'Current plate status'}));
    finish([]);
    const response = await outcome;
    expect(response).toHaveProperty('error');
    expect(String((response as {error: unknown}).error)).toContain('Tab changed');
    expect(useAppStore.getState().statusMessage).toBe('Current plate status');
  }
);
