import React from 'react';
import { createRoot } from 'react-dom/client';
import { invoke, isTauri } from '@tauri-apps/api/core';
import './style.css';

type Diagnostics = {
  platform: string;
  captureState: string;
  microphonePermission: string;
  systemAudioPermission: string;
  remoteEndpointName: string | null;
  message: string;
};

function App() {
  const [diagnostics, setDiagnostics] = React.useState<Diagnostics | null>(null);
  const [error, setError] = React.useState<string | null>(null);

  React.useEffect(() => {
    if (!isTauri()) return;
    invoke<Diagnostics>('diagnostics').then(setDiagnostics).catch(e => setError(String(e)));
  }, []);

  return <main>
    <header><span className="brand">unmute</span><span>M0 · Audio diagnostics</span></header>
    <h1>Know what you’re hearing.</h1>
    <p className="intro">Separate local microphone and system output. Headphones are required.</p>
    <section className="notice" aria-live="polite">
      <strong>{error ? 'Diagnostics unavailable' : diagnostics?.captureState ?? 'NOT CONNECTED'}</strong>
      <p>{error ?? diagnostics?.message ?? (isTauri()
        ? 'Reading backend status…'
        : 'Browser preview. Open the desktop app to read backend diagnostics.')}</p>
    </section>
    <div className="sources">
      <section><span className="label">USER</span><h2>Local microphone</h2>
        <dl><dt>Capture</dt><dd>Not implemented</dd>
          <dt>Permission</dt><dd>{diagnostics?.microphonePermission ?? 'Unknown'}</dd></dl>
      </section>
      <section><span className="label">REMOTE</span><h2>System output</h2>
        <dl><dt>Endpoint</dt><dd>{diagnostics?.remoteEndpointName ?? 'Not selected'}</dd>
          <dt>Permission</dt><dd>{diagnostics?.systemAudioPermission ?? 'Unknown'}</dd></dl>
      </section>
    </div>
    <section><h2>Acceptance status</h2>
      <p>No audio has been captured or validated. Meters, drift, and bleed measurements are unavailable.</p>
      <p>Real meeting calls, microphone quality, headset changes, and signed-app permission tests remain pending.</p>
    </section>
    <footer>Normalized audio: 16 kHz · mono · signed PCM16 little-endian</footer>
  </main>;
}

createRoot(document.getElementById('root')!).render(<App />);
