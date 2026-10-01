import React from 'react';
import { listen } from '@tauri-apps/api/event';
import { applyLocalEvent, emptyMeeting, type LocalEvent } from './local';
import { invoke, isTauri } from '@tauri-apps/api/core';
import { getCurrentWindow, LogicalSize, PhysicalSize } from '@tauri-apps/api/window';
import { DEMO_END, mockAt, nextMoment } from './mock';
import './meeting.css';

export default function Meeting() {
  const [permissionBusy, setPermissionBusy] = React.useState(false);
  const [demo, setDemo] = React.useState(false);
  const [local, setLocal] = React.useState(emptyMeeting);
  const [status, setStatus] = React.useState('Stopped');
  const [transparency, setTransparency] = React.useState(() => {
    try { const saved = localStorage.getItem('meeting-transparency'); return saved === null ? 75 : Math.min(100, Math.max(0, Number(saved) || 0)); }
    catch { return 75; }
  });
  React.useEffect(() => { try { localStorage.setItem('meeting-transparency', String(transparency)); } catch { /* storage is optional */ } }, [transparency]);
  const active = status !== 'Stopped';
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
  const [error, setError] = React.useState<string | null>(null);
  const [following, setFollowing] = React.useState(true);
  const expandedSize = React.useRef<PhysicalSize | null>(null);
  const conversation = React.useRef<HTMLDivElement>(null);
  const menuRef = React.useRef<HTMLDivElement>(null);
  const menuButton = React.useRef<HTMLButtonElement>(null);
  const state = demo ? mockAt(seconds) : local;
  const turns = state.turns.filter(turn => turn.speaker === 'REMOTE');

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
    if (permissionBusy) return;
    if (active) { await invoke('stop_capture'); setStatus('Stopped'); return; }
    setDemo(false); setPlaying(false); setLocal(emptyMeeting); setFollowing(true); setStatus('Preparing…');
    try { await invoke('start_copilot'); }
    catch (e) { setStatus('Stopped'); throw e; }
  });
  const advance = () => { setPlaying(false); setSeconds(nextMoment(seconds)); setFollowing(true); };
  const closeMenu = () => { setMenu(false); menuButton.current?.focus(); };

  return <main className={`meeting ${collapsed ? 'is-collapsed' : ''} ${menu ? 'menu-open' : ''}`}
    style={{ '--surface-opacity': (100 - transparency) / 100 } as React.CSSProperties}>
    <header className="window-bar">
      <div className="drag-handle" title="Move window" onMouseDown={event => {
        if (event.button === 0 && isTauri()) void native(() => getCurrentWindow().startDragging());
      }}><span aria-hidden="true">⠿</span></div>
      <div className="window-controls">
        <button className="icon-button" disabled={permissionBusy} aria-label={active ? 'Stop listening' : 'Start listening'} title={active ? 'Stop listening' : 'Start listening'} onClick={() => void toggleListening()}>{active ? 'Ⅱ' : '▷'}</button>
        {!collapsed && <button ref={menuButton} className="icon-button" aria-label="Settings" title="Settings" aria-expanded={menu} onClick={() => setMenu(!menu)}>···</button>}
        <button className="icon-button" aria-label={collapsed ? 'Expand' : 'Collapse'} title={collapsed ? 'Expand' : 'Collapse'} disabled={resizing} onClick={() => void toggleCollapsed()}>{collapsed ? '▢' : '−'}</button>
        <button className="icon-button" aria-label="Close window" title="Close window" onClick={() => void native(() => getCurrentWindow().close())}>×</button>
      </div>
    </header>
    {menu && <div className="meeting-menu" ref={menuRef} onKeyDown={e => { if (e.key === 'Escape') closeMenu(); }}>
      <label className="transparency-control">Background transparency <output>{transparency}%</output>
        <input type="range" min="0" max="100" step="5" value={transparency} aria-label="Background transparency" onChange={e => setTransparency(Number(e.target.value))} />
      </label>
      <button onClick={() => setTransparency(100)}>Text only</button>
      {!active && <button disabled={permissionBusy} onClick={() => {
        setPermissionBusy(true); void native(() => invoke('request_audio_permissions')).finally(() => setPermissionBusy(false));
      }}>Audio permissions…</button>}
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
      <section className="conversation-region" aria-label="Remote conversation">
        <div className="conversation-scroll" ref={conversation} tabIndex={0} aria-label="Session conversation" onScroll={event => {
          const el = event.currentTarget;
          setFollowing(el.scrollHeight - el.scrollTop - el.clientHeight < 24);
        }}>
          {!demo && turns.length === 0 && !active && <button className="start-button" disabled={permissionBusy} onClick={() => void toggleListening()}>Start listening</button>}
          {!demo && turns.length === 0 && active && status !== 'Listening' && <p className="preparing" role="status">{status.includes('Downloading') ? 'Downloading models for first use…' : 'Preparing…'}</p>}
          {turns.length > 0 && <p className="transcript">{turns.map(turn => <span key={turn.id} className={`turn ${turn.status.toLowerCase()}`} aria-label={turn.status === 'PARTIAL' ? 'Partial transcript' : undefined}>{turn.text}{' '}</span>)}</p>}
        </div>
        {!following && <button className="latest-button" aria-label="Latest conversation" title="Latest conversation" onClick={() => setFollowing(true)}>↓</button>}
      </section>
      <section className="response-region" tabIndex={0} aria-label="Suggested responses" aria-live="polite" aria-atomic="true">
        {state.say.state === 'READY' && (state.say.options?.length ? <ol className="response-options">
          {state.say.options.map(option => <li key={option.kind} title={option.kind.replace('_', ' ')} aria-label={`${option.kind.replace('_', ' ')}: ${option.text}`}><p className="suggestion">{option.text}</p></li>)}
        </ol> : <p className="suggestion">{state.say.text}</p>)}
      </section>
    </div>
  </main>;
}
