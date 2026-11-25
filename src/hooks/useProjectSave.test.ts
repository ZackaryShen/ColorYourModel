import { act, renderHook } from '@testing-library/react';
import { beforeEach, expect, it, vi } from 'vitest';
import { useAppStore } from '../store/appStore';
import { useProjectSave } from './useProjectSave';
import type { MeshData } from '../types/mesh';
const mocks = vi.hoisted(() => ({ save: vi.fn(), export3mf: vi.fn() }));
vi.mock('@tauri-apps/plugin-dialog', () => ({ save: mocks.save }));
vi.mock('./useTauriCommand', () => ({ useTauriCommand: () => ({ export3mf: mocks.export3mf }) }));
beforeEach(() => {
  vi.clearAllMocks();
  useAppStore.setState({ meshData: { plateCount: 1 } as MeshData, projectPath: '/tmp/source.3mf', importSourcePath: null, isLoading: false, importBusy: false, saveBusy: false, pendingImportPath: null, language: 'en' });
  mocks.export3mf.mockResolvedValue('Exported to /tmp/source.3mf');
});
it('keeps the current path when the backend rejects a multi-plate overwrite', async () => {
  useAppStore.setState({ meshData: { plateCount: 3 } as MeshData });
  mocks.export3mf.mockRejectedValue('Cannot overwrite the source multi-plate project');
  const { result } = renderHook(() => useProjectSave());
  await act(() => result.current.saveProject());
  expect(useAppStore.getState().projectPath).toBe('/tmp/source.3mf');
});
it('can save normally after saving a loaded plate to a different file', async () => {
  useAppStore.setState({ meshData: { plateCount: 3 } as MeshData });
  mocks.save.mockResolvedValue('/tmp/plate.3mf');
  mocks.export3mf.mockResolvedValue('Exported to /tmp/plate.3mf');
  const { result } = renderHook(() => useProjectSave());
  await act(() => result.current.saveProjectAs());
  await act(() => result.current.saveProject());
  expect(mocks.export3mf).toHaveBeenCalledTimes(2);
  expect(mocks.export3mf.mock.calls[1][0]).toBe('/tmp/plate.3mf');
});
it('does not save the old mesh while another import is loading', async () => {
  useAppStore.setState({ isLoading: true });
  const { result } = renderHook(() => useProjectSave());
  await act(() => result.current.saveProject());
  expect(mocks.export3mf).not.toHaveBeenCalled();
});
it('contains save-dialog rejection and leaves the project path unchanged', async () => {
  mocks.save.mockRejectedValue('dialog unavailable');
  const { result } = renderHook(() => useProjectSave());
  await act(() => result.current.saveProjectAs());
  expect(mocks.export3mf).not.toHaveBeenCalled();
  expect(useAppStore.getState().projectPath).toBe('/tmp/source.3mf');
});
it('remembers the actual path returned by the backend', async () => {
  mocks.export3mf.mockResolvedValue('Exported to /tmp/actual.3mf');
  const { result } = renderHook(() => useProjectSave());
  await act(() => result.current.saveProject());
  expect(useAppStore.getState().projectPath).toBe('/tmp/actual.3mf');
});
it('does not save while the import file picker owns the workflow', async () => {
  useAppStore.setState({ importBusy: true });
  const { result } = renderHook(() => useProjectSave());
  await act(() => result.current.saveProject());
  expect(mocks.export3mf).not.toHaveBeenCalled();
});
it('serialises save requests across hook instances', async () => {
  let finish!: (value: string) => void;
  mocks.export3mf.mockImplementationOnce(() => new Promise<string>((resolve) => { finish = resolve; }));
  const one = renderHook(() => useProjectSave());
  const two = renderHook(() => useProjectSave());
  await act(async () => {
    const first = one.result.current.saveProject();
    await two.result.current.saveProject();
    finish('Exported to /tmp/source.3mf');
    await first;
  });
  expect(mocks.export3mf).toHaveBeenCalledTimes(1);
});

it.each(['/tmp/model.3mf.3mf', '/tmp/model.3MF', '/tmp/model-without-extension'])(
  'Save As writes exactly the native-dialog-confirmed path %s', async path => {
    mocks.save.mockResolvedValue(path);
    mocks.export3mf.mockResolvedValue(`Exported to ${path}`);
    const { result } = renderHook(() => useProjectSave());
    await act(() => result.current.saveProjectAs());
    expect(mocks.export3mf.mock.calls[0][0]).toBe(path);
    expect(useAppStore.getState().projectPath).toBe(path);
  }
);

it("uses a distinct CYM copy for the first save and writes there directly afterward", async () => {
  useAppStore.setState({ projectPath: null, importSourcePath: "/tmp/Model.3mf" });
  mocks.save.mockResolvedValue("/tmp/Model.cym.3mf");
  mocks.export3mf.mockResolvedValue("Exported to /tmp/Model.cym.3mf");
  const { result } = renderHook(() => useProjectSave());
  await act(() => result.current.saveProject());
  await act(() => result.current.saveProject());
  expect(mocks.save).toHaveBeenCalledTimes(1);
  expect(mocks.save.mock.calls[0][0].defaultPath).toBe("/tmp/Model.cym.3mf");
  expect(mocks.export3mf.mock.calls).toEqual([
    ["/tmp/Model.cym.3mf", undefined, true], ["/tmp/Model.cym.3mf", undefined, true],
  ]);
});
