import { tr } from './i18n.js';
import { syncNativeLanguage } from './i18n.js';
import { ConfirmProvider } from './components/ConfirmProvider.jsx';
import React from 'react';
import ReactDOM from 'react-dom/client';
import App from './App.jsx';
import './index.css';
import './glass.css';
import './eyecare.js';
import { installElastic } from './elastic.js';
import { initializePrivacy } from './private-storage.js';
import { kv } from './kv.js';
import { config } from './api.js';
import { initAppIcon } from './app-icon.js';
import { initAnimationCache } from './animation-cache.js';
import { localHost } from './local-host.js';

installElastic();
initAppIcon();
initAnimationCache();
const root=ReactDOM.createRoot(document.getElementById('root'));
async function boot() {
let stage = 'privacy';
try {
await initAnimationCache();
await initializePrivacy(config.server);
stage = 'local-host';
await localHost.ensureStarted();
stage = 'cache';
await kv.migratePrivacy();
await syncNativeLanguage();
root.render(
  <React.StrictMode>
    <ConfirmProvider><App /></ConfirmProvider>
  </React.StrictMode>,
);
} catch(error) {
  const message = stage === 'local-host'
    ? 'O host Android não iniciou. Os dados foram preservados. '
    : 'Não foi possível abrir o armazenamento protegido. Preserve os dados e tente novamente. ';
  root.render(<div className="flex h-full items-center justify-center bg-bgDark p-6 text-center"><p>{tr(message)}{error.code ? `${error.code}: ` : ''}{error.message}</p></div>);
}
}
boot();
