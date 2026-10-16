import { tr, useLanguage } from '../i18n.js';
import React, { useEffect, useRef, useState } from 'react';
import { Link2, Loader2, CheckCircle2, AlertCircle } from 'lucide-react';
import { api, config, isNative } from '../api.js';
import { normalizePcServer, testPcConnection } from '../server-connection.js';
import { synchronizePairedPc } from '../pc-pairing.js';

// Optional PC connection. Editing/canceling never clears the current endpoints or credential.
export default function ServerSetup({ needsServer, onDone, onCancel }) {
  useLanguage();
  const [server, setServer] = useState(config.connection.local || '');
  const [remote, setRemote] = useState(config.connection.remote || '');
  const [code, setCode] = useState('');
  const [name, setName] = useState(() => (isNative() ? tr('Celular Android') : tr('Navegador')));
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState('');
  const [probing, setProbing] = useState(false);
  const [probeStatus, setProbeStatus] = useState(null);
  const [pairedPc, setPairedPc] = useState('');
  const [waiting, setWaiting] = useState(false);
  const action = useRef(null);
  useEffect(() => () => action.current?.abort(), []);

  async function testConnection() {
    setProbeStatus(null);
    setError('');
    const target = server.trim() || remote.trim();
    if (!target) {
      setProbeStatus({ ok: false, message: tr('Informe o endereço do PC primeiro.') });
      return;
    }
    setProbing(true);
    try {
      const res = await testPcConnection(target, { httpsOnly: !server.trim() });
      setProbeStatus({
        ok: true,
        message: tr('Conexão com o PC confirmada em {0} ms.', [res.latencyMs])
      });
    } catch (err) {
      setProbeStatus({
        ok: false,
        message: err.name === 'TypeError' || err.name === 'TimeoutError'
          ? tr('Não foi possível acessar {0}. Copie o endereço completo exibido em “NO CELULAR” no PC; confira todos os números do IP e a porta.', [target])
          : tr(err.message)
      });
    } finally {
      setProbing(false);
    }
  }

  function appendPort() {
    try {
      const url = new URL(normalizePcServer(server, false));
      url.port = '3000';
      setServer(url.origin);
      setError('');
    } catch (err) { setError(err.message); }
    setProbeStatus(null);
  }

  async function connect(e) {
    e.preventDefault();
    if (action.current) return;
    const controller = action.current = new AbortController();
    setError('');
    setBusy(true);
    setWaiting(false);
    let rollback;
    try {
      if (isNative()) {
        if (!server.trim() && !remote.trim()) throw new Error('Informe pelo menos um endereço do PC.');
        rollback = config.configureConnection({ local: server, remote });
        await api.health({ pc: config.connection.local || config.connection.remote });
      }
      const pc = isNative() ? config.connection.local || config.connection.remote : '';
      if (!pc || pairedPc !== pc) {
        await api.pair(code.trim(), name.trim() || tr('Aparelho'), pc);
        if (pc) setPairedPc(pc);
      }
      // Pairing has committed the credential. Keep its matching endpoint if sync needs another attempt.
      rollback = undefined;
      if (isNative()) await synchronizePairedPc({ connect: signal => api.switchToPc(signal), signal: controller.signal,
        onWaiting: () => { if (!controller.signal.aborted) setWaiting(true); } });
      if (controller.signal.aborted) return;
      onDone();
    } catch (err) {
      rollback?.();
      if (controller.signal.aborted) return;
      setError(err.status === 0 ? 'Não consegui falar com o PC. Confira os endereços, a porta e o firewall.' : err.message);
    } finally {
      if (action.current === controller) { action.current = null; if (!controller.signal.aborted) { setBusy(false); setWaiting(false); } }
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
              <span className="text-textMuted">{tr('Endereço do PC na rede local (Wi-Fi)')}</span>
              <div className="mt-1 flex gap-2">
                <input
                  aria-label={tr('Endereço do PC na rede local (Wi-Fi)')}
                  value={server}
                  disabled={busy}
                  onChange={(e) => { setServer(e.target.value); setProbeStatus(null); }}
                  inputMode="url"
                  autoCapitalize="none"
                  autoCorrect="off"
                  className="w-full rounded-xl bg-composer px-4 py-3 text-[16px] outline-none"
                  placeholder="http://192.168.1.100:3000"
                />
                <button
                  type="button"
                  disabled={probing || busy || (!server.trim() && !remote.trim())}
                  onClick={testConnection}
                  className="flex shrink-0 items-center gap-1 rounded-xl bg-composer px-3 py-2 text-xs font-medium text-accentPurpleLight disabled:opacity-40"
                >
                  {probing ? <Loader2 size={14} className="animate-spin" /> : tr('Testar')}
                </button>
              </div>
            </label>

            {server.trim() && !server.includes(':3000') && (
              <button
                type="button"
                onClick={appendPort}
                className="text-[11px] text-accentPurpleLight underline"
              >
                {tr('Adicionar porta padrão :3000')}
              </button>
            )}

            {probeStatus && (
              <div role="status" className={`flex items-start gap-2 rounded-xl px-3 py-2 text-xs leading-relaxed ${probeStatus.ok ? 'bg-green-500/10 text-green-300' : 'bg-yellow-500/10 text-yellow-300'}`}>
                {probeStatus.ok ? <CheckCircle2 size={16} className="shrink-0 mt-0.5 text-green-400" /> : <AlertCircle size={16} className="shrink-0 mt-0.5 text-yellow-400" />}
                <span>{probeStatus.message}</span>
              </div>
            )}

            <label className="block text-sm">
              <span className="text-textMuted">{tr('Endereço pela internet (opcional)')}</span>
              <input disabled={busy} aria-label={tr('Endereço pela internet (opcional)')} value={remote} onChange={(e) => { setRemote(e.target.value); setProbeStatus(null); }} inputMode="url" autoCapitalize="none" autoCorrect="off"
                className="mt-1 w-full rounded-xl bg-composer px-4 py-3 text-[16px] outline-none" placeholder="https://chat.example.com" />
            </label>
            <p className="text-xs leading-relaxed text-textMuted">{tr('Conecte o celular e o PC à mesma rede Wi-Fi. Use o endereço exibido pelo servidor no PC, incluindo a porta (ex: http://192.168.1.100:3000).')}</p>
            <p className="text-xs leading-relaxed text-textMuted">{tr('No celular, 127.0.0.1 e localhost apontam para o próprio aparelho. Copie o endereço “NO CELULAR” exibido pelo servidor no PC.')}</p>
          </div>
        )}

        <label className="block text-sm">
          <span className="text-textMuted">{tr("Código de pareamento (6 dígitos)")}</span>
          <input
            aria-label={tr('Código de pareamento (6 dígitos)')}
            value={code}
            disabled={busy}
            onChange={(e) => setCode(e.target.value.replace(/\D/g, '').slice(0, 6))}
            inputMode="numeric"
            className="mt-1 w-full rounded-xl bg-composer px-4 py-3 text-center text-[22px] tracking-[0.4em] outline-none"
            placeholder="000000"
          />
        </label>

        <label className="block text-sm">
          <span className="text-textMuted">{tr("Nome deste aparelho")}</span>
          <input disabled={busy} value={name} onChange={(e) => setName(e.target.value)} className="mt-1 w-full rounded-xl bg-composer px-4 py-3 text-[16px] outline-none" />
        </label>

        {error && <p role="alert" className="rounded-xl bg-red-500/10 px-3 py-2 text-sm text-red-300">{tr(error)}</p>}
        {waiting && <p role="status" className="rounded-xl bg-accentPurple/10 px-3 py-2 text-sm text-accentPurpleLight">{tr('Pareamento salvo. Aguardando o trabalho ativo terminar; a sincronização será tentada a cada 10 segundos, por até 5 minutos. Não é preciso gerar outro código.')}</p>}

        <button
          type="submit"
          disabled={busy || code.length !== 6}
          className="flex w-full items-center justify-center gap-2 rounded-full bg-accentPurple py-3.5 font-semibold text-white disabled:opacity-40"
        >
          {busy && <Loader2 size={18} className="animate-spin" />}{tr(" Conectar\n        ")}</button>
        {(onCancel || config.embeddedReady) && <button type="button" disabled={busy && !waiting} onClick={() => { action.current?.abort(); (onCancel || onDone)(); }} className="w-full py-2 text-sm text-textMuted">{tr(config.embeddedReady ? 'Continuar no aparelho' : 'Cancelar')}</button>}
        <p className="text-xs leading-relaxed text-textMuted">{tr("\n          O código aparece na janela do servidor ao iniciar o ")}<b>start-server.ps1</b>{tr(", ou no PC em ")}<b>{tr("⋯ → Configurações → Gerar código")}</b>{tr(". Ele vale por 5 minutos e só pode ser usado uma vez.\n        ")}</p>
      </form>
    </div>
  );
}
