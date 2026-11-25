import { fireEvent, render, cleanup } from '@testing-library/react';
import { afterEach, expect, it } from 'vitest';
import { useState } from 'react';
import { flushSync } from 'react-dom';
import { useAppStore } from '../store/appStore';
import { DialogShell } from '../components/DialogShell';
import { listenViewportKeydown } from './viewportKeyboard';

let unlisten: (() => void) | undefined;
afterEach(() => { unlisten?.(); unlisten = undefined; cleanup(); document.body.replaceChildren(); });

it('a confirmation Escape is consumed even when cancellation immediately unmounts the dialog', () => {
  const seeds = [{x:1,y:2,z:3,faceIndex:0}];
  useAppStore.setState({activeTool:'seed', seedPoints:seeds});
  function Confirmation() {
    const [open,setOpen] = useState(true);
    return open ? <DialogShell title="Reset?" body="Discard work?" icon="!" onCancel={() => flushSync(() => setOpen(false))}><button>Cancel</button></DialogShell> : null;
  }
  const view = render(<Confirmation/>);
  unlisten = listenViewportKeydown(event => {
    if (event.key === 'Escape') useAppStore.setState({activeTool:'view',seedPoints:[]});
  });
  fireEvent.keyDown(window, {key:'Escape'});
  expect(view.queryByRole('alertdialog')).toBeNull();
  expect(useAppStore.getState().activeTool).toBe('seed');
  expect(useAppStore.getState().seedPoints).toEqual(seeds);
});

it.each(['input', 'textarea', 'select', 'editable'] as const)('editing a %s cannot run viewport Backspace or Enter actions', target => {
  let points = 3;
  unlisten = listenViewportKeydown(event => {
    if (event.key === 'Backspace') { event.preventDefault(); points--; }
    if (event.key === 'Enter') points = 0;
  });
  const field = target === 'editable' ? document.createElement('span') : document.createElement(target);
  if (target === 'editable') field.setAttribute('contenteditable','true');
  document.body.append(field);
  expect(fireEvent.keyDown(field, {key:'Backspace'})).toBe(true);
  fireEvent.keyDown(field, {key:'Enter'});
  expect(points).toBe(3);
  field.remove();
});

it('IME composition and an already handled event cannot commit a model action', () => {
  let commits = 0;
  unlisten = listenViewportKeydown(() => { commits++; });
  fireEvent.keyDown(window, {key:'Enter', isComposing:true});
  const consumed = new KeyboardEvent('keydown', {key:'Enter', cancelable:true});
  consumed.preventDefault();
  window.dispatchEvent(consumed);
  expect(commits).toBe(0);
});

it('ordinary canvas shortcuts still run exactly once', () => {
  let points = 3;
  unlisten = listenViewportKeydown(event => { if (event.key === 'Backspace') { points--; event.preventDefault(); } });
  const canvas = document.createElement('canvas');
  document.body.append(canvas);
  expect(fireEvent.keyDown(canvas, {key:'Backspace'})).toBe(false);
  expect(points).toBe(2);
  canvas.remove();
});
