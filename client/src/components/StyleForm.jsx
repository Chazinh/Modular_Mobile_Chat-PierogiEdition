import React, { useEffect, useRef, useState } from 'react';
import { api } from '../api.js';
import { queueStyleSettings, flushStyleSettings } from '../style-settings.js';

// GLOBAL app settings (apply to every conversation from the next reply on; stored on the PC):
// character limit per generated text, narration × dialogue ratio and content weight/explicitness.
const fmt = (n) => Number(n).toLocaleString('pt-BR');

const CONTENT_PRESETS = [
  ['Proibido +18', 'Conteúdo +18: proibido. Mantenha a história adequada para todos os públicos.'],
  ['Sugestivo', 'Intensidade: sugestivo, sem descrições explícitas; use elipses e insinuações.'],
  ['Permitido +18', 'Conteúdo +18: permitido quando a cena e os personagens pedirem, entre adultos e no tom da história.'],
];

export default function StyleForm({ onChange }) {
  const [s, setS] = useState(null);
  const [limits, setLimits] = useState({ botCharsMin: 500, botCharsMax: 8000, contentMax: 1500 });
  const [error, setError] = useState('');
  const [saved, setSaved] = useState('');
  const timer = useRef(null);
  const mounted = useRef(true);

  async function load() {
    try {
      setError('');
      const r = await api.globalSettings();
      setS({ lengthMode: 'maximum', monologueChars: 300, turnPacing: 'interactive', ...r.settings });
      setLimits((l) => ({ ...l, ...(r.limits || {}) }));
    } catch (e) {
      setError(e.message || 'Não foi possível carregar os ajustes.');
    }
  }
  useEffect(() => {
    mounted.current = true;
    load();
    return () => {
      mounted.current = false;
      clearTimeout(timer.current);
      flushStyleSettings().catch(() => {});
    };
  }, []);

  // saves after a short delay (dragging the slider does not fire one request per pixel)
  function change(patch) {
    setS((cur) => ({ ...cur, ...patch }));
    queueStyleSettings(patch);
    setSaved('Salvando…');
    clearTimeout(timer.current);
    timer.current = setTimeout(async () => {
      try {
        const r = await flushStyleSettings();
        if (!mounted.current) return;
        setError('');
        setSaved('Salvo no PC');
        if (r) onChange?.(r.settings);
      } catch (e) {
        if (!mounted.current) return;
        setSaved('');
        setError(e.message || 'Não foi possível salvar.');
      }
    }, 500);
  }

  if (!s) {
    return (
      <div className="px-5 text-sm text-textMuted">
        {error ? (
          <div role="alert" className="space-y-3">
            <p className="text-red-300">{error}</p>
            <button onClick={load} className="rounded-full bg-cardDark px-4 py-2 text-white">Tentar de novo</button>
          </div>
        ) : (
          'Carregando…'
        )}
      </div>
    );
  }

  const narration = 100 - s.dialoguePct;
  return (
    <div className="space-y-5 px-5 pb-6 text-sm text-textMuted">
      <p>Valem para <b className="text-white">todos os chats</b>, a partir da próxima resposta. {saved && <span className="text-accentPurpleLight">{saved}</span>}</p>

      <section className="space-y-2 rounded-2xl bg-cardDark p-4 text-white">
        <div className="flex items-center justify-between">
          <span className="font-medium">Tamanho da resposta</span>
          <span className="rounded-md bg-accentPurple/20 px-2 py-0.5 font-mono text-accentPurpleLight">{fmt(s.botChars)}</span>
        </div>
        <input
          type="range" min={limits.botCharsMin} max={limits.botCharsMax} step="100" value={s.botChars}
          onChange={(e) => change({ botChars: Number(e.target.value) })}
          className="w-full accent-[#7c3aed]" aria-label="Tamanho da resposta"
        />
        <div className="flex flex-wrap gap-2">
          {[500, 1000, 2000, 4000, 6000, 8000].map((n) => (
            <button key={n} onClick={() => change({ botChars: n })} aria-pressed={s.botChars === n} className={`rounded-full px-3 py-1 text-xs ${s.botChars === n ? 'bg-accentPurple/30 ring-1 ring-accentPurpleLight' : 'bg-composer'}`}>
              {fmt(n)}
            </button>
          ))}
        </div>
        <div className="flex flex-wrap gap-2" role="group" aria-label="Como usar o tamanho da resposta">
          {[['maximum', 'Até este tamanho'], ['target', 'Alvo aproximado']].map(([value, label]) => (
            <button key={value} onClick={() => change({ lengthMode: value })} aria-pressed={s.lengthMode === value} className={`rounded-full px-3 py-1 text-xs ${s.lengthMode === value ? 'bg-accentPurple/30 ring-1 ring-accentPurpleLight' : 'bg-composer'}`}>{label}</button>
          ))}
        </div>
        <p className="text-xs text-textMuted">{s.lengthMode === 'target' ? 'Busca de 85% a 100% deste tamanho, sem ultrapassá-lo.' : 'Pode responder menos quando a reação estiver completa, sem preencher o limite.'} O orçamento de tokens é enviado à API antes de gerar. A conversão é aproximada; o teto também é conferido no servidor, em unidades UTF-16 (um emoji conta como 2).</p>
      </section>

      <section className="space-y-3 rounded-2xl bg-cardDark p-4 text-white">
        <div className="flex items-center justify-between gap-2">
          <span className="font-medium">Falas seguidas de cada personagem</span>
          <span className="font-mono text-accentPurpleLight">{s.monologueChars === 0 ? 'Livre' : `${fmt(s.monologueChars)} caracteres`}</span>
        </div>
        <input type="range" min={limits.monologueCharsMin || 100} max={limits.monologueCharsMax || 2000} step="50" value={s.monologueChars || 300} disabled={s.monologueChars === 0} onChange={(e) => change({ monologueChars: Number(e.target.value) })} className="w-full accent-[#7c3aed] disabled:opacity-30" aria-label="Limite de falas seguidas por personagem" />
        <div className="flex flex-wrap gap-2">
          {[150, 300, 600, 0].map((n) => <button key={n} onClick={() => change({ monologueChars: n })} aria-pressed={s.monologueChars === n} className={`rounded-full px-3 py-1 text-xs ${s.monologueChars === n ? 'bg-accentPurple/30 ring-1 ring-accentPurpleLight' : 'bg-composer'}`}>{n === 0 ? 'Livre' : fmt(n)}</button>)}
        </div>
        <p className="text-xs text-textMuted">Conta as falas seguidas do mesmo personagem, inclusive quando separadas por ações ou parágrafos (ações entre asteriscos não entram; emoji conta como 2). Um excesso recebe uma tentativa de reescrita; o resultado pode variar conforme o modelo.</p>
        <span className="block font-medium">Avanço da cena por resposta</span>
        <div className="flex flex-wrap gap-2" role="group" aria-label="Avanço da cena por resposta">
          {[['interactive', 'Devolver a vez'], ['balanced', 'Equilibrado'], ['free', 'Livre']].map(([value, label]) => <button key={value} onClick={() => change({ turnPacing: value })} aria-pressed={s.turnPacing === value} className={`rounded-full px-3 py-1 text-xs ${s.turnPacing === value ? 'bg-accentPurple/30 ring-1 ring-accentPurpleLight' : 'bg-composer'}`}>{label}</button>)}
        </div>
        <p className="text-xs text-textMuted">{s.turnPacing === 'interactive' ? 'Uma reação curta por personagem relevante e uma pausa para você responder, sem longas conversas entre eles.' : s.turnPacing === 'balanced' ? 'Desenvolve a reação e até um acontecimento novo antes de devolver a vez.' : 'Permite desenvolver mais a cena, sempre preservando suas escolhas e ações.'}</p>
      </section>

      <section className="space-y-2 rounded-2xl bg-cardDark p-4 text-white">
        <span className="font-medium">Narração × personagens</span>
        <div className="flex items-center justify-between text-xs">
          <span><b className="font-mono text-accentPurpleLight">{narration}%</b> narração</span>
          <span><b className="font-mono text-accentPurpleLight">{s.dialoguePct}%</b> personagens (falas)</span>
        </div>
        <input
          type="range" min="0" max="100" step="5" value={s.dialoguePct}
          onChange={(e) => change({ dialoguePct: Number(e.target.value) })}
          className="w-full accent-[#7c3aed]" aria-label="Proporção entre narração e falas dos personagens"
        />
        <p className="text-xs text-textMuted">Proporção alvo do texto da próxima resposta. Nos extremos, 0% gera só narração e 100% só falas. Padrão: 35% narração / 65% falas.</p>
      </section>

      <section className="space-y-2 rounded-2xl bg-cardDark p-4 text-white">
        <span className="font-medium">Peso e explicitude do conteúdo</span>
        <div className="flex flex-wrap gap-2">
          {CONTENT_PRESETS.map(([label, text]) => (
            <button key={label} onClick={() => change({ content: text })} aria-pressed={s.content === text} className={`rounded-full px-3 py-1 text-xs ${s.content === text ? 'bg-accentPurple/30 ring-1 ring-accentPurpleLight' : 'bg-composer'}`}>
              {label}
            </button>
          ))}
          <button onClick={() => change({ content: '' })} className="rounded-full bg-composer px-3 py-1 text-xs">Limpar</button>
        </div>
        <textarea
          value={s.content}
          maxLength={limits.contentMax}
          rows={5}
          onChange={(e) => change({ content: e.target.value })}
          placeholder="Escreva como quiser. Ex.: Conteúdo +18 proibido. Violência: moderada, sem gore."
          className="w-full resize-y rounded-xl bg-composer px-3 py-2.5 text-[15px] text-white outline-none placeholder:text-textMuted"
          aria-label="Peso e explicitude do conteúdo"
        />
        <p className="text-right text-xs text-textMuted">{s.content.length}/{limits.contentMax}</p>
      </section>

      {error && <p role="alert" className="text-red-300">{error}</p>}
    </div>
  );
}
