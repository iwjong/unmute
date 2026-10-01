import React from 'react';
import { invoke, isTauri } from '@tauri-apps/api/core';
import './diagnostics.css';

type Source = { rms?: number; peak?: number; dropped_frames?: number; discontinuities?: number; drift_ns?: number };
type Diagnostics = {
  platform: string; error?: string; session_active?: boolean;
  preflight?: { microphone: string; system_audio: string; input_name: string; route: { name: string; kind: string } };
  snapshot?: { capture_state?: string; error?: string; warning?: string; elapsed_seconds?: number;
    system_audio_permission?: string; user?: Source; remote?: Source;
    correlation?: { maximum: number; lag_ms: number }; relative_drift_ns?: number };
};
const metric = (value: number | undefined | null, digits = 3) => value == null ? 'Unavailable' : value.toFixed(digits);
export default function Diagnostics() {
  const [d, setD] = React.useState<Diagnostics | null>(null);
  const [error, setError] = React.useState<string | null>(null);
  const [confirmed, setConfirmed] = React.useState(false);
  const [record, setRecord] = React.useState(false);
  const [busy, setBusy] = React.useState(false);
  const [path, setPath] = React.useState<string | null>(null);
  React.useEffect(() => {
    if (!isTauri()) return;
    let cancelled = false;
    const poll = async () => {
      try { const value = await invoke<Diagnostics>('diagnostics'); if (!cancelled) setD(value); }
      catch (e) { if (!cancelled) setError(String(e)); }
    };
    void poll(); const id = setInterval(poll, 1000);
    return () => { cancelled = true; clearInterval(id); };
  }, []);
  const action = async (name: string) => {
    setBusy(true); setError(null);
    try { const value = await invoke<string>(name, { systemAudioConfirmed: confirmed, record }); if (name === 'start_capture') setPath(value); }
    catch (e) { setError(String(e)); } finally { setBusy(false); }
  };
  const snapshot = d?.snapshot;
  const active = d?.session_active ?? false;
  return <main>
    <header><span className="brand">unmute</span><span>M0 · Audio diagnostics</span></header>
    <h1>Know what you’re hearing.</h1>
    <p className="intro">Local microphone and system output. Speakers are supported; acoustic bleed is measured, not removed.</p>
    <section className="notice" aria-live="polite">
      <strong>{snapshot?.capture_state ?? (isTauri() ? 'STOPPED' : 'BROWSER PREVIEW')}</strong>
      <p>{error ?? snapshot?.error ?? d?.error ?? snapshot?.warning ?? 'No source-isolation acceptance result is inferred from meters or correlation.'}</p>
      {!isTauri() && <p>Open the desktop application for capture. This preview has no audio backend.</p>}
    </section>
    <section className="controls">
      <h2>Permissions and recording</h2>
      <p>Microphone: {d?.preflight?.microphone ?? 'Unknown'} · System audio: {snapshot?.system_audio_permission ?? d?.preflight?.system_audio ?? 'Unknown'}</p>
      <p>Confirm System Audio Recording for this app in macOS Privacy &amp; Security. The tap API does not expose a public permission-state query.</p>
      <label><input type="checkbox" checked={confirmed} onChange={e => setConfirmed(e.target.checked)} disabled={active} /> I have granted System Audio Recording to this application.</label>
      <label><input type="checkbox" checked={record} onChange={e => setRecord(e.target.checked)} disabled={active} /> Save separate USER and REMOTE recordings</label>
      <div className="buttons">
        <button disabled={!isTauri() || busy || active} onClick={() => void action('request_audio_permissions')}>Request permissions</button>
        <button disabled={!isTauri() || busy || active || !confirmed || d?.preflight?.microphone !== 'GRANTED'} onClick={() => void action('start_capture')}>Start capture</button>
        <button disabled={!isTauri() || busy || !active} onClick={() => void action('stop_capture')}>Stop capture</button>
      </div>
      {path && <p className="path">Diagnostics: {path}</p>}
    </section>
    <div className="sources">
      {(['USER', 'REMOTE'] as const).map(source => {
        const stats = source === 'USER' ? snapshot?.user : snapshot?.remote;
        return <section key={source}><span className="label">{source}</span><h2>{source === 'USER' ? 'Local microphone' : 'System output'}</h2>
          <p>{source === 'USER' ? d?.preflight?.input_name ?? 'Not queried' : d?.preflight?.route.name ?? 'Not queried'}</p>
          <dl><dt>RMS / peak</dt><dd>{metric(stats?.rms)} / {metric(stats?.peak)}</dd>
            <dt>Dropped frames</dt><dd>{stats?.dropped_frames ?? 'Unavailable'}</dd>
            <dt>Discontinuities</dt><dd>{stats?.discontinuities ?? 'Unavailable'}</dd>
            <dt>Drift (ms)</dt><dd>{metric(stats?.drift_ns == null ? null : stats.drift_ns / 1e6)}</dd></dl>
        </section>;
      })}
    </div>
    <section><h2>Source overlap · observational</h2>
      <p>Correlation: {metric(snapshot?.correlation?.maximum)} · lag: {metric(snapshot?.correlation?.lag_ms, 1)} ms (0–300 ms search)</p>
      <p>With speakers, USER can include the remote participant through the room. Listen to separate recordings and inspect timelines before interpreting source quality.</p>
      <p>Real meeting, microphone quality, physical reconnect, and signed-app permissions: PENDING_HUMAN_VALIDATION.</p>
    </section>
    <footer>Normalized audio: 16 kHz · mono · signed PCM16 little-endian · Audio remains outside the WebView.</footer>
  </main>;
}

