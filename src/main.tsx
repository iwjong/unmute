import { createRoot } from 'react-dom/client';

// Each desktop window loads only its own UI and stylesheet.
const view = new URLSearchParams(location.search).get('view');
document.title = view === 'diagnostics' ? 'Unmute · Audio diagnostics' : 'Unmute · Meeting Copilot';
void (view === 'diagnostics'
  ? import('./ui/diagnostics/Diagnostics')
  : import('./ui/meeting/Meeting')).then(({ default: App }) => {
  createRoot(document.getElementById('root')!).render(<App />);
});
