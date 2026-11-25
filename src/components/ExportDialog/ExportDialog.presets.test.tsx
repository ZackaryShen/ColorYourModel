import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { beforeEach, expect, it, vi } from 'vitest';
import { save } from '@tauri-apps/plugin-dialog';
import { invoke } from '@tauri-apps/api/core';
import { ExportDialog } from './ExportDialog';
import { useAppStore } from '../../store/appStore';
import type { ExportSelection, MachineSummary } from '../../types/export';
vi.mock('@tauri-apps/api/core', () => ({invoke:vi.fn()}));
vi.mock('@tauri-apps/api/event', () => ({listen:vi.fn(async () => () => {})}));
vi.mock('@tauri-apps/plugin-dialog', () => ({save:vi.fn()}));
const machine: MachineSummary = {
  id:'snapmaker_u1',name:'Snapmaker U1',vendor:'Snapmaker', nozzles:['0.4','0.6'],targetSlicers:['snapmaker_orca'],
  processes:[{name:'Standard 0.4',nozzles:['0.4'],layerHeight:'0.2'},{name:'Fine 0.4',nozzles:['0.4'],layerHeight:'0.1'},{name:'Standard 0.6',nozzles:['0.6'],layerHeight:'0.3'}],
  filaments:[{name:'PLA 0.4',nozzles:['0.4'],material:'PLA',vendor:'Generic'},{name:'PETG 0.4',nozzles:['0.4'],material:'PETG',vendor:'Generic'},{name:'PLA 0.6',nozzles:['0.6'],material:'PLA',vendor:'Generic'}],
};
let writtenSelection: ExportSelection | undefined;
beforeEach(() => {
  writtenSelection = undefined;
  vi.mocked(save).mockResolvedValue('/tmp/presets.3mf');
  useAppStore.setState({activeTabId:1,tabEpoch:1,language:'en',importBusy:false,saveBusy:false,lastExportSelection:null, meshData:{vertices:[0,0,0],faces:[0,0,0],faceColors:[255,0,0,255],segmentLabels:[1],segments:[],faceCount:1,bbox:{min:[0,0,0],max:[0,0,0]}}});
  vi.mocked(invoke).mockImplementation(async (command, args) => {
    if (command === 'export_3mf_command') { writtenSelection = (args as {selection:ExportSelection}).selection; return 'Exported to /tmp/presets.3mf'; }
    return command === 'list_export_presets' ? [machine] : ['#FF0000'];
  });
});
it('restores a persisted non-default process when it is valid for the nozzle', async () => {
  useAppStore.setState({lastExportSelection:{machineId:'snapmaker_u1',nozzleDiameter:'0.4',processName:'Fine 0.4',filamentNames:['PLA 0.4'],targetSlicer:'snapmaker_orca'}});
  render(<ExportDialog onClose={() => {}}/>);
  await screen.findByRole('option',{name:'Fine 0.4 (0.1mm)'});
  expect((screen.getByRole('option',{name:'Fine 0.4 (0.1mm)'}) as HTMLOptionElement).selected).toBe(true);
});
it('keeps all material choices consistent with the newly selected nozzle', async () => {
  render(<ExportDialog onClose={() => {}}/>);
  await screen.findByRole('option',{name:'Standard 0.4 (0.2mm)'});
  const materialSelects = screen.getAllByRole('combobox').filter(el => Array.from((el as HTMLSelectElement).options).some(o => o.value === 'PETG 0.4'));
  fireEvent.change(materialSelects[0],{target:{value:'PETG 0.4'}});
  fireEvent.click(screen.getByRole('button',{name:'0.6mm'}));
  await screen.findByRole('option',{name:'Standard 0.6 (0.3mm)'});
  fireEvent.click(screen.getByRole('button',{name:/Apply to all/i}));
  fireEvent.click(screen.getByRole('button',{name:'Choose location & export'}));
  await waitFor(() => expect(writtenSelection?.filamentNames).toEqual(['PLA 0.6']));
  await waitFor(() => {
    for (const el of screen.getAllByRole('combobox')) {
      const select=el as HTMLSelectElement;
      expect(select.value).not.toBe('');
      expect(Array.from(select.options).some(o => o.value === select.value)).toBe(true);
    }
  });
});
