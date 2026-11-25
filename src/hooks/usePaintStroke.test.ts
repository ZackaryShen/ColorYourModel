import { act, renderHook } from '@testing-library/react';
import * as React from 'react';
import { expect, it, vi } from 'vitest';
import { usePaintStroke } from './usePaintStroke';

it('assigns a new stroke after switching away and remounting a plate view', () => {
  const firstView = renderHook(() => usePaintStroke());
  let firstStroke = 0;
  act(() => { firstStroke = firstView.result.current.beginStroke(); });
  firstView.unmount();

  const returningView = renderHook(() => usePaintStroke());
  let nextStroke = 0;
  act(() => { nextStroke = returningView.result.current.beginStroke(); });
  expect(nextStroke).toBeGreaterThan(firstStroke);
  expect(returningView.result.current.activeStrokeIdRef.current).toBe(nextStroke);
  returningView.unmount();
});

it('keeps stroke IDs unique when the module reloads within a renderer session', async () => {
  const view = renderHook(() => usePaintStroke());
  const previousStroke = view.result.current.beginStroke();
  view.unmount();
  vi.resetModules();
  // Preserve the renderer's real React singleton across the simulated HMR.
  vi.doMock('react', () => React);
  try {
    const reloaded = await import('./usePaintStroke');
    const updatedView = renderHook(() => reloaded.usePaintStroke());
    expect(updatedView.result.current.beginStroke()).toBeGreaterThan(previousStroke);
    updatedView.unmount();
  } finally {
    vi.doUnmock('react');
  }
});
