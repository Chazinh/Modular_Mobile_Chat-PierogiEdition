import React, { useEffect, useRef, useState } from 'react';
import { api, config, isNative } from '../api.js';
import { tr, useLanguage } from '../i18n.js';
import { Screen } from './Sheet.jsx';
import ChoiceField from './ChoiceField.jsx';
import { useConfirm } from './ConfirmProvider.jsx';
import { emptyProviderForm, emptyPolicyForm, redactProviderConfig, providerForm, policyForm,
  buildProviderPayload, removeProviderPayload, buildRoutesPayload, buildPolicyPayload } from '../mobile-settings.js';

const inputClass = 'mt-1 w-full rounded-xl bg-composer px-3 py-2.5 text-[16px] outline-none';
const buttonClass = 'rounded-full bg-composer px-4 py-2.5 text-sm disabled:opacity-40';
const routesJSON = config => JSON.stringify({ aliases: config.aliases, fallbacks: config.fallbacks }, null, 2);
function Field({ label, ...props }) {
  return <label className="block text-sm"><span className="text-textMuted">{label}</span><input {...props} className={inputClass} /></label>;
}

export default function LocalHostSettings({ onClose }) {
  useLanguage();
  const confirm = useConfirm();
  const [providers, setProviders] = useState(null);
  const [form, setForm] = useState(emptyProviderForm);
  const [editId, setEditId] = useState(null);
  const [formOpen, setFormOpen] = useState(false);
  const [policy, setPolicy] = useState(emptyPolicyForm);
  const [routes, setRoutes] = useState('');
  const [message, setMessage] = useState('');
  const [busy, setBusy] = useState(false);
  const [loading, setLoading] = useState(true);
  const [reload, setReload] = useState(0);
  const lifetime = useRef(null), writing = useRef(false);
  useEffect(() => {
    const ac = lifetime.current = new AbortController();
    if (!isNative() || !config.embeddedReady) { setLoading(false); return () => ac.abort(); }
    setLoading(true); setMessage('');
    Promise.all([api.localProviders({ signal: ac.signal }), api.localPolicy({ signal: ac.signal })]).then(([p, value]) => {
      if (ac.signal.aborted) return;
      const safe = redactProviderConfig(p);
      setProviders(safe); setRoutes(routesJSON(safe)); setPolicy(policyForm(value.memoryPolicy));
    }).catch(() => { if (!ac.signal.aborted) setMessage(tr('Não foi possível carregar a configuração do host Android.')); })
      .finally(() => { if (!ac.signal.aborted) setLoading(false); });
    return () => { ac.abort(); };
  }, [reload]);
  const patch = change => setForm(previous => ({ ...previous, ...change }));
  const patchPolicy = change => setPolicy(previous => ({ ...previous, ...change }));
  function validate(build) {
    try { return build(); } catch (error) { setMessage(tr(error.message)); return null; }
  }
  async function write(save, apply) {
    if (writing.current || lifetime.current?.signal.aborted) return;
    writing.current = true; setBusy(true); setMessage('');
    const signal = lifetime.current.signal;
    try {
      const result = await save({ signal });
      if (!signal.aborted) { apply(result); setMessage(tr('Configuração salva no aparelho.')); }
    } catch {
      if (!signal.aborted) setMessage(tr('Não foi possível confirmar o salvamento. Recarregue a configuração antes de tentar novamente.'));
    } finally { writing.current = false; if (!signal.aborted) setBusy(false); }
  }
  function applyProviders(value) {
    const safe = redactProviderConfig(value);
    setProviders(safe); setRoutes(routesJSON(safe)); setForm(emptyProviderForm()); setEditId(null); setFormOpen(false);
  }
  function saveProvider(event) {
    event.preventDefault();
    const payload = validate(() => buildProviderPayload(providers, form, editId));
    if (payload) void write(options => api.saveLocalProviders(payload, options), applyProviders);
  }
  function savePolicy(event) {
    event.preventDefault();
    const payload = validate(() => buildPolicyPayload(policy, providers));
    if (payload) void write(options => api.saveLocalPolicy(payload, options), value => setPolicy(policyForm(value.memoryPolicy)));
  }
  function saveRoutes() {
    const payload = validate(() => buildRoutesPayload(providers, routes));
    if (payload) void write(options => api.saveLocalProviders(payload, options), applyProviders);
  }
  async function remove(id) {
    if (!await confirm(tr('Remover o provedor “{0}” e seus mapeamentos neste aparelho?', [id]))) return;
    if (lifetime.current?.signal.aborted) return;
    void write(options => api.saveLocalProviders(removeProviderPayload(providers, id), options), applyProviders);
  }
  const selected = providers?.providers.find(p => p.id === policy.provider);
  return <Screen title={tr('Host Android')} onClose={onClose}>
    <div className="space-y-5 px-4 py-4 text-[15px]">
      <p className="text-sm text-textMuted">{tr('Esta configuração pertence ao aparelho, mesmo quando o PC está ativo. As credenciais não são replicadas para o PC.')}</p>
      <p className="text-xs text-textMuted">{tr('Os dois arquivos são baixados automaticamente enquanto o host Android está ativo: cerca de 1,45 GB em disco. Este valor não é uma medida de RAM.')}</p>
      {loading && <p role="status">{tr('Carregando configuração…')}</p>}
      {message && <p role="status" className="break-words rounded-xl bg-cardDark p-3 text-sm">{message}</p>}
      <button type="button" disabled={busy || loading} onClick={() => { setForm(emptyProviderForm()); setFormOpen(false); setReload(n => n + 1); }} className={buttonClass}>{tr('Recarregar configuração')}</button>
      {providers && !loading && <>
        <section className="mc-bubble space-y-3 rounded-[28px] p-4">
          <h3 className="text-sm font-semibold">{tr('Provedores do aparelho')}</h3>
          <p className="text-xs text-textMuted">{tr('Use APIs OpenAI-compatible, como DeepSeek direto ou OpenRouter para Sonnet. Copie o ID real do modelo do catálogo; Anthropic Messages nativa não possui adapter neste host.')}</p>
          {providers.providers.length === 0 && <p className="text-sm text-textMuted">{tr('Nenhum provedor configurado.')}</p>}
          {providers.providers.map(p => <div key={p.id} className="rounded-xl bg-composer p-3 text-sm">
            <p className="break-words font-semibold">{p.id}{providers.default === p.id && ` · ${tr('Padrão')}`}</p>
            <p className="break-all text-xs text-textMuted">{p.baseUrl}</p>
            <p className="break-words text-xs text-textMuted">{p.models.join(', ')}</p>
            <p className="text-xs text-textMuted">{tr(p.hasApiKey ? 'Chave salva (oculta)' : 'Sem chave salva')}</p>
            <div className="mt-2 flex gap-3"><button type="button" disabled={busy} onClick={() => { setEditId(p.id); setForm(providerForm(p, providers)); setFormOpen(true); setMessage(''); }}>{tr('Editar')}</button>
              <button type="button" disabled={busy} onClick={() => remove(p.id)} className="text-red-300">{tr('Remover')}</button></div>
          </div>)}
          <button type="button" disabled={busy} onClick={() => { setEditId(null); setForm(emptyProviderForm()); setFormOpen(true); setMessage(''); }} className={buttonClass}>{tr('Adicionar provedor')}</button>
          {formOpen && <form onSubmit={saveProvider} className="space-y-3">
            <fieldset disabled={busy} className="space-y-3">
              <ChoiceField label={tr('Protocolo')} value={form.protocol} onChange={protocol => patch({ protocol })} options={[
                { value: 'openai-compatible', label: 'OpenAI-compatible' }, { value: 'anthropic-messages', label: tr('Anthropic Messages (não suportado)') }]} />
              {form.protocol !== 'openai-compatible' && <p className="text-xs text-red-300">{tr('Anthropic Messages não é suportada. Use um endpoint OpenAI-compatible, como OpenRouter.')}</p>}
              <Field label={tr('ID do provedor')} value={form.id} readOnly={!!editId} required maxLength={64} autoCapitalize="none" onChange={e => patch({ id: e.target.value })} />
              <Field label={tr('baseURL HTTPS')} value={form.baseUrl} required type="url" inputMode="url" autoCapitalize="none" autoCorrect="off" onChange={e => patch({ baseUrl: e.target.value })} />
              <label className="block text-sm"><span className="text-textMuted">{tr('IDs reais dos modelos (um por linha)')}</span>
                <textarea value={form.models} required rows={3} maxLength={8192} autoCapitalize="none" autoCorrect="off" onChange={e => patch({ models: e.target.value })} className={inputClass} /></label>
              <Field label={tr('Chave de API (em branco mantém apenas no mesmo endpoint)')} value={form.apiKey} type="password" autoComplete="new-password" autoCapitalize="none" autoCorrect="off" spellCheck={false} maxLength={4096} onChange={e => patch({ apiKey: e.target.value })} />
              <label className="flex items-center gap-2 text-sm"><input type="checkbox" checked={form.clearKey} onChange={e => patch({ clearKey: e.target.checked })} />{tr('Remover chave salva')}</label>
              <label className="flex items-center gap-2 text-sm"><input type="checkbox" checked={form.makeDefault} onChange={e => patch({ makeDefault: e.target.checked })} />{tr('Usar como provedor padrão do aparelho')}</label>
              <details><summary className="cursor-pointer text-sm text-textMuted">{tr('Capacidade e reasoning (opcional)')}</summary><div className="mt-3 space-y-3">
                <ChoiceField label={tr('Formato de reasoning')} value={form.reasoningFormat} onChange={reasoningFormat => patch({ reasoningFormat })} options={[
                  { value: 'omit', label: tr('Omitir') }, { value: 'openai', label: 'OpenAI' }, { value: 'openrouter', label: 'OpenRouter' }]} />
                <Field label={tr('Janela de contexto (1024–262144 tokens)')} type="number" min={1024} max={262144} step={1} value={form.contextWindow} onChange={e => patch({ contextWindow: e.target.value })} />
                <Field label={tr('Saída máxima (256–8192 tokens)')} type="number" min={256} max={8192} step={1} value={form.maxOutput} onChange={e => patch({ maxOutput: e.target.value })} />
              </div></details>
              <div className="flex gap-2"><button type="submit" className={buttonClass}>{tr('Salvar provedor')}</button>
                <button type="button" onClick={() => { setForm(emptyProviderForm()); setFormOpen(false); setEditId(null); }} className={buttonClass}>{tr('Cancelar')}</button></div>
            </fieldset>
          </form>}
          <details><summary className="cursor-pointer text-sm text-textMuted">{tr('Mapear rotas do PC (avançado)')}</summary>
            <p className="my-2 text-xs text-textMuted">{tr('Somente aliases/fallbacks explícitos: {"aliases":{"rota-do-PC":{"provider":"seu-id","model":"modelo-real"}},"fallbacks":{}}. Nenhuma rota do PC recebe autorização automática para usar a nuvem.')}</p>
            <textarea aria-label={tr('Mapeamentos JSON sem credenciais')} value={routes} onChange={e => setRoutes(e.target.value)} rows={5} maxLength={8192} disabled={busy} spellCheck={false} className={`${inputClass} font-mono text-sm`} />
            <button type="button" disabled={busy} onClick={saveRoutes} className={`${buttonClass} mt-2`}>{tr('Salvar mapeamentos')}</button>
          </details>
        </section>
        <section className="mc-bubble space-y-3 rounded-[28px] p-4">
          <h3 className="text-sm font-semibold">{tr('Processamento da memória')}</h3>
          <p className="text-xs text-textMuted">{tr('Padrão: processar no PC quando voltar. Em standby com o PC ativo, todos os jobs do aparelho ficam pausados.')}</p>
          <form onSubmit={savePolicy}><fieldset disabled={busy} className="space-y-3">
            <ChoiceField label={tr('Política de memória')} value={policy.mode} onChange={mode => patchPolicy({ mode, historyConsent: false })} options={[
              { value: 'deferred', label: tr('Processar no PC quando voltar') }, { value: 'local', label: tr('Modelo compacto no aparelho') }, { value: 'api', label: tr('API configurada') }]} />
            {policy.mode === 'local' && <p className="text-xs text-yellow-300">{tr('O modelo local usa CPU e memória do aparelho. Ter 8 GB de RAM não garante execução ou desempenho; aguarde o download dos arquivos.')}</p>}
            {policy.mode === 'api' && <>
              <p className="text-xs text-yellow-300">{tr('A API usa internet e créditos do provedor. O histórico será enviado somente ao destino escolhido e com seu consentimento.')}</p>
              <ChoiceField label={tr('Provedor para memória')} value={policy.provider} onChange={provider => patchPolicy({ provider, model: '', historyConsent: false })} options={[
                { value: '', label: tr('Escolha um provedor') }, ...providers.providers.map(p => ({ value: p.id, label: p.id }))]} />
              <ChoiceField label={tr('Modelo para memória')} value={policy.model} onChange={model => patchPolicy({ model, historyConsent: false })} options={[
                { value: '', label: tr('Escolha um modelo') }, ...(selected?.models || []).map(model => ({ value: model, label: model }))]} />
              <label className="flex items-start gap-2 text-sm"><input type="checkbox" checked={policy.historyConsent} onChange={e => patchPolicy({ historyConsent: e.target.checked })} className="mt-1" />{tr('Autorizo enviar histórico para processar memória na API selecionada.')}</label>
              <details><summary className="cursor-pointer text-sm text-textMuted">{tr('Limites da API (opcional)')}</summary><div className="mt-3 space-y-3">
                <Field label={tr('Saída por job (256–2048 tokens; padrão 1200)')} type="number" min={256} max={2048} step={1} value={policy.maxTokens} onChange={e => patchPolicy({ maxTokens: e.target.value })} />
                <Field label={tr('Entrada por job (1024–16384 tokens; padrão 6000)')} type="number" min={1024} max={16384} step={1} value={policy.maxInputTokens} onChange={e => patchPolicy({ maxInputTokens: e.target.value })} />
                <Field label={tr('Tempo limite (5–120 segundos; padrão 60)')} type="number" min={5} max={120} step={1} value={policy.timeoutSeconds} onChange={e => patchPolicy({ timeoutSeconds: e.target.value })} />
              </div></details>
            </>}
            <button type="submit" className={buttonClass}>{tr('Salvar política de memória')}</button>
          </fieldset></form>
        </section>
      </>}
    </div>
  </Screen>;
}
