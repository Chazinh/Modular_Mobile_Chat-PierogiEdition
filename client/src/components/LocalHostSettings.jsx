import React, { useEffect, useRef, useState } from 'react';
import { api, config, isNative } from '../api.js';
import { tr, useLanguage } from '../i18n.js';
import { Screen } from './Sheet.jsx';
import ChoiceField from './ChoiceField.jsx';
import { useConfirm } from './ConfirmProvider.jsx';
import { emptyProviderForm, emptyPolicyForm, redactProviderConfig, providerForm, policyForm,
  providerTemplate, redactDiscovery, buildProviderPayload, removeProviderPayload, buildRoutesPayload, buildPolicyPayload } from '../mobile-settings.js';
import { createLatestRequest } from '../connection-lifetime.js';
import { notifyProviderCatalog } from '../provider-catalog.js';

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
  const [discovery, setDiscovery] = useState(null);
  const [discovering, setDiscovering] = useState(false);
  const [form, setForm] = useState(emptyProviderForm);
  const [editId, setEditId] = useState(null);
  const [formOpen, setFormOpen] = useState(false);
  const [policy, setPolicy] = useState(emptyPolicyForm);
  const [routes, setRoutes] = useState('');
  const [message, setMessage] = useState('');
  const [busy, setBusy] = useState(false);
  const [loading, setLoading] = useState(true);
  const [reload, setReload] = useState(0);
  const lifetime = useRef(null), writing = useRef(false), discoveryRequest = useRef(null);
  useEffect(() => {
    const ac = lifetime.current = new AbortController();
    const reads = discoveryRequest.current = createLatestRequest(() => 'embedded');
    if (!isNative() || !config.embeddedReady) { setLoading(false); return () => ac.abort(); }
    setLoading(true); setMessage('');
    Promise.all([api.localProviders({ signal: ac.signal }), api.localPolicy({ signal: ac.signal })]).then(([p, value]) => {
      if (ac.signal.aborted) return;
      const safe = redactProviderConfig(p);
      setProviders(safe); setRoutes(routesJSON(safe)); setPolicy(policyForm(value.memoryPolicy));
      setDiscovery(redactDiscovery(p.discovery));
      if (p.discovery) notifyProviderCatalog();
    }).catch(() => { if (!ac.signal.aborted) setMessage(tr('Não foi possível carregar a configuração do host Android.')); })
      .finally(() => { if (!ac.signal.aborted) setLoading(false); });
    return () => { ac.abort(); reads.cancel(); };
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
    notifyProviderCatalog();
  }
  async function refreshDiscovery() {
    const lifetimeSignal = lifetime.current?.signal;
    if (!providers || loading || writing.current || !lifetimeSignal || lifetimeSignal.aborted) return;
    setDiscovering(true);
    try {
      await discoveryRequest.current.run('discovery', async (signal, isCurrent) => {
        const value = await api.localDiscovery({ signal });
        if (!lifetimeSignal.aborted && isCurrent()) { setDiscovery(redactDiscovery(value)); notifyProviderCatalog(); }
      });
    } catch {
      if (!lifetimeSignal.aborted) setMessage(tr('Não foi possível atualizar a detecção local.'));
    } finally { if (!lifetimeSignal.aborted) setDiscovering(false); }
  }
  function openTemplate(kind) {
    const draft = providerTemplate(kind), existing = providers.providers.find(p => p.id === draft.id);
    setEditId(existing?.id || null); setForm(existing ? providerForm(existing, providers) : draft); setFormOpen(true);
    setMessage(existing ? tr('Provedor já salvo: o atalho abriu a configuração existente.') : '');
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
  const statusLabels = { available: tr('Disponível'), auth_required: tr('Exige autenticação'), absent: tr('Não detectado'),
    incompatible: tr('Resposta incompatível'), detected_native_api: tr('API própria detectada') };
  const statusHelp = { available: tr('API local detectada. Selecione explicitamente um modelo no seletor do chat.'),
    auth_required: tr('A descoberta não envia credenciais. Configure o provedor e digite a chave, se ele aceitar OpenAI-compatible.'),
    absent: tr('O serviço não respondeu. O app não iniciou nem instalou serviços e não abriu portas.'),
    incompatible: tr('A resposta não corresponde ao protocolo esperado.'),
    detected_native_api: tr('OpenCode serve usa API própria, incompatível com OpenAI chat completions. Este host não implementa inferência nativa do OpenCode.') };
  return <Screen title={tr('Host Android')} onClose={onClose}>
    <div className="space-y-5 px-4 py-4 text-[15px]">
      <p className="text-sm text-textMuted">{tr('Esta configuração pertence ao aparelho, mesmo quando o PC está ativo. As credenciais não são replicadas para o PC.')}</p>
      <p className="text-xs text-textMuted">{tr('Os dois arquivos são baixados automaticamente enquanto o host Android está ativo: cerca de 1,45 GB em disco. Este valor não é uma medida de RAM.')}</p>
      {loading && <p role="status">{tr('Carregando configuração…')}</p>}
      {message && <p role="status" className="break-words rounded-xl bg-cardDark p-3 text-sm">{message}</p>}
      <button type="button" disabled={busy || loading || discovering} onClick={() => { setForm(emptyProviderForm()); setFormOpen(false); setReload(n => n + 1); }} className={buttonClass}>{tr('Recarregar configuração')}</button>
      {providers && !loading && <>
        <section aria-label={tr('Serviços em localhost')} className="mc-bubble space-y-3 rounded-[28px] p-4">
          <h3 className="text-sm font-semibold">{tr('Serviços em localhost')}</h3>
          <p className="text-xs text-textMuted">{tr('A detecção consulta somente o loopback do host Android, mesmo com o PC ativo. Localhost não é o PC pareado; uma API local pode encaminhar pedidos para a internet. O modelo compacto no aparelho continua separado.')}</p>
          <p className="text-xs text-textMuted">{tr('Detecção somente de leitura, com cache de 30 segundos no host. Atualizar dentro desse prazo pode repetir o resultado; não há polling de descoberta.')}</p>
          {discovery ? <>
            {discovery.checkedAt && <p className="text-xs text-textMuted">{tr('Última detecção: {0}', [new Date(discovery.checkedAt).toLocaleString()])}</p>}
            {[{ id: 'nineRouter', label: '9Router · 127.0.0.1:20128 · OpenAI-compatible' },
              { id: 'openCode', label: 'OpenCode serve · 127.0.0.1:4096 · API própria' }].map(({ id, label }) => <div key={id} className="space-y-1 rounded-xl bg-composer p-3 text-sm">
              <h4 className="font-semibold">{id === 'openCode' ? tr('OpenCode serve · 127.0.0.1:4096 · API própria') : label}</h4>
              <p role="status">{statusLabels[discovery[id].status]}</p>
              <p className="text-xs text-textMuted">{statusHelp[discovery[id].status]}</p>
              {id === 'nineRouter' && discovery.nineRouter.models.length > 0 && <p className="break-words text-xs text-textMuted">{tr('IDs detectados: {0}', [discovery.nineRouter.models.join(', ')])}</p>}
              {id === 'nineRouter' && discovery.nineRouter.status === 'auth_required' && <button type="button" disabled={busy} onClick={() => openTemplate('9router')} className={`${buttonClass} mt-2`}>{tr('Configurar 9Router com chave')}</button>}
            </div>)}
          </> : <p className="text-sm text-textMuted">{tr('Detecção local ainda indisponível.')}</p>}
          <p className="text-xs text-textMuted">{tr('Com o host Android ativo, o 9Router disponível aparece em API configurada no celular no seletor do chat. A descoberta não escolhe modelo nem muda a rota da conversa. Uma configuração manual salva tem prioridade.')}</p>
          <p className="text-xs text-textMuted">{tr('OpenCode serve usa API própria, incompatível com OpenAI chat completions. Este host não implementa inferência nativa do OpenCode.')}</p>
          <p className="text-xs text-textMuted">{tr('OpenCode Zen HTTPS é um serviço opcional pela internet, diferente de OpenCode serve. Exige chave e pode consumir créditos; use apenas IDs do catálogo compatíveis com chat completions.')}</p>
          <button type="button" disabled={busy || discovering} onClick={refreshDiscovery} className={buttonClass}>{tr(discovering ? 'Detectando…' : 'Atualizar detecção')}</button>
        </section>
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
          <div className="flex flex-wrap gap-2">
            <button type="button" disabled={busy} onClick={() => openTemplate('9router')} className={buttonClass}>{tr('Preencher 9Router local')}</button>
            <button type="button" disabled={busy} onClick={() => openTemplate('zen')} className={buttonClass}>{tr('Preencher OpenCode Zen HTTPS')}</button>
          </div>
          <p className="text-xs text-textMuted">{tr('Os atalhos só preenchem o formulário. Informe os IDs reais e a chave; autorize loopback se necessário e salve explicitamente. Nenhum atalho autoriza envio de histórico para memória.')}</p>
          {formOpen && <form onSubmit={saveProvider} className="space-y-3">
            <fieldset disabled={busy} className="space-y-3">
              <ChoiceField label={tr('Protocolo')} value={form.protocol} onChange={protocol => patch({ protocol })} options={[
                { value: 'openai-compatible', label: 'OpenAI-compatible' }, { value: 'anthropic-messages', label: tr('Anthropic Messages (não suportado)') }]} />
              {form.protocol !== 'openai-compatible' && <p className="text-xs text-red-300">{tr('Anthropic Messages não é suportada. Use um endpoint OpenAI-compatible, como OpenRouter.')}</p>}
              <Field label={tr('ID do provedor')} value={form.id} readOnly={!!editId} required maxLength={64} autoCapitalize="none" onChange={e => patch({ id: e.target.value })} />
              <Field label={tr('baseURL (HTTPS ou loopback autorizado)')} value={form.baseUrl} required type="url" inputMode="url" autoCapitalize="none" autoCorrect="off" onChange={e => patch({ baseUrl: e.target.value })} />
              <label className="flex items-start gap-2 text-sm"><input type="checkbox" checked={form.allowLoopback} onChange={e => patch({ allowLoopback: e.target.checked })} className="mt-1" />{tr('Autorizar loopback neste aparelho (127/8, ::1 ou localhost exato)')}</label>
              <p className="text-xs text-textMuted">{tr('HTTP só é aceito com esta autorização e destino loopback literal ou localhost exato. Provedores pela internet exigem HTTPS. Informe a baseURL sem /chat/completions.')}</p>
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
