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

installElastic();
initAppIcon();
const root=ReactDOM.createRoot(document.getElementById('root'));
async function boot() {
try {
await initializePrivacy(config.server);
await kv.migratePrivacy();
await syncNativeLanguage();
root.render(
  <React.StrictMode>
    <ConfirmProvider><App /></ConfirmProvider>
  </React.StrictMode>,
);
} catch(error) {
  root.render(<div className="flex h-full items-center justify-center bg-bgDark p-6 text-center"><p>{tr("Não foi possível abrir o armazenamento protegido. Preserve os dados e abra novamente o servidor. ")}{error.message}</p></div>);
}
}
boot();
