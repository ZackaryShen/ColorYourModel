import { fireEvent, render, screen, cleanup } from '@testing-library/react';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import { TabBar } from './TabBar';
import { useAppStore } from '../store/appStore';
const actions = vi.hoisted(() => ({ switchTab: vi.fn(), closeTab: vi.fn(), renameTab: vi.fn() }));
vi.mock('../hooks/useProjectTabs', () => ({ useProjectTabs: () => actions }));
afterEach(cleanup);
beforeEach(() => { vi.clearAllMocks(); useAppStore.setState({ activeTabId:1, tabs:[{id:1,name:'One',faceCount:10,dirty:false},{id:2,name:'Two',faceCount:20,dirty:true}], paintDirty:false, language:'en', importBusy:false, isLoading:false, saveBusy:false }); });
it('switches by stable tab id and asks before closing an inactive dirty plate', () => {
  render(<TabBar/>);
  fireEvent.click(screen.getByRole('tab', {name:'Two •'}));
  expect(actions.switchTab).toHaveBeenCalledWith(2);
  fireEvent.click(screen.getByRole('button', {name:'Close plate Two'}));
  expect(actions.closeTab).not.toHaveBeenCalled();
  fireEvent.click(screen.getByRole('button', {name:'Cancel'}));
  expect(actions.closeTab).not.toHaveBeenCalled();
});
it('Escape while renaming leaves the original name unchanged', () => {
  render(<TabBar/>);
  fireEvent.doubleClick(screen.getByRole('tab', {name:'One'}));
  const input = screen.getByRole('textbox');
  fireEvent.change(input, {target:{value:'Discard this'}});
  fireEvent.keyDown(input, {key:'Escape'});
  expect(actions.renameTab).not.toHaveBeenCalled();
  expect(screen.getByRole('tab', {name:'One'})).toBeTruthy();
});
