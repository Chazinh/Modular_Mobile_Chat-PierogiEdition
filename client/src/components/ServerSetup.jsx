import React, { useState } from 'react';
import { Link2, Loader2 } from 'lucide-react';
import { api, config, isNative } from '../api.js';

// PC address (APK) + pairing with a temporary code. Provider API keys never reach the device.
export default function ServerSetup({ needsServer, onDone }) {
  const [server, setServer] = useState(config.server || 'http://');
  const [code, setCode] = useState('');
  const [name, setName] = useState(() => (isNative() ? 'Celular Android' : 'Navegador'));
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState('');

  async function connect(e) {
    e.preventDefault();
    setError('');
    setBusy(true);
    const prev = config.server;
    try {
      if (needsServer) {
        const url = server.trim().replace(/\/+$/, '');
        if (!/^https?:\/\/[^/\s]+/i.test(url)) throw new Error('Informe o endereço completo, ex.: http://192.168.0.10:3000');
        config.server = url;
        await api.health();
      }
      await api.pair(code.trim(), name.trim() || 'Aparelho');
      onDone();
    } catch (err) {
      if (needsServer) config.server = prev;
      setError(err.status === 0 ? 'Não consegui falar com o PC. Confira o endereço, o Wi-Fi e o firewall.' : err.message);
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
            <h1 className="text-lg font-bold">Conectar ao PC</h1>
            <p className="text-sm text-textMuted">O PC guarda as conversas e a memória.</p>
          </div>
        </div>

        {needsServer && (
          <label className="block text-sm">
            <span className="text-textMuted">Endereço do PC na rede local</span>
            <input
              value={server}
              onChange={(e) => setServer(e.target.value)}
              inputMode="url"
              autoCapitalize="none"
              autoCorrect="off"
              className="mt-1 w-full rounded-xl bg-composer px-4 py-3 text-[16px] outline-none"
              placeholder="http://192.168.0.10:3000"
            />
          </label>
        )}

        <label className="block text-sm">
          <span className="text-textMuted">Código de pareamento (6 dígitos)</span>
          <input
            value={code}
            onChange={(e) => setCode(e.target.value.replace(/\D/g, '').slice(0, 6))}
            inputMode="numeric"
            className="mt-1 w-full rounded-xl bg-composer px-4 py-3 text-center text-[22px] tracking-[0.4em] outline-none"
            placeholder="000000"
          />
        </label>

        <label className="block text-sm">
          <span className="text-textMuted">Nome deste aparelho</span>
          <input value={name} onChange={(e) => setName(e.target.value)} className="mt-1 w-full rounded-xl bg-composer px-4 py-3 text-[16px] outline-none" />
        </label>

        {error && <p className="rounded-xl bg-red-500/10 px-3 py-2 text-sm text-red-300">{error}</p>}

        <button
          type="submit"
          disabled={busy || code.length !== 6}
          className="flex w-full items-center justify-center gap-2 rounded-full bg-accentPurple py-3.5 font-semibold text-white disabled:opacity-40"
        >
          {busy && <Loader2 size={18} className="animate-spin" />} Conectar
        </button>
        <p className="text-xs leading-relaxed text-textMuted">
          O código aparece na janela do servidor ao iniciar o <b>start-server.bat</b>, ou no PC em <b>⋯ → Configurações → Gerar código</b>. Ele vale por 5 minutos e só pode ser usado uma vez.
        </p>
      </form>
    </div>
  );
}
