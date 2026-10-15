import React, { useEffect, useRef, useState } from 'react';
import { api } from '../api.js';
import { tr, useLanguage } from '../i18n.js';
import { useLayer } from '../nav.js';
import { WifiOff, Wifi, Smartphone, Clock, CheckCircle, Loader2 } from 'lucide-react';
import '../drawer.css';

export default function ConnectionLossModal({ isOpen, onClose, onSwitchToLocal, onReconnected }) {
  useLanguage();
  const [phase, setPhase] = useState('prompt'); // 'prompt' | 'waiting' | 'switching'
  const [timeLeft, setTimeLeft] = useState(300); // 5 minutes in seconds
  const [probing, setProbing] = useState(false);
  const [reconnected, setReconnected] = useState(false);
  const [switching, setSwitching] = useState(false);
  const [error, setError] = useState('');

  useLayer(isOpen, onClose);

  useEffect(() => {
    if (isOpen) {
      setPhase('prompt');
      setTimeLeft(300);
      setProbing(false);
      setReconnected(false);
      setSwitching(false);
      setError('');
    }
  }, [isOpen]);

  // Countdown & periodic PC probing during waiting phase
  useEffect(() => {
    if (!isOpen || phase !== 'waiting' || reconnected) return;

    let probeTimeout = null;
    let probingActive = false;

    const timer = setInterval(() => {
      setTimeLeft((prev) => Math.max(0, prev - 1));
    }, 1000);

    const checkPc = async () => {
      if (probingActive || reconnected) return;
      probingActive = true;
      setProbing(true);
      try {
        const online = await api.probePc();
        if (online) {
          setReconnected(true);
          setTimeout(() => {
            onReconnected?.();
            onClose?.();
          }, 800);
          return;
        }
      } catch {
        // PC still unreachable
      } finally {
        probingActive = false;
        setProbing(false);
        if (!reconnected) {
          probeTimeout = setTimeout(checkPc, 5000);
        }
      }
    };

    // First check after 1.5s, then every 5s
    probeTimeout = setTimeout(checkPc, 1500);

    return () => {
      clearInterval(timer);
      clearTimeout(probeTimeout);
    };
  }, [isOpen, phase, reconnected, onReconnected, onClose]);

  if (!isOpen) return null;

  async function handleSwitch() {
    if (switching) return;
    setSwitching(true);
    setPhase('switching');
    setError('');
    try {
      await onSwitchToLocal();
      onClose();
    } catch (err) {
      setError(err?.message || tr('Falha ao ativar a Auto-Hospedagem.'));
      setPhase('prompt');
    } finally {
      setSwitching(false);
    }
  }

  function handleStartWait() {
    setTimeLeft(300);
    setPhase('waiting');
  }

  const minutes = Math.floor(timeLeft / 60);
  const seconds = timeLeft % 60;
  const formattedTime = `${String(minutes).padStart(2, '0')}:${String(seconds).padStart(2, '0')}`;

  return (
    <div
      className="fixed inset-0 z-50 flex items-center justify-center p-4 bg-black/65 backdrop-blur-md mc-modal-backdrop"
      onClick={onClose}
    >
      <div
        className="mc-glass-pill w-full max-w-sm rounded-[28px] p-6 shadow-2xl mc-modal-card border border-white/15 text-center bg-cardDark/95 backdrop-blur-xl"
        onClick={(e) => e.stopPropagation()}
      >
        {phase === 'prompt' && (
          <div>
            <div className="w-16 h-16 mx-auto mb-4 rounded-full bg-amber-500/15 border border-amber-500/30 flex items-center justify-center text-amber-400 shadow-[0_0_24px_rgba(245,158,11,0.2)]">
              <WifiOff size={32} />
            </div>

            <h3 className="mb-2 text-xl font-bold text-white tracking-wide">
              {tr("Conexão com o PC perdida")}
            </h3>

            <p className="mb-6 text-sm text-textMuted leading-relaxed px-1">
              {tr("Não foi possível conectar ao servidor do seu PC. Deseja aguardar até 5 minutos para ver se a conexão se restabelece, ou usar a Auto-Hospedagem agora?")}
            </p>

            {error && (
              <p className="mb-4 text-xs text-red-400 bg-red-500/10 rounded-xl p-2.5 border border-red-500/20">
                {error}
              </p>
            )}

            <div className="flex flex-col gap-3">
              <button
                type="button"
                onClick={handleSwitch}
                className="w-full rounded-2xl bg-gradient-to-r from-accentPurple to-purple-600 px-5 py-3.5 text-sm font-bold text-white shadow-lg shadow-purple-950/40 active:scale-98 transition-all hover:brightness-110 flex items-center justify-center gap-2"
              >
                <Smartphone size={18} />
                {tr("Usar Auto-Hospedagem")}
              </button>

              <button
                type="button"
                onClick={handleStartWait}
                className="w-full rounded-2xl bg-white/10 border border-white/15 px-5 py-3 text-sm font-semibold text-white/90 active:scale-98 active:bg-white/15 transition-all flex items-center justify-center gap-2"
              >
                <Clock size={18} />
                {tr("Aguardar reconexão (5 min)")}
              </button>
            </div>
          </div>
        )}

        {phase === 'waiting' && (
          <div>
            {reconnected ? (
              <div className="w-16 h-16 mx-auto mb-4 rounded-full bg-emerald-500/15 border border-emerald-500/30 flex items-center justify-center text-emerald-400 shadow-[0_0_24px_rgba(16,185,129,0.3)] animate-pulse">
                <CheckCircle size={32} />
              </div>
            ) : (
              <div className="w-16 h-16 mx-auto mb-4 rounded-full bg-purple-500/15 border border-purple-500/30 flex items-center justify-center text-purple-400 relative">
                <Wifi size={30} className={probing ? 'animate-pulse' : ''} />
                <div className="absolute inset-0 rounded-full border border-purple-400/40 animate-ping opacity-60" />
              </div>
            )}

            <h3 className="mb-2 text-xl font-bold text-white tracking-wide">
              {reconnected ? tr("PC reconectado com sucesso!") : tr("Aguardando o PC…")}
            </h3>

            {!reconnected && (
              <>
                <div className="my-5 p-4 rounded-2xl bg-white/5 border border-white/10">
                  <div className="text-3xl font-mono font-bold text-accentPurpleLight tracking-widest tabular-nums">
                    {formattedTime}
                  </div>
                  <p className="mt-2 text-xs text-textMuted flex items-center justify-center gap-1.5">
                    <Loader2 size={12} className="animate-spin text-purple-400" />
                    {tr("Tentando reconectar ao PC em segundo plano…")}
                  </p>
                </div>

                {timeLeft === 0 && (
                  <p className="text-xs text-amber-300 mb-4 bg-amber-500/10 p-2.5 rounded-xl border border-amber-500/20">
                    {tr("Tempo limite esgotado (5 min). O PC continua inacessível.")}
                  </p>
                )}

                <div className="flex flex-col gap-2.5">
                  <button
                    type="button"
                    onClick={handleSwitch}
                    className="w-full rounded-2xl bg-gradient-to-r from-accentPurple to-purple-600 px-5 py-3.5 text-sm font-bold text-white shadow-lg active:scale-98 transition-all flex items-center justify-center gap-2"
                  >
                    <Smartphone size={18} />
                    {tr("Alternar para Auto-Hospedagem agora")}
                  </button>

                  <button
                    type="button"
                    onClick={onClose}
                    className="w-full rounded-2xl bg-transparent py-2.5 text-xs text-textMuted hover:text-white transition-colors"
                  >
                    {tr("Minimizar")}
                  </button>
                </div>
              </>
            )}
          </div>
        )}

        {phase === 'switching' && (
          <div className="py-4">
            <div className="w-16 h-16 mx-auto mb-4 rounded-full bg-purple-500/15 border border-purple-500/30 flex items-center justify-center text-purple-400">
              <Loader2 size={32} className="animate-spin text-accentPurpleLight" />
            </div>

            <h3 className="mb-2 text-lg font-bold text-white">
              {tr("Iniciando Auto-Hospedagem…")}
            </h3>

            <p className="text-xs text-textMuted">
              {tr("Ativando o banco e modelos locais sem reiniciar o app…")}
            </p>
          </div>
        )}
      </div>
    </div>
  );
}
