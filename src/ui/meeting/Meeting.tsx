import React from 'react';
import { listen } from '@tauri-apps/api/event';
import { applyLocalEvent, emptyMeeting, markSaid, restoreMeeting, type LocalEvent } from './local';
import { invoke, isTauri } from '@tauri-apps/api/core';
import { getCurrentWindow, LogicalSize, PhysicalSize } from '@tauri-apps/api/window';
import { DEMO_END, mockAt, nextMoment } from './mock';
import './meeting.css';

export default function Meeting() {
  const [permissionBusy, setPermissionBusy] = React.useState(false);
  const [demo, setDemo] = React.useState(false);
  const [restored] = React.useState(() => {
    try { return { meeting: restoreMeeting(localStorage.getItem('meeting-session-v1')), error: null }; }
    catch { return { meeting: emptyMeeting, error: 'Saved meeting could not be read. Its original data has been kept.' }; }
  });
  const historyReadable = React.useRef(!restored.error);
  const [error, setError] = React.useState<string | null>(restored.error);
  const [local, setLocal] = React.useState(restored.meeting);
  const [busy, setBusy] = React.useState(false);
  const [confirmNew, setConfirmNew] = React.useState(false);
  const [notice, setNotice] = React.useState<string | null>(null);
  const actions = React.useRef<(payload: { action: string; value?: number; message?: string }) => void>(() => {});
  React.useEffect(() => {
    if (!historyReadable.current) { setError(restored.error); return; }
    try { localStorage.setItem('meeting-session-v1', JSON.stringify(local)); }
    catch { setError('Meeting history could not be saved. Keep this window open.'); }
  }, [local]);
  React.useEffect(() => {
    const subscription = listen<{ action: string; value?: number; message?: string }>('meeting-action', ({ payload }) => actions.current(payload));
    return () => { void subscription.then(unlisten => unlisten()); };
  }, []);
  const [status, setStatus] = React.useState('Stopped');
  const [transparency, setTransparency] = React.useState(() => {
    try { const saved = localStorage.getItem('meeting-transparency'); return saved === null ? 75 : Math.min(100, Math.max(0, Number(saved) || 0)); }
    catch { return 75; }
  });
  React.useEffect(() => { try { localStorage.setItem('meeting-transparency', String(transparency)); } catch { /* storage is optional */ } }, [transparency]);
  const active = status !== 'Stopped';
  const listening = status === 'Listening';
  const paused = status === 'Paused';
  const preparing = active && !listening && !paused;
  React.useEffect(() => {
    const subscription = listen<LocalEvent>('copilot', ({ payload }) => {
      setLocal(previous => applyLocalEvent(previous, payload));
      if (payload.type === 'status' && payload.status) setStatus(payload.status);
      if (payload.type === 'error' || (payload.type === 'say' && payload.state === 'ERROR')) setError(payload.message ?? 'Local processing failed.');
    });
    return () => { void subscription.then(unlisten => unlisten()); };
  }, []);
  const [seconds, setSeconds] = React.useState(12);
  const [playing, setPlaying] = React.useState(false);
  const [collapsed, setCollapsed] = React.useState(false);
  const [resizing, setResizing] = React.useState(false);
  const [menu, setMenu] = React.useState(false);
  const [following, setFollowing] = React.useState(true);
  const expandedSize = React.useRef<PhysicalSize | null>(null);
  const conversation = React.useRef<HTMLDivElement>(null);
  const menuRef = React.useRef<HTMLDivElement>(null);
  const menuButton = React.useRef<HTMLButtonElement>(null);
  const state = demo ? mockAt(seconds) : local;
  const turns = demo && state.say.options
    ? [...state.turns, { id: 'preview-reply', speaker: 'YOU' as const, text: state.say.text ?? '', status: 'FINAL' as const, options: state.say.options }]
    : state.turns;

  React.useEffect(() => {
    if (!playing) return;
    const timer = window.setInterval(() => setSeconds(value => Math.min(value + 1, DEMO_END)), 1000);
    return () => window.clearInterval(timer);
  }, [playing]);
  React.useEffect(() => { if (seconds === DEMO_END) setPlaying(false); }, [seconds]);
  React.useLayoutEffect(() => {
    if (following && conversation.current) conversation.current.scrollTop = conversation.current.scrollHeight;
  }, [state, collapsed, following]);
  React.useEffect(() => {
    if (!menu) return;
    menuRef.current?.querySelector('button')?.focus();
    const dismiss = (event: PointerEvent) => {
      if (!menuRef.current?.contains(event.target as Node) && !menuButton.current?.contains(event.target as Node)) setMenu(false);
    };
    document.addEventListener('pointerdown', dismiss);
    return () => document.removeEventListener('pointerdown', dismiss);
  }, [menu]);

  const native = async (action: () => Promise<unknown>) => {
    setError(null);
    try { await action(); } catch (e) { setError(String(e)); }
  };
  const toggleCollapsed = async () => {
    if (resizing) return;
    setResizing(true);
    setMenu(false);
    await native(async () => {
      if (!isTauri()) throw new Error('Open the Unmute desktop app to use window controls.');
      const win = getCurrentWindow();
      if (!collapsed) {
        expandedSize.current = await win.innerSize();
        await win.setMinSize(new LogicalSize(320, 40));
        try {
          const scale = await win.scaleFactor();
          await win.setSize(new PhysicalSize(expandedSize.current.width, Math.round(40 * scale)));
          await win.setResizable(false);
        } catch (e) {
          await win.setResizable(true);
          await win.setMinSize(new LogicalSize(320, 200));
          await win.setSize(expandedSize.current);
          throw e;
        }
      } else {
        await win.setResizable(true);
        await win.setMinSize(new LogicalSize(320, 200));
        await win.setSize(expandedSize.current ?? new LogicalSize(480, 320));
      }
      setCollapsed(!collapsed);
    });
    setResizing(false);
  };
  const toggleListening = () => native(async () => {
    if (permissionBusy || busy || preparing) return;
    setBusy(true);
    try {
      if (listening || paused) {
        await invoke('control_copilot', { action: listening ? 'pause' : 'resume' });
        return;
      }
      setDemo(false); setPlaying(false); setStatus('Preparing…');
      try { await invoke('start_copilot'); }
      catch (e) { setStatus('Stopped'); throw e; }
    } finally { setBusy(false); }
  });
  const suggest = () => native(() => invoke('control_copilot', { action: 'suggest' }));
  const audioPermissions = () => {
    if (permissionBusy) return;
    setPermissionBusy(true);
    void native(async () => {
      const message = await invoke<string>('setup_audio_access');
      setNotice(message);
    }).finally(() => setPermissionBusy(false));
  };
  const newMeeting = () => native(async () => {
    setBusy(true);
    try {
      await invoke('stop_capture');
      // Preserve unreadable storage too; never silently overwrite the original.
      if (!historyReadable.current) {
        localStorage.setItem(`meeting-recovery-${Date.now()}`, localStorage.getItem('meeting-session-v1') ?? '');
        historyReadable.current = true;
      }
      // Archive before clearing; the menu can restore any saved meeting.
      if (local.turns.length) localStorage.setItem(`meeting-archive-${Date.now()}`, JSON.stringify(local));
      setLocal(emptyMeeting); setStatus('Stopped'); setConfirmNew(false);
      setFollowing(true); setDemo(false); closeMenu();
    } finally { setBusy(false); }
  });
  actions.current = payload => {
    if (payload.action === 'transparency' && payload.value !== undefined) setTransparency(Math.min(100, Math.max(0, payload.value)));
    if (payload.action === 'settings') { if (collapsed) void toggleCollapsed().then(() => setMenu(true)); else setMenu(true); }
    if (payload.action === 'toggle') void toggleListening();
    if (payload.action === 'suggest') void suggest();
    if (payload.action === 'notice') setNotice(payload.message ?? null);
    if (payload.action === 'error') setError(payload.message ?? 'Please check audio permissions.');
  };
  const advance = () => { setPlaying(false); setSeconds(nextMoment(seconds)); setFollowing(true); };
  const closeMenu = () => { setMenu(false); menuButton.current?.focus(); };

  return <main className={`meeting ${collapsed ? 'is-collapsed' : ''} ${menu ? 'menu-open' : ''}`}
    style={{ '--surface-opacity': (100 - transparency) / 100 } as React.CSSProperties}>
    <header className="window-bar">
      <div className="drag-handle" title="Move window" onMouseDown={event => {
        if (event.button === 0 && isTauri()) void native(() => getCurrentWindow().startDragging());
      }}><span aria-hidden="true">⠿</span></div>
      <div className="window-controls">
        <button className="icon-button" disabled={permissionBusy || busy || preparing} aria-label={listening ? 'Pause listening' : paused ? 'Resume listening' : 'Start listening'} title={listening ? 'Pause listening' : paused ? 'Resume listening' : 'Start listening'} onClick={() => void toggleListening()}>{listening ? 'Ⅱ' : '▷'}</button>
        {!collapsed && <button ref={menuButton} className="icon-button" aria-label="Settings" title="Settings" aria-expanded={menu} onClick={() => setMenu(!menu)}>···</button>}
        <button className="icon-button" aria-label={collapsed ? 'Expand' : 'Collapse'} title={collapsed ? 'Expand' : 'Collapse'} disabled={resizing} onClick={() => void toggleCollapsed()}>{collapsed ? '▢' : '−'}</button>
        <button className="icon-button" aria-label="Hide window" title="Hide window" onClick={() => void native(() => getCurrentWindow().close())}>×</button>
      </div>
    </header>
    {menu && <div className="meeting-menu" ref={menuRef} onKeyDown={e => { if (e.key === 'Escape') closeMenu(); }}>
      <label className="transparency-control">Background transparency <output>{transparency}%</output>
        <input type="range" min="0" max="100" step="5" value={transparency} aria-label="Background transparency" onChange={e => setTransparency(Number(e.target.value))} />
      </label>
      <button onClick={() => setTransparency(100)}>Text only</button>
      <button disabled={permissionBusy} onClick={audioPermissions}>Audio permissions…</button>
      <button disabled={busy} onClick={() => setConfirmNew(!confirmNew)}>New meeting…</button>
      {confirmNew && <div><p className="error-detail">Archive this meeting and start a new one?</p><button disabled={busy} onClick={() => void newMeeting()}>Archive and start new</button><button onClick={() => setConfirmNew(false)}>Cancel</button></div>}
      <details><summary>Saved meetings</summary>
        {Object.keys(localStorage).filter(key => key.startsWith('meeting-archive-')).sort().reverse().map(key =>
          <button key={key} disabled={busy} onClick={() => void native(async () => {
            const saved = restoreMeeting(localStorage.getItem(key));
            await invoke('stop_capture');
            if (local.turns.length) localStorage.setItem(`meeting-archive-${Date.now()}`, JSON.stringify(local));
            setLocal(saved); setStatus('Stopped'); setDemo(false); closeMenu();
          })}>{new Date(Number(key.replace('meeting-archive-', ''))).toLocaleString('en-US')}</button>)}
      </details>
      {error && <p className="error-detail">{error}</p>}
      <details><summary>More</summary>
        {!active && <button onClick={() => { setDemo(!demo); setPlaying(false); closeMenu(); }}>{demo ? 'Exit preview' : 'Preview'}</button>}
        {demo && <>
          <button onClick={() => { setPlaying(!playing); if (seconds === DEMO_END) setSeconds(0); closeMenu(); }}>{playing ? 'Pause preview' : 'Play preview'}</button>
          <button onClick={() => { advance(); closeMenu(); }}>Next example</button>
        </>}
        <button onClick={() => { closeMenu(); void native(() => invoke('open_diagnostics')); }}>Open diagnostics ↗</button>
      </details>
    </div>}
    <div className="expanded-content" hidden={collapsed}>
      {error && <div className="window-error" role="alert">
        <button onClick={() => setMenu(true)}>{error.includes('PERMISSION_REQUIRED') ? 'Please allow audio access.' : 'Please check settings.'}</button>
        <button aria-label="Dismiss alert" onClick={() => setError(null)}>×</button>
      </div>}
      {notice && <div className="window-error" role="status"><span>{notice}</span><button aria-label="Dismiss notice" onClick={() => setNotice(null)}>×</button></div>}
      <section className="conversation-region" aria-label="Meeting conversation">
        <div className="conversation-scroll" ref={conversation} tabIndex={0} aria-label="Session conversation" onScroll={event => {
          const el = event.currentTarget;
          setFollowing(el.scrollHeight - el.scrollTop - el.clientHeight < 24);
        }}>
          {!demo && turns.length === 0 && active && status !== 'Listening' && <p className="preparing" role="status">{status.includes('Downloading') ? 'Downloading models for first use…' : 'Preparing…'}</p>}
          {turns.map(turn => <article key={turn.id} className={`message ${turn.speaker === 'REMOTE' ? 'remote' : 'you'}`} aria-label={turn.speaker === 'REMOTE' ? 'Remote' : 'Response suggestions'}>
            {turn.options ? <>
              <ol className="response-options">{turn.options.map(option => <li key={option.kind}>
                <p className="suggestion">{option.text}</p>
                <button className="said-button" disabled={demo} aria-pressed={turn.saidKind === option.kind} title="Mark this suggestion as something you said" onClick={() => setLocal(previous => markSaid(previous, turn.id, option.kind))}>{turn.saidKind === option.kind ? '✓ Said' : 'Mark as said'}</button>
              </li>)}</ol>
            </> : <p className={`turn ${turn.status.toLowerCase()}`} aria-label={turn.status === 'PARTIAL' ? 'Partial transcript' : undefined}>{turn.text}</p>}
          </article>)}

        </div>
        {!following && <button className="latest-button" aria-label="Latest conversation" title="Latest conversation" onClick={() => setFollowing(true)}>↓</button>}
      </section>
      <footer className="meeting-actions">
        <button disabled={demo || busy || preparing} onClick={() => void toggleListening()}>{listening ? 'Pause listening' : paused ? 'Resume listening' : 'Start listening'}</button>
        <button disabled={demo || !(listening || paused) || !turns.some(t => t.speaker === 'REMOTE')} onClick={() => void suggest()}>{state.say.state === 'THINKING' ? 'Suggesting…' : 'Suggest now'}</button>
      </footer>
    </div>
  </main>;
}
