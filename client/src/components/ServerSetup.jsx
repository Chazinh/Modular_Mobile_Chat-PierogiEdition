import { tr, useLanguage } from '../i18n.js';
import React, { useState } from 'react';
import { Link2, Loader2 } from 'lucide-react';
import { api, config, isNative } from '../api.js';

// PC address (APK) + pairing with a temporary code. Provider API keys never reach the device.
export default function ServerSetup({ needsServer, onDone }) {
  useLanguage();
  const [server, setServer] = useState(config.connection.local || '');
  const [remote, setRemote] = useState(config.connection.remote || '');
  const [code, setCode] = useState('');
  const [name, setName] = useState(() => (isNative() ? tr('Celular Android') : tr('Navegador')));
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState('');

  async function connect(e) {
    e.preventDefault();
    setError('');
    setBusy(true);
    let rollback;
    try {
      if (isNative()) {
        if (!server.trim() && !remote.trim()) throw new Error('Informe pelo menos um endereço HTTPS do PC.');
        rollback = config.configureConnection({ local: server, remote });
        await api.health({ pc: server.trim() || remote.trim() });
      }
      await api.pair(code.trim(), name.trim() || tr('Aparelho'), isNative() ? config.connection.local || config.connection.remote : '');
      onDone();
    } catch (err) {
      rollback?.();
      setError(err.status === 0 ? 'Não consegui falar com o PC. Confira os endereços, o certificado e o firewall.' : err.message);
    } finally {
      setBusy(false);
    }
  }

  return (
    <div className="flex h-full items-center justify-center overflow-y-auto px-5" style={{ paddingTop: 'var(--sat)' }}>
      <form onSubmit={connect} className="w-full max-w-sm space-y-4 rounded-3xl bg-cardDark p-6">
        <div className="flex items-center gap-3">
          <div className="rounded-2xl bg-accentPurple/25 p-3 text-accentPurpleLight">
            <Link2 size={24} />
          </div>
          <div>
            <h1 className="text-lg font-bold">{tr("Conectar ao PC")}</h1>
            <p className="text-sm text-textMuted">{tr(config.embeddedReady ? 'O app também funciona sozinho. Pareie para sincronizar com o PC.' : 'O PC guarda as conversas e a memória.')}</p>
          </div>
        </div>

        {isNative() && (
          <div className="space-y-3">
          <label className="block text-sm">
            <span className="text-textMuted">{tr('Endereço HTTPS na rede local')}</span>
            <input
              value={server}
              onChange={(e) => setServer(e.target.value)}
              inputMode="url"
              autoCapitalize="none"
              autoCorrect="off"
              className="mt-1 w-full rounded-xl bg-composer px-4 py-3 text-[16px] outline-none"
              placeholder="https://192.168.0.10:8443"
            />
          </label>
          <label className="block text-sm">
            <span className="text-textMuted">{tr('Endereço HTTPS pela internet (opcional)')}</span>
            <input value={remote} onChange={(e) => setRemote(e.target.value)} inputMode="url" autoCapitalize="none" autoCorrect="off"
              className="mt-1 w-full rounded-xl bg-composer px-4 py-3 text-[16px] outline-none" placeholder="https://chat.example.com" />
          </label>
          <p className="text-xs leading-relaxed text-textMuted">{tr('Os dois endereços devem levar ao mesmo PC. O app prefere a rede local e usa a internet quando necessário. Faça o primeiro pareamento pela rede local.')}</p>
          </div>
        )}

        <label className="block text-sm">
          <span className="text-textMuted">{tr("Código de pareamento (6 dígitos)")}</span>
          <input
            value={code}
            onChange={(e) => setCode(e.target.value.replace(/\D/g, '').slice(0, 6))}
            inputMode="numeric"
            className="mt-1 w-full rounded-xl bg-composer px-4 py-3 text-center text-[22px] tracking-[0.4em] outline-none"
            placeholder="000000"
          />
        </label>

        <label className="block text-sm">
          <span className="text-textMuted">{tr("Nome deste aparelho")}</span>
          <input value={name} onChange={(e) => setName(e.target.value)} className="mt-1 w-full rounded-xl bg-composer px-4 py-3 text-[16px] outline-none" />
        </label>

        {error && <p className="rounded-xl bg-red-500/10 px-3 py-2 text-sm text-red-300">{tr(error)}</p>}

        <button
          type="submit"
          disabled={busy || code.length !== 6}
          className="flex w-full items-center justify-center gap-2 rounded-full bg-accentPurple py-3.5 font-semibold text-white disabled:opacity-40"
        >
          {busy && <Loader2 size={18} className="animate-spin" />}{tr(" Conectar\n        ")}</button>
        {config.embeddedReady && <button type="button" disabled={busy} onClick={onDone} className="w-full py-2 text-sm text-textMuted">{tr('Continuar no aparelho')}</button>}
        <p className="text-xs leading-relaxed text-textMuted">{tr("\n          O código aparece na janela do servidor ao iniciar o ")}<b>start-server.bat</b>{tr(", ou no PC em ")}<b>{tr("⋯ → Configurações → Gerar código")}</b>{tr(". Ele vale por 5 minutos e só pode ser usado uma vez.\n        ")}</p>
      </form>
    </div>
  );
}
