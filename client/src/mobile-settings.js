// Form contracts for the embedded host. Only an explicitly entered key belongs in a PUT.
export const MEMORY_BUDGETS = Object.freeze({ maxTokens: 1200, maxInputTokens: 6000, timeoutSeconds: 60 });
export const emptyProviderForm = () => ({ id: '', baseUrl: '', models: '', apiKey: '', protocol: 'openai-compatible', reasoningFormat: 'omit', contextWindow: 8192, maxOutput: 2048, makeDefault: false, clearKey: false });
export const emptyPolicyForm = () => ({ mode: 'deferred', provider: '', model: '', historyConsent: false, ...MEMORY_BUDGETS });
const fail = message => { throw new Error(message); };
const text = value => typeof value === 'string' ? value.trim() : '';
const validModel = value => typeof value === 'string' && !!value.trim() && value.length <= 256 && !/[\x00-\x1f\x7f]/.test(value);
function integer(value, min, max) {
  const n = value === '' || value == null ? NaN : Number(value);
  if (!Number.isInteger(n) || n < min || n > max) fail('Limites inválidos. Confira os intervalos indicados.');
  return n;
}
export function redactProviderConfig(config = {}) {
  return {
    default: config.default ?? null,
    providers: (config.providers || []).map(p => ({ id: p.id, baseUrl: p.baseUrl, models: [...p.models],
      hasApiKey: p.hasApiKey === true, allowLoopback: p.allowLoopback === true,
      reasoningFormat: p.reasoningFormat || 'omit', contextWindow: p.contextWindow ?? 8192, maxOutput: p.maxOutput ?? 2048 })),
    aliases: structuredClone(config.aliases || {}), fallbacks: structuredClone(config.fallbacks || {}),
  };
}
export function providerForm(provider, config) {
  return { ...emptyProviderForm(), id: provider.id, baseUrl: provider.baseUrl, models: provider.models.join('\n'),
    reasoningFormat: provider.reasoningFormat, contextWindow: provider.contextWindow, maxOutput: provider.maxOutput,
    makeDefault: config.default === provider.id };
}
function writableConfig(config) {
  const safe = redactProviderConfig(config);
  return { ...safe, providers: safe.providers.map(({ hasApiKey, ...p }) => p) };
}
export function validateRouteMappings(mappings, providers) {
  if (!mappings || typeof mappings !== 'object' || Array.isArray(mappings)
      || Object.keys(mappings).some(k => !['aliases', 'fallbacks'].includes(k))) fail('Use apenas aliases e fallbacks, sem chaves de API.');
  const out = {};
  for (const kind of ['aliases', 'fallbacks']) {
    const routes = mappings[kind] || {};
    if (typeof routes !== 'object' || Array.isArray(routes) || Object.keys(routes).length > 256) fail('Mapeamento de rotas inválido.');
    for (const [route, target] of Object.entries(routes)) {
      if (!route.trim() || route.length > 512 || route === 'mobile-local' || route.startsWith('mobile-api:')
          || !target || Object.keys(target).some(k => !['provider', 'model'].includes(k))
          || !providers.some(p => p.id === target.provider && p.models.includes(target.model))) fail('Cada rota exige um provedor e um modelo configurados.');
    }
    out[kind] = structuredClone(routes);
  }
  return out;
}
export function buildProviderPayload(config, form, editId = null) {
  if (form.protocol !== 'openai-compatible') fail('Anthropic Messages não é suportada. Use um endpoint OpenAI-compatible, como OpenRouter.');
  const id = text(form.id), baseUrl = text(form.baseUrl).replace(/\/+$/, '');
  if (!/^[A-Za-z0-9_-]{1,64}$/.test(id)) fail('Informe um ID de provedor válido (letras, números, _ ou -).');
  let url;
  try { url = new URL(baseUrl); } catch { fail('Informe uma baseURL HTTPS válida, sem credenciais, parâmetros ou fragmentos.'); }
  if (url.protocol !== 'https:' || !url.hostname || url.username || url.password || url.search || url.hash) fail('Informe uma baseURL HTTPS válida, sem credenciais, parâmetros ou fragmentos.');
  if (url.hostname.toLowerCase() === 'api.anthropic.com' || /\/messages\/?$/.test(url.pathname)) fail('Anthropic Messages não é suportada. Use um endpoint OpenAI-compatible, como OpenRouter.');
  const models = [...new Set(text(form.models).split(/[\n,]/).map(v => v.trim()))];
  if (!models.length || models.length > 128 || models.some(m => !validModel(m))) fail('Informe os IDs reais dos modelos, conforme o catálogo do provedor.');
  const payload = writableConfig(config);
  if (editId && (editId !== id || !payload.providers.some(p => p.id === editId))) fail('O ID do provedor em edição deve ser preservado.');
  if (!editId && (payload.providers.length >= 16 || payload.providers.some(p => p.id === id))) fail('ID já utilizado ou limite de provedores atingido.');
  const contextWindow = integer(form.contextWindow, 1024, 262144), maxOutput = integer(form.maxOutput, 256, 8192);
  if (maxOutput >= contextWindow || !['omit', 'openai', 'openrouter'].includes(form.reasoningFormat)) fail('Limites inválidos. Confira os intervalos indicados.');
  const provider = { id, baseUrl, models, reasoningFormat: form.reasoningFormat, contextWindow, maxOutput, allowLoopback: false };
  const key = text(form.apiKey);
  if (key && (key.length > 4096 || /[^\x20-\x7e]/.test(key))) fail('Chave de API inválida.');
  if (form.clearKey && key) fail('Escolha entre substituir ou remover a chave.');
  if (key) provider.apiKey = key;
  else if (form.clearKey) provider.apiKey = null;
  payload.providers = editId ? payload.providers.map(p => p.id === editId ? provider : p) : [...payload.providers, provider];
  if (form.makeDefault) payload.default = id;
  else if (payload.default === editId) payload.default = null;
  validateRouteMappings({ aliases: payload.aliases, fallbacks: payload.fallbacks }, payload.providers);
  return payload;
}
export function removeProviderPayload(config, id) {
  const payload = writableConfig(config);
  payload.providers = payload.providers.filter(p => p.id !== id);
  if (payload.default === id) payload.default = null;
  for (const kind of ['aliases', 'fallbacks']) payload[kind] = Object.fromEntries(Object.entries(payload[kind]).filter(([, target]) => target.provider !== id));
  return payload;
}
export function buildRoutesPayload(config, json) {
  if (typeof json !== 'string' || json.length > 8192) fail('Mapeamento de rotas inválido.');
  let mappings;
  try { mappings = JSON.parse(json); } catch { fail('Mapeamento de rotas inválido.'); }
  const payload = writableConfig(config);
  return { ...payload, ...validateRouteMappings(mappings, payload.providers) };
}
export function policyForm(policy = {}) {
  return { ...emptyPolicyForm(), mode: policy.mode || 'deferred', ...(policy.mode === 'api' ? policy.api : {}) };
}
export function buildPolicyPayload(form, config) {
  if (!['deferred', 'local', 'api'].includes(form.mode)) fail('Política de memória inválida.');
  if (form.mode !== 'api') return { mode: form.mode };
  const provider = text(form.provider), model = text(form.model);
  if (!provider || !model || !config.providers.some(p => p.id === provider && p.models.includes(model))) fail('Escolha explicitamente um provedor e um modelo configurados.');
  if (form.historyConsent !== true) fail('Autorize explicitamente o envio de histórico para processar a memória via API.');
  return { mode: 'api', api: { provider, model, historyConsent: true,
    maxTokens: integer(form.maxTokens ?? MEMORY_BUDGETS.maxTokens, 256, 2048),
    maxInputTokens: integer(form.maxInputTokens ?? MEMORY_BUDGETS.maxInputTokens, 1024, 16384),
    timeoutSeconds: integer(form.timeoutSeconds ?? MEMORY_BUDGETS.timeoutSeconds, 5, 120) } };
}
export const shouldShowServerSetup = ({ manual = false, needsServer, unpaired, embeddedReady }) => manual || (!embeddedReady && (needsServer || unpaired));
