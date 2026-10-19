import { tr, useLanguage, currentLanguage } from '../i18n.js';
import React, { useEffect, useRef, useState } from 'react';
import { api } from '../api.js';
import { queueStyleSettings, flushStyleSettings } from '../style-settings.js';

// GLOBAL app settings (apply to every conversation from the next reply on; stored on the PC):
// character limit per generated text, narration × dialogue ratio and content weight/explicitness.
const fmt = (n) => Number(n).toLocaleString(currentLanguage());

const CONTENT_PRESETS = [
  ['Proibido +18', 'Conteúdo +18: proibido. Mantenha a história adequada para todos os públicos.'],
  ['Sugestivo', 'Intensidade: sugestivo, sem descrições explícitas; use elipses e insinuações.'],
  ['Permitido +18', 'Conteúdo +18: permitido quando a cena e os personagens pedirem, entre adultos e no tom da história.'],
];

export default function StyleForm({ onChange }) {
  useLanguage();
  const [s, setS] = useState(null);
  const [limits, setLimits] = useState({ botCharsMin: 500, botCharsMax: 8000, contentMax: 1500, tempMin: 0.2, tempMax: 1.4 });
  const [error, setError] = useState('');
  const [saved, setSaved] = useState('');
  const timer = useRef(null);
  const mounted = useRef(true);

  async function load() {
    try {
      setError('');
      const r = await api.globalSettings();
      setS({ lengthMode: 'maximum', monologueChars: 300, turnPacing: 'interactive', temperature: 0.8, ...r.settings });
      setLimits((l) => ({ tempMin: 0.2, tempMax: 1.4, ...l, ...(r.limits || {}) }));
    } catch (e) {
      setError(e.message || tr("Não foi possível carregar os ajustes."));
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
        setError(e.message || tr("Não foi possível salvar."));
      }
    }, 500);
  }

  if (!s) {
    return (
      <div className="px-5 text-sm text-textMuted">
        {error ? (
          <div role="alert" className="space-y-3">
            <p className="text-red-300">{tr(error)}</p>
            <button onClick={load} className="rounded-full bg-cardDark px-4 py-2 text-white">{tr("Tentar de novo")}</button>
          </div>
        ) : (
          tr("Carregando…")
        )}
      </div>
    );
  }

  const narration = 100 - s.dialoguePct;
  return (
    <div className="space-y-5 px-5 pb-6 text-sm text-textMuted">
      <p>{tr("Valem para ")}<b className="text-white">{tr("todos os chats")}</b>{tr(", a partir da próxima resposta. ")}{saved && <span className="text-accentPurpleLight">{tr(saved)}</span>}</p>

      <section className="space-y-2 mc-bubble rounded-[26px] p-4 text-white">
        <div className="flex items-center justify-between">
          <span className="font-medium">{tr("Tamanho da resposta")}</span>
          <span className="rounded-md bg-accentPurple/20 px-2 py-0.5 font-mono text-accentPurpleLight">{fmt(s.botChars)}</span>
        </div>
        <input
          type="range" min={limits.botCharsMin} max={limits.botCharsMax} step="100" value={s.botChars}
          onChange={(e) => change({ botChars: Number(e.target.value) })}
          className="w-full accent-[#7c3aed]" aria-label={tr("Tamanho da resposta")}
        />
        <div className="flex flex-wrap gap-2">
          {[500, 1000, 2000, 4000, 6000, 8000].map((n) => (
            <button key={n} onClick={() => change({ botChars: n })} aria-pressed={s.botChars === n} className={`mc-bubble rounded-full px-3 py-1 text-xs ${s.botChars === n ? 'bg-accentPurple/30 ring-1 ring-accentPurpleLight' : 'bg-composer'}`}>
              {fmt(n)}
            </button>
          ))}
        </div>
        <div className="flex flex-wrap gap-2" role="group" aria-label={tr("Como usar o tamanho da resposta")}>
          {[['maximum', 'Até este tamanho'], ['target', 'Alvo aproximado']].map(([value, label]) => (
            <button key={value} onClick={() => change({ lengthMode: value })} aria-pressed={s.lengthMode === value} className={`mc-bubble rounded-full px-3 py-1 text-xs ${s.lengthMode === value ? 'bg-accentPurple/30 ring-1 ring-accentPurpleLight' : 'bg-composer'}`}>{tr(label)}</button>
          ))}
        </div>
        <p className="text-xs text-textMuted">{s.lengthMode === 'target' ? tr("Mira em torno deste tamanho: aceita de {0} a {1} e só corta acima de {2}, no fim da última frase completa.", [fmt(Math.ceil(s.botChars * 0.85)), fmt(Math.ceil(s.botChars * 1.25)), fmt(Math.ceil(s.botChars * 1.35))]) : tr("Pode responder menos quando a reação estiver completa, sem preencher o limite.")}{tr(" O orçamento de tokens é enviado à API antes de gerar. A conversão é aproximada; o teto também é conferido no servidor, em unidades UTF-16 (um emoji conta como 2).")}</p>
      </section>

      <section className="space-y-3 mc-bubble rounded-[26px] p-4 text-white">
        <div className="flex items-center justify-between gap-2">
          <span className="font-medium">{tr("Falas seguidas de cada personagem")}</span>
          <span className="font-mono text-accentPurpleLight">{s.monologueChars === 0 ? tr("Livre") : tr("{0} caracteres", [fmt(s.monologueChars)])}</span>
        </div>
        <input type="range" min={limits.monologueCharsMin || 100} max={limits.monologueCharsMax || 2000} step="50" value={s.monologueChars || 300} disabled={s.monologueChars === 0} onChange={(e) => change({ monologueChars: Number(e.target.value) })} className="w-full accent-[#7c3aed] disabled:opacity-30" aria-label={tr("Limite de falas seguidas por personagem")} />
        <div className="flex flex-wrap gap-2">
          {[150, 300, 600, 0].map((n) => <button key={n} onClick={() => change({ monologueChars: n })} aria-pressed={s.monologueChars === n} className={`mc-bubble rounded-full px-3 py-1 text-xs ${s.monologueChars === n ? 'bg-accentPurple/30 ring-1 ring-accentPurpleLight' : 'bg-composer'}`}>{n === 0 ? tr("Livre") : fmt(n)}</button>)}
        </div>
        <p className="text-xs text-textMuted">{tr("Conta as falas seguidas do mesmo personagem, inclusive quando separadas por ações ou parágrafos (ações entre asteriscos não entram; emoji conta como 2). Um excesso recebe uma tentativa de reescrita; o resultado pode variar conforme o modelo.")}</p>
        <span className="block font-medium">{tr("Avanço da cena por resposta")}</span>
        <div className="flex flex-wrap gap-2" role="group" aria-label={tr("Avanço da cena por resposta")}>
          {[['interactive', 'Devolver a vez'], ['balanced', 'Equilibrado'], ['free', 'Livre']].map(([value, label]) => <button key={value} onClick={() => change({ turnPacing: value })} aria-pressed={s.turnPacing === value} className={`mc-bubble rounded-full px-3 py-1 text-xs ${s.turnPacing === value ? 'bg-accentPurple/30 ring-1 ring-accentPurpleLight' : 'bg-composer'}`}>{tr(label)}</button>)}
        </div>
        <p className="text-xs text-textMuted">{s.turnPacing === 'interactive' ? tr("Uma reação curta por personagem relevante e uma pausa para você responder, sem longas conversas entre eles.") : s.turnPacing === 'balanced' ? tr("Desenvolve a reação e até um acontecimento novo antes de devolver a vez.") : tr("Permite desenvolver mais a cena, sempre preservando suas escolhas e ações.")}</p>
      </section>

      <section className="space-y-2 mc-bubble rounded-[26px] p-4 text-white">
        <span className="font-medium">{tr("Narração × personagens")}</span>
        <div className="flex items-center justify-between text-xs">
          <span><b className="font-mono text-accentPurpleLight">{narration}%</b>{tr(" narração")}</span>
          <span><b className="font-mono text-accentPurpleLight">{s.dialoguePct}%</b>{tr(" personagens (falas)")}</span>
        </div>
        <input
          type="range" min="0" max="100" step="5" value={s.dialoguePct}
          onChange={(e) => change({ dialoguePct: Number(e.target.value) })}
          className="w-full accent-[#7c3aed]" aria-label={tr("Proporção entre narração e falas dos personagens")}
        />
        <p className="text-xs text-textMuted">{tr("Proporção alvo do texto da próxima resposta. Nos extremos, 0% gera só narração e 100% só falas. Padrão: 35% narração / 65% falas.")}</p>
      </section>

      <section className="space-y-2 mc-bubble rounded-[26px] p-4 text-white">
        <div className="flex items-center justify-between">
          <span className="font-medium">{tr("Criatividade do modelo")}</span>
          <span className="rounded-md bg-accentPurple/20 px-2 py-0.5 font-mono text-accentPurpleLight">
            {Number(s.temperature ?? 0.8).toLocaleString(currentLanguage(), { minimumFractionDigits: 2, maximumFractionDigits: 2 })}
          </span>
        </div>
        <div className="flex items-center justify-between text-xs text-textMuted">
          <span>{tr("Menos criativo")}</span>
          <span>{tr("Mais criativo")}</span>
        </div>
        <input
          type="range"
          min={limits.tempMin || 0.2}
          max={limits.tempMax || 1.4}
          step="0.05"
          value={s.temperature ?? 0.8}
          onChange={(e) => change({ temperature: Number(e.target.value) })}
          className="w-full accent-[#7c3aed]"
          aria-label={tr("Criatividade do modelo (termômetro do seguimento narrativo)")}
        />
        <div className="flex flex-wrap gap-2">
          {[
            ['Fiel', 0.4],
            ['Equilibrado', 0.8],
            ['Criativo', 1.1],
          ].map(([label, val]) => (
            <button
              key={val}
              onClick={() => change({ temperature: val })}
              aria-pressed={(s.temperature ?? 0.8) === val}
              className={`mc-bubble rounded-full px-3 py-1 text-xs ${(s.temperature ?? 0.8) === val ? 'bg-accentPurple/30 ring-1 ring-accentPurpleLight' : 'bg-composer'}`}
            >
              {tr(label)} ({val.toFixed(1)})
            </button>
          ))}
        </div>
        <p className="text-xs text-textMuted">
          {(s.temperature ?? 0.8) < 0.6
            ? tr("Menor criatividade: segue a narrativa com fidelidade estrita, consistência e previsibilidade.")
            : (s.temperature ?? 0.8) > 0.95
            ? tr("Maior criatividade: mais liberdade e iniciativa narrativa, desdobramentos surpreendentes e reviravoltas na cena.")
            : tr("Equilibrado: combina fidelidade ao rumo da história com criatividade espontânea nas reações.")}
          {tr(" Controla o termômetro (temperatura) do seguimento narrativo para as respostas.")}
        </p>
      </section>

      <section className="space-y-2 mc-bubble rounded-[26px] p-4 text-white">
        <span className="font-medium">{tr("Peso e explicitude do conteúdo")}</span>
        <div className="flex flex-wrap gap-2">
          {CONTENT_PRESETS.map(([label, text]) => (
            <button key={tr(label)} onClick={() => change({ content: text })} aria-pressed={s.content === text} className={`mc-bubble rounded-full px-3 py-1 text-xs ${s.content === text ? 'bg-accentPurple/30 ring-1 ring-accentPurpleLight' : 'bg-composer'}`}>
              {tr(label)}
            </button>
          ))}
          <button onClick={() => change({ content: '' })} className="mc-bubble rounded-full px-3 py-1 text-xs">{tr("Limpar")}</button>
        </div>
        <textarea
          value={s.content}
          maxLength={limits.contentMax}
          rows={5}
          onChange={(e) => change({ content: e.target.value })}
          placeholder={tr("Escreva como quiser. Ex.: Conteúdo +18 proibido. Violência: moderada, sem gore.")}
          className="w-full resize-y rounded-xl bg-composer px-3 py-2.5 text-[15px] text-white outline-none placeholder:text-textMuted"
          aria-label={tr("Peso e explicitude do conteúdo")}
        />
        <p className="text-right text-xs text-textMuted">{s.content.length}/{limits.contentMax}</p>
      </section>

      {error && <p role="alert" className="text-red-300">{tr(error)}</p>}
    </div>
  );
}
