import { useState } from 'react';
import { useAppStore } from '../store/appStore';
import { useProjectTabs } from '../hooks/useProjectTabs';
import { DialogShell, dialogCancelStyle, dialogDangerStyle } from './DialogShell';
import { useT } from '../i18n';

export function TabBar() {
  const t = useT();
  const tabs = useAppStore(s => s.tabs);
  const active = useAppStore(s => s.activeTabId);
  const dirty = useAppStore(s => s.paintDirty);
  const busy = useAppStore(s => s.importBusy || s.saveBusy || s.isLoading);
  const { switchTab, closeTab, renameTab } = useProjectTabs();
  const closing = useAppStore(s => s.pendingCloseTabId);
  const setClosing = (id: number | null) => useAppStore.setState({ pendingCloseTabId: id });
  const [editing, setEditing] = useState<number | null>(null);
  const [name, setName] = useState('');
  if (!tabs.length) return null;
  return <>
    <div role="tablist" aria-label={t('tabs.plates')} style={{ display:'flex', gap:4, overflowX:'auto', padding:'6px 10px', background:'var(--bg-panel)', borderBottom:'1px solid var(--border)', flexShrink:0 }}>
      {tabs.map(tab => <div key={tab.id} style={{ display:'flex', alignItems:'center', gap:4, border:'1px solid var(--border)', borderRadius:5, background:tab.id === active ? 'var(--bg-active)' : 'transparent', flexShrink:0 }}>
        {editing === tab.id ? <input aria-label={t('tabs.name')} value={name} maxLength={128} autoFocus onChange={e => setName(e.target.value)} onBlur={e => { setEditing(null); if (e.currentTarget.dataset.cancelled !== "true" && name.trim() && name !== tab.name) void renameTab(tab.id,name.trim()); }} onKeyDown={e => { e.stopPropagation(); if(e.key === 'Enter') e.currentTarget.blur(); if(e.key === 'Escape') { e.currentTarget.dataset.cancelled = 'true'; setEditing(null); } }} /> :
          <button className="cym-btn" role="tab" aria-selected={tab.id === active} disabled={busy} title={tab.name} onClick={() => { if(tab.id !== active) void switchTab(tab.id); }} onDoubleClick={() => { setEditing(tab.id); setName(tab.name); }} style={{ background:'transparent', color:'var(--text-1)', border:0, padding:'6px 12px', maxWidth:220, overflow:'hidden', textOverflow:'ellipsis', whiteSpace:'nowrap' }}>{tab.name}{tab.dirty || (tab.id === active && dirty) ? ' •' : ''}</button>}
        <button className="cym-btn" aria-label={`${t('tabs.close')} ${tab.name}`} disabled={busy} style={{ border:0, background:'transparent', color:'var(--text-2)', padding:'4px 8px' }} onClick={() => { if(tab.dirty || (tab.id === active && dirty)) setClosing(tab.id); else void closeTab(tab.id); }}>×</button>
      </div>)}
    </div>
    {closing != null && <DialogShell icon="⚠️" title={t('tabs.closeTitle')} body={t('tabs.closeBody')} onCancel={() => setClosing(null)}>
      <button className="cym-btn" style={dialogCancelStyle} onClick={() => setClosing(null)}>{t('drop.replaceCancel')}</button>
      <button className="cym-btn" style={dialogDangerStyle} onClick={() => { const id = closing; setClosing(null); void closeTab(id); }}>{t('tabs.discardClose')}</button>
    </DialogShell>}
  </>;
}
