import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs/promises';
import { createApiTransport, publicPresets } from '../client/src/api.js';
import { emptyProviderForm, emptyPolicyForm, redactProviderConfig, providerForm, policyForm,
  providerTemplate, redactDiscovery, buildProviderPayload, removeProviderPayload, buildRoutesPayload, buildPolicyPayload, shouldShowServerSetup } from '../client/src/mobile-settings.js';
import { watchProviderCatalog, PROVIDER_CATALOG_CHANGED } from '../client/src/provider-catalog.js';
import { downloadProgress, statusPollDelay, startStatusPolling } from '../client/src/autohost-status.js';
import { createLatestRequest, createSharedListener, startReconnectionWait, waitForOperation } from '../client/src/connection-lifetime.js';

const provider = { id: 'direct', baseUrl: 'https://api.example.com/v1', models: ['catalog-model'], hasApiKey: true, reasoningFormat: 'omit', contextWindow: 8192, maxOutput: 2048 };
const config = () => ({ default: 'direct', providers: [{ ...provider, models: [...provider.models] }], aliases: { 'pc-route': { provider: 'direct', model: 'catalog-model' } }, fallbacks: {} });
const form = patch => ({ ...emptyProviderForm(), id: 'second', baseUrl: 'https://second.example.com/v1', models: 'real-model', ...patch });
const flush = async () => { await Promise.resolve(); await Promise.resolve(); };
const event = (type, detail = {}) => Object.assign(new Event(type), { detail });

test('providers: GET redigido e formulário de edição nunca pré-carregam chaves', () => {
  const raw = config(); raw.providers[0].apiKey = 'secret-not-for-UI';
  const safe = redactProviderConfig(raw), edit = providerForm(raw.providers[0], safe);
  assert.ok(!JSON.stringify(safe).includes('secret-not-for-UI'));
  assert.equal(edit.apiKey, '');
  assert.equal(edit.clearKey, false);
  assert.equal(raw.providers[0].apiKey, 'secret-not-for-UI');
});

test('providers: apenas chave digitada entra no PUT; omission/null preservam contrato 3R', () => {
  const raw = config(); raw.providers[0].apiKey = 'old-secret';
  const created = buildProviderPayload(raw, form({ apiKey: 'new-secret' }));
  assert.equal(created.providers[1].apiKey, 'new-secret');
  assert.ok(!Object.hasOwn(created.providers[0], 'apiKey'));
  assert.ok(!Object.hasOwn(created.providers[0], 'hasApiKey'));
  const edit = providerForm(raw.providers[0], raw);
  assert.ok(!Object.hasOwn(buildProviderPayload(raw, edit, 'direct').providers[0], 'apiKey'));
  const changed = buildProviderPayload(raw, { ...edit, baseUrl: 'https://changed.example.com/v1' }, 'direct');
  assert.ok(!Object.hasOwn(changed.providers[0], 'apiKey')); // Backend drops omission on a changed endpoint.
  assert.equal(buildProviderPayload(raw, { ...edit, clearKey: true }, 'direct').providers[0].apiKey, null);
  assert.throws(() => buildProviderPayload(raw, { ...edit, clearKey: true, apiKey: 'new' }, 'direct'));
});

test('providers: campos incompletos, HTTP sem opt-in, URL com secrets e Anthropic Messages são recusados', () => {
  for (const patch of [{ id: '' }, { models: '' }, { models: 'model,\n' }, { baseUrl: '' }, { baseUrl: 'http://api.example.com/v1' },
    { baseUrl: 'https://user:secret@api.example.com/v1' }, { baseUrl: 'https://api.example.com/v1?key=secret' },
    { baseUrl: 'https://api.example.com/v1#key' }, { protocol: 'anthropic-messages' }, { baseUrl: 'https://api.anthropic.com/v1' },
    { baseUrl: 'https://api.example.com/v1/messages' }, { apiKey: 'bad\nkey' }]) {
    assert.throws(() => buildProviderPayload(config(), form(patch)), JSON.stringify(Object.keys(patch)));
  }
});

test('4B providers: loopback literal/exato exige opt-in booleano, inclusive HTTPS; remotos HTTP/spoofs são recusados', () => {
  assert.equal(emptyProviderForm().allowLoopback, false);
  for (const host of ['127.0.0.1', '127.23.4.5', '127.255.255.255', 'localhost', 'LOCALHOST', '[::1]', '[0:0:0:0:0:0:0:1]',
    '[::ffff:127.0.0.1]', '[::ffff:127.255.255.254]', '[::ffff:7f00:1]']) {
    for (const protocol of ['http', 'https']) {
      const baseUrl = `${protocol}://${host}:20128/v1`;
      assert.equal(buildProviderPayload(config(), form({ baseUrl, allowLoopback: true })).providers[1].allowLoopback, true, baseUrl);
      for (const allowLoopback of [false, undefined, 'true']) assert.throws(() => buildProviderPayload(config(), form({ baseUrl, allowLoopback })), baseUrl);
    }
  }
  for (const host of ['api.example.com', '192.168.1.10', '0.0.0.0', '128.0.0.1', 'localhost.evil.test', 'evil-localhost.test',
    'evillocalhost', 'localhost.', 'sub.localhost', '%6cocalhost', '127.0.0.1.evil.test', '[::]', '[::2]', '[::ffff:192.168.1.1]', '0x7f000001', '2130706433', '127.1']) {
    assert.throws(() => buildProviderPayload(config(), form({ baseUrl: `http://${host}:20128/v1`, allowLoopback: true })), host);
  }
  assert.equal(buildProviderPayload(config(), form({ allowLoopback: true })).providers[1].baseUrl, 'https://second.example.com/v1');
});

test('4B providers: rejeita normalização perigosa raw, userinfo vazio, query/hash vazio e caminho duplicado', () => {
  for (const baseUrl of ['https://@api.example.com/v1', 'https://api.example.com/v1?', 'https://api.example.com/v1#',
    'https://api.example.com\\v1', 'https:\\api.example.com/v1', 'https://api.ex\nample.com/v1',
    'https://api.example.com/v1\t', '\nhttps://api.example.com/v1', 'https://api.example.com/\u007fv1',
    'https://api.example.com/v1/chat/completions', 'https://api.example.com/chat/completions/v1/',
    'https://api.example.com/v1/chat/completions/chat/completions', 'https://api.example.com/v1/%63hat/completions',
    'https://api.example.com/v1/%6dessages', 'https://api.anthropic.com./v1']) {
    assert.throws(() => buildProviderPayload(config(), form({ baseUrl, allowLoopback: true })), baseUrl);
  }
  for (const host of ['%6cocalhost', '0x7f000001', '2130706433', '127.1']) {
    assert.throws(() => buildProviderPayload(config(), form({ baseUrl: `https://${host}:20128/v1`, allowLoopback: true })), host);
  }
  assert.deepEqual(buildProviderPayload(config(), form({ models: 'real-model\nreal-model,second-real' })).providers[1].models, ['real-model', 'second-real']);
});

test('4B providers: edição mantém opt-in e senha omitida/null, atalhos sem modelo/chave/default/consentimento', () => {
  const raw = config();
  raw.providers[0] = { ...raw.providers[0], baseUrl: 'http://127.0.0.1:20128/v1', allowLoopback: true, apiKey: 'server-only' };
  const edit = providerForm(raw.providers[0], raw);
  assert.equal(edit.allowLoopback, true); assert.equal(edit.apiKey, '');
  const payload = buildProviderPayload(raw, edit, 'direct');
  assert.equal(payload.providers[0].allowLoopback, true);
  assert.ok(!Object.hasOwn(payload.providers[0], 'apiKey'));
  assert.equal(buildProviderPayload(raw, { ...edit, clearKey: true }, 'direct').providers[0].apiKey, null);
  for (const kind of ['9router', 'zen']) {
    const draft = providerTemplate(kind);
    assert.equal(draft.apiKey, ''); assert.equal(draft.models, ''); assert.equal(draft.makeDefault, false);
    assert.equal(draft.allowLoopback, false); assert.ok(!Object.hasOwn(draft, 'historyConsent'));
    assert.throws(() => buildProviderPayload(config(), draft));
  }
  assert.equal(providerTemplate('9router').baseUrl, 'http://127.0.0.1:20128/v1');
  assert.equal(providerTemplate('zen').baseUrl, 'https://opencode.ai/zen/v1');
  assert.equal(raw.providers[0].apiKey, 'server-only');
});

test('4B discovery: somente campos públicos; nenhum snapshot entra em PUT de provider, remoção ou rotas', () => {
  const raw = config(); raw.discovery = { checkedAt: '2026-10-16T15:00:00Z', ttlSeconds: 30, apiKey: 'secret',
    nineRouter: { status: 'available', models: ['catalog-model'], apiKey: 'secret', message: 'secret' },
    openCode: { status: 'detected_native_api', models: [], token: 'secret' } };
  const safe = redactDiscovery(raw.discovery);
  assert.equal(safe.nineRouter.status, 'available'); assert.equal(safe.openCode.status, 'detected_native_api');
  assert.equal(safe.ttlSeconds, 30); assert.deepEqual(safe.nineRouter.models, ['catalog-model']);
  assert.ok(!JSON.stringify(safe).includes('secret'));
  for (const value of [redactProviderConfig(raw), providerForm(raw.providers[0], raw), buildProviderPayload(raw, form()),
    removeProviderPayload(raw, 'direct'), buildRoutesPayload(raw, '{}')]) assert.ok(!Object.hasOwn(value, 'discovery'));
  for (const status of ['auth_required', 'absent', 'incompatible']) assert.equal(redactDiscovery({ nineRouter: { status } }).nineRouter.status, status);
  assert.equal(redactDiscovery({ nineRouter: { status: 'secret' } }).nineRouter.status, 'incompatible');
});

test('4B discovery: GET fixa embedded/auth/signal, sem fallback para PC, instalação ou activate', async () => {
  const source = await fs.readFile(new URL('../client/src/api.js', import.meta.url), 'utf8');
  assert.match(source, /localDiscovery: \(options\) => request\('GET', '\/api\/local\/discovery', undefined, embeddedOptions\(options\)\)/);
  assert.match(source, /const embeddedOptions = \(options = \{\}\) => \(\{ signal: options.signal, embedded: true \}\)/);
  const ac = new AbortController(), acquired = [], sent = [];
  let released = 0;
  const transport = createApiTransport({ acquire: async options => {
    acquired.push(options); return { server: 'http://127.0.0.1:3011', token: 'embedded-only', kind: 'local', release() { released++; } };
  }, fetch: async (url, options) => { sent.push({ url, options }); return Response.json({ nineRouter: { status: 'absent' } }); } });
  await transport.request('GET', '/api/local/discovery', undefined, { signal: ac.signal, embedded: true });
  assert.equal(acquired.length, 1); assert.equal(acquired[0].embedded, true);
  assert.equal(sent[0].url, 'http://127.0.0.1:3011/api/local/discovery');
  assert.equal(sent[0].options.method, 'GET'); assert.equal(sent[0].options.body, undefined);
  assert.equal(sent[0].options.headers.Authorization, 'Bearer embedded-only');
  assert.equal(sent[0].options.signal.aborted, false); assert.equal(released, 1);
  const failed = createApiTransport({ acquire: async () => { acquired.push({}); return { server: 'http://127.0.0.1:3011', kind: 'local', release() {} }; },
    failed() {}, fetch: async () => { throw Error('offline'); } });
  await assert.rejects(failed.request('GET', '/api/local/discovery', undefined, { embedded: true }));
  assert.equal(acquired.length, 2, 'failed embedded GET is not replayed elsewhere');
  assert.equal(publicPresets({ presets: [{ id: 'mobile-api', routes: [{ route: 'mobile-api:9router-local:catalog-model' }] }] }).presets.length, 1);
});

test('providers: edição preserva identidade e rotas; defaults e limites exigem valores válidos', () => {
  const original = config();
  const payload = buildProviderPayload(original, form({ makeDefault: true }));
  assert.equal(payload.default, 'second'); assert.deepEqual(payload.aliases, original.aliases);
  assert.throws(() => buildProviderPayload(original, form({ id: 'direct' })));
  assert.throws(() => buildProviderPayload(original, form(), 'direct'));
  assert.throws(() => buildProviderPayload(original, form({ contextWindow: 1024, maxOutput: 1024 })));
  assert.throws(() => buildProviderPayload(original, form({ maxOutput: 8193 })));
  assert.throws(() => buildProviderPayload(original, { ...providerForm(provider, original), models: 'different-model' }, 'direct'));
});

test('providers: remover apaga só o provedor escolhido e seus mapeamentos explícitos', () => {
  const original = config(); original.providers.push({ ...provider, id: 'other' });
  original.aliases.other = { provider: 'other', model: 'catalog-model' };
  const payload = removeProviderPayload(original, 'direct');
  assert.equal(payload.default, null); assert.equal(payload.providers.length, 1);
  assert.deepEqual(payload.aliases, { other: original.aliases.other });
  assert.equal(original.providers.length, 2);
});

test('providers: JSON avançado só aceita aliases/fallbacks com destinos reais e sem secrets', () => {
  const original = config();
  assert.deepEqual(buildRoutesPayload(original, JSON.stringify({ aliases: original.aliases })).aliases, original.aliases);
  for (const json of ['{', '{"apiKey":"secret"}', '{"providers":[]}', '{"aliases":{"pc":{"provider":"direct","model":"invented"}}}',
    '{"aliases":{"pc":{"provider":"direct","model":"catalog-model","apiKey":"secret"}}}',
    '{"aliases":{"mobile-local":{"provider":"direct","model":"catalog-model"}}}']) assert.throws(() => buildRoutesPayload(original, json));
  assert.throws(() => buildRoutesPayload(original, ' '.repeat(8193)));
});

test('policy: deferred é padrão seguro; local/deferred nunca levam API nem consentimento residual', () => {
  const initial = emptyPolicyForm();
  assert.equal(initial.historyConsent, false);
  assert.equal(initial.provider, ''); assert.equal(initial.model, '');
  assert.deepEqual(buildPolicyPayload(initial, config()), { mode: 'deferred' });
  assert.deepEqual(buildPolicyPayload({ ...initial, mode: 'local', provider: 'direct', historyConsent: true }, config()), { mode: 'local' });
  assert.deepEqual(policyForm(), initial);
});

test('policy: API exige escolha de provedor/modelo e consentimento, com budgets 1200/6000/60', () => {
  const api = { ...emptyPolicyForm(), mode: 'api', provider: 'direct', model: 'catalog-model', historyConsent: true };
  assert.deepEqual(buildPolicyPayload(api, config()), { mode: 'api', api: { provider: 'direct', model: 'catalog-model', historyConsent: true, maxTokens: 1200, maxInputTokens: 6000, timeoutSeconds: 60 } });
  for (const patch of [{ provider: '' }, { model: '' }, { model: 'invented' }, { historyConsent: false }, { historyConsent: 'true' },
    { maxTokens: 255 }, { maxTokens: 2049 }, { maxInputTokens: 1023 }, { maxInputTokens: 16385 }, { timeoutSeconds: 4 }, { timeoutSeconds: 121 }, { timeoutSeconds: '' }, { timeoutSeconds: 60.5 }]) assert.throws(() => buildPolicyPayload({ ...api, ...patch }, config()));
});

test('setup: standalone ignora UNPAIRED PC; abertura manual não depende de needsServer', () => {
  assert.equal(shouldShowServerSetup({ embeddedReady: true, unpaired: true, needsServer: true }), false);
  assert.equal(shouldShowServerSetup({ embeddedReady: true, manual: true }), true);
  assert.equal(shouldShowServerSetup({ embeddedReady: false, unpaired: true }), true);
  assert.equal(shouldShowServerSetup({ embeddedReady: false, unpaired: false, needsServer: false }), false);
});

test('UI wiring: trocar/cancelar PC não apaga configuração/token e formulário zera chave após save', async () => {
  const source = async file => fs.readFile(new URL(`../client/src/${file}`, import.meta.url), 'utf8');
  const app = await source('App.jsx'), setup = await source('components/ServerSetup.jsx'), settings = await source('components/LocalHostSettings.jsx');
  const reset = app.slice(app.indexOf('onResetServer='));
  assert.ok(reset.includes('setManualServerSetup(true)'));
  assert.doesNotMatch(reset, /config\.(server|token)\s*=/);
  assert.ok(app.includes('onCancel={manualServerSetup ? closeTopLayer : undefined}'));
  assert.ok(setup.includes('onClick={onCancel || onDone}'));
  assert.ok(setup.includes('rollback?.()'));
  assert.match(settings, /value=\{form\.apiKey\} type="password"/);
  assert.ok(settings.includes('setForm(emptyProviderForm()); setEditId(null); setFormOpen(false)'));
  assert.doesNotMatch(settings, /localStorage|console\.|JSON\.stringify\(form/);
  assert.ok(app.includes('hostStatus={!openId && !globalOpen ? hostStatus : null}'));
  const view = await source('components/ChatView.jsx');
  assert.match(view, /showHostStatus = hostStatus && !drawer/);
});

test('download: contrato phase/downloaded/total, compatibilidade bytes e percentual sem RAM estimada', () => {
  assert.deepEqual(downloadProgress({ download: { phase: 'downloading', downloaded: 25, total: 100, error: null } }), { phase: 'downloading', downloaded: 25, total: 100, percent: 25, error: '' });
  assert.equal(downloadProgress({ download: { state: 'ready', downloadedBytes: 200, totalBytes: 100 } }).percent, 100);
  assert.equal(downloadProgress({}).percent, null);
  assert.equal(statusPollDelay({ activated: true, embed: { download: { phase: 'verifying' } } }), 2000);
  assert.equal(statusPollDelay({ activated: false, chat: { download: { phase: 'pending' } } }), 10000);
  assert.equal(statusPollDelay({}, true), 2000);
});

function pollingFixture(getStatus, enabled = () => true) {
  const document = new EventTarget(), window = new EventTarget(), scheduled = new Map(), received = [], errors = [], calls = [];
  document.visibilityState = 'visible';
  let seq = 0;
  const options = { document, window, enabled, getStatus: options => { calls.push(options); return getStatus(options); },
    onStatus: value => received.push(value), onError: error => errors.push(error),
    timers: { setTimeout(fn, ms) { scheduled.set(++seq, { fn, ms }); return seq; }, clearTimeout(id) { scheduled.delete(id); } } };
  return { document, window, scheduled, received, errors, calls, options, start: () => startStatusPolling(options) };
}

test('polling: download 2s, idle 10s, hidden aborta/pausa e resume consulta sem activate', async () => {
  let status = { activated: true, chat: { download: { phase: 'downloading' } } };
  const f = pollingFixture(async () => status), watcher = f.start();
  await flush(); assert.equal([...f.scheduled.values()][0].ms, 2000);
  f.document.visibilityState = 'hidden'; f.document.dispatchEvent(event('visibilitychange'));
  assert.equal(f.calls[0].signal.aborted, true); assert.equal(f.scheduled.size, 0); assert.equal(f.calls.length, 1);
  status = { activated: false };
  f.document.visibilityState = 'visible'; f.document.dispatchEvent(event('visibilitychange'));
  await flush(); assert.equal(f.calls.length, 2); assert.equal([...f.scheduled.values()][0].ms, 10000);
  watcher.stop(); assert.equal(f.scheduled.size, 0);
});

test('polling: eventos 6A SYNCING/SYNCED e connection-change reconsultam destino fixo', async () => {
  const f = pollingFixture(async () => ({ activated: false })), watcher = f.start();
  await flush();
  f.window.dispatchEvent(event('mc:connection-health', { code: 'SYNCING' }));
  await flush(); assert.equal([...f.scheduled.values()][0].ms, 2000);
  f.window.dispatchEvent(event('mc:connection-health', { code: 'SYNCED' }));
  await flush(); assert.equal([...f.scheduled.values()][0].ms, 10000);
  f.window.dispatchEvent(event('mc:connection-change', { kind: 'pc' }));
  await flush(); assert.equal(f.calls.length, 4);
  watcher.stop();
});

test('polling: desmontagem/StrictMode descarta resposta tardia e remove listeners/timers', async () => {
  let finish;
  const f = pollingFixture(() => new Promise(resolve => { finish = resolve; }));
  const first = f.start(); first.stop();
  finish({ activated: true }); await flush();
  assert.equal(f.received.length, 0); assert.equal(f.scheduled.size, 0);
  const second = f.start(); finish({ activated: false }); await flush(); second.stop();
  f.window.dispatchEvent(event('mc:resume')); f.document.dispatchEvent(event('visibilitychange'));
  assert.equal(f.calls.length, 2); assert.equal(f.received.length, 1); assert.equal(f.scheduled.size, 0);
});

test('polling: web/embedded indisponível não consulta; falha GET não ativa nem repete writes', async () => {
  const disabled = pollingFixture(async () => ({}), () => false), inactive = disabled.start();
  await flush(); assert.equal(disabled.calls.length, 0); inactive.stop();
  const f = pollingFixture(async () => { throw Error('network'); }), watcher = f.start();
  await flush(); assert.equal(f.errors.length, 1); assert.equal(f.calls.length, 1); assert.equal([...f.scheduled.values()][0].ms, 10000);
  watcher.stop();
});

test('transport: ativação local recusa troca para PC/browser antes de despachar POST', async () => {
  let sent = 0, released = 0;
  const transport = createApiTransport({ acquire: async () => ({ server: 'http://127.0.0.1:3011', token: 'local', kind: 'local', release() { released++; } }),
    fetch: async () => { sent++; return Response.json({}); } });
  await assert.rejects(transport.request('POST', '/api/local/activate', undefined, { embedded: true, localOnly: true }), error => error.code === 'HOST_CHANGED');
  assert.equal(sent, 0); assert.equal(released, 1);
});

test('i18n: novas strings estáticas de AutoHost e validação possuem inglês no mapa existente', async () => {
  const dictionary = JSON.parse(await fs.readFile(new URL('../client/src/locales/en.json', import.meta.url), 'utf8'));
  for (const file of ['components/AutoHostStatus.jsx', 'components/LocalHostSettings.jsx', 'mobile-settings.js']) {
    const source = await fs.readFile(new URL(`../client/src/${file}`, import.meta.url), 'utf8');
    for (const match of source.matchAll(/(?:tr|fail)\('([^']+)'/g)) assert.ok(dictionary[match[1]], `${file}: ${match[1]}`);
  }
});

const deferred = () => { let resolve, reject; const promise = new Promise((a, b) => { resolve = a; reject = b; }); return { promise, resolve, reject }; };
const drain = async () => { for (let i = 0; i < 24; i++) await Promise.resolve(); };

test('4B catálogo: refresh por evento, resposta anterior e unmount ignorados, sem mudar rota/geração', async () => {
  const window = new EventTarget(), requests = [], received = [], cache = deferred();
  let destination = 'local:1';
  const stop = watchProviderCatalog({ window, snapshot: () => destination, getCached: () => cache.promise,
    getPresets: options => { const gate = deferred(); requests.push({ ...gate, ...options }); return gate.promise; },
    onPresets: value => received.push(value) });
  await drain(); assert.equal(requests.length, 1);
  const initial = { presets: [{ id: 'mobile-local' }] };
  requests[0].resolve(initial); await drain();
  assert.deepEqual(received, [initial]);
  cache.resolve({ presets: [{ id: 'stale-cache' }] }); await drain();
  assert.deepEqual(received, [initial]);
  window.dispatchEvent(new Event(PROVIDER_CATALOG_CHANGED)); await drain();
  assert.equal(requests.length, 2);
  window.dispatchEvent(new Event(PROVIDER_CATALOG_CHANGED)); await drain();
  assert.equal(requests[1].signal.aborted, true);
  const updated = { presets: [{ id: 'mobile-api', routes: [{ route: 'mobile-api:9router-local:real' }] }] };
  requests[2].resolve(updated); requests[1].resolve({ presets: [] }); await drain();
  assert.deepEqual(received, [initial, updated]);
  destination = 'pc:2'; window.dispatchEvent(new Event('mc:connection-change')); await drain();
  assert.equal(requests.length, 4);
  stop(); assert.equal(requests[3].signal.aborted, true);
  requests[3].resolve({ presets: [] }); await drain();
  window.dispatchEvent(new Event(PROVIDER_CATALOG_CHANGED)); window.dispatchEvent(new Event('mc:connection-change')); await drain();
  assert.equal(requests.length, 4); assert.deepEqual(received, [initial, updated]);
});

test('4B UI: descoberta isolada e cancelável, atalhos preservam provider manual e consentimento', async () => {
  const source = await fs.readFile(new URL('../client/src/components/LocalHostSettings.jsx', import.meta.url), 'utf8');
  assert.match(source, /setDiscovery\(redactDiscovery\(p.discovery\)\)/);
  assert.match(source, /api.localDiscovery\(\{ signal \}\)/);
  assert.match(source, /!lifetimeSignal.aborted && isCurrent\(\)/);
  assert.match(source, /ac.abort\(\); reads.cancel\(\)/);
  assert.match(source, /existing \? providerForm\(existing, providers\) : draft/);
  assert.doesNotMatch(source, /setInterval|setTimeout|localActivate|switchTo|saveChat|api.presets/);
  const template = source.slice(source.indexOf('function openTemplate'), source.indexOf('function saveProvider'));
  assert.doesNotMatch(template, /saveLocal|patchPolicy|setPolicy/);
});

function recoveryFixture(overrides = {}) {
  const document = new EventTarget(), window = new EventTarget(), jobs = new Map(), progress = [], errors = [], signals = [];
  document.visibilityState = 'visible';
  let sequence = 0, clock = 0, probes = 0, promotions = 0, closes = 0;
  const timers = { setTimeout(fn, ms) { jobs.set(++sequence, { fn, ms }); return sequence; }, clearTimeout(id) { jobs.delete(id); } };
  const options = { document, window, timers, now: () => clock, durationMs: 10000,
    probe: async signal => { probes++; signals.push(signal); return overrides.probe ? overrides.probe(signal) : true; },
    reconnect: async signal => { promotions++; signals.push(signal); return overrides.reconnect ? overrides.reconnect(signal) : { kind: 'pc' }; },
    onProgress: value => progress.push(value), onError: error => errors.push(error), onClose: () => closes++ };
  const watcher = startReconnectionWait(options);
  return { document, window, jobs, progress, errors, signals, watcher, timers,
    counts: () => ({ probes, promotions, closes }), advance: ms => { clock += ms; },
    async run(ms) { const entry = [...jobs].find(([, job]) => job.ms === ms); assert.ok(entry, `timer ${ms} exists`); jobs.delete(entry[0]); await entry[1].fn(); await drain(); } };
}

test('lote3a UI: health aguarda promoção/sync; sucesso não agenda probe extra e close é cancelável', async () => {
  const gate = deferred(), f = recoveryFixture({ reconnect: () => gate.promise });
  const checking = f.run(1500); await drain();
  assert.equal(f.counts().promotions, 1);
  assert.ok(!f.progress.some(p => p.reconnected));
  gate.resolve({ kind: 'pc' }); await checking;
  assert.ok(f.progress.some(p => p.reconnected));
  assert.deepEqual([...f.jobs.values()].map(j => j.ms), [800]);
  f.watcher.stop();
  assert.equal(f.jobs.size, 0); assert.equal(f.counts().closes, 0);
});

test('lote3a UI: sync falho ou destino local informa erro sem sucesso/activate em loop', async () => {
  for (const reconnect of [async () => { throw Error('Conflito de sincronização'); }, async () => ({ kind: 'local' })]) {
    const f = recoveryFixture({ reconnect });
    await f.run(1500);
    assert.equal(f.errors.length, 1); assert.equal(f.counts().promotions, 1);
    assert.equal(f.jobs.size, 0); assert.equal(f.counts().closes, 0);
    assert.ok(!f.progress.some(p => p.reconnected));
    f.window.dispatchEvent(new Event('mc:resume')); await drain();
    assert.equal(f.counts().promotions, 1); assert.equal(f.jobs.size, 0);
    f.watcher.stop();
  }
});

test('lote3a UI: hidden aborta probe, descarta resposta tardia e resume faz uma tentativa', async () => {
  const gate = deferred(), f = recoveryFixture({ probe: () => gate.promise });
  const checking = f.run(1500); await drain();
  f.document.visibilityState = 'hidden'; f.document.dispatchEvent(new Event('visibilitychange'));
  assert.equal(f.signals[0].aborted, true); assert.equal(f.jobs.size, 0);
  gate.resolve(true); await checking;
  assert.equal(f.counts().promotions, 0); assert.equal(f.jobs.size, 0);
  f.document.visibilityState = 'visible'; f.document.dispatchEvent(new Event('visibilitychange'));
  await f.run(1500);
  assert.equal(f.counts().probes, 2); assert.equal(f.counts().promotions, 1);
  await f.run(800); assert.equal(f.counts().closes, 1);
  f.watcher.stop();
});

test('lote3a UI: hidden/unmount durante promoção aborta consumidor e não anuncia sucesso tardio', async () => {
  for (const unmount of [false, true]) {
    const gate = deferred(), f = recoveryFixture({ reconnect: () => gate.promise });
    const checking = f.run(1500); await drain();
    if (unmount) f.watcher.stop();
    else { f.document.visibilityState = 'hidden'; f.document.dispatchEvent(new Event('visibilitychange')); }
    assert.equal(f.signals.at(-1).aborted, true);
    gate.resolve({ kind: 'pc' }); await checking;
    assert.ok(!f.progress.some(p => p.reconnected)); assert.equal(f.jobs.size, 0); assert.equal(f.counts().closes, 0);
    f.watcher.stop();
    f.window.dispatchEvent(new Event('mc:resume')); f.document.dispatchEvent(new Event('visibilitychange'));
    assert.equal(f.jobs.size, 0);
  }
});

test('lote3a UI: countdown termina em zero sem sondar; tempo hidden conta para deadline', async () => {
  for (const hidden of [false, true]) {
    const f = recoveryFixture({ probe: async () => false });
    if (hidden) { f.document.visibilityState = 'hidden'; f.document.dispatchEvent(new Event('visibilitychange')); }
    f.advance(10000);
    if (hidden) { f.document.visibilityState = 'visible'; f.document.dispatchEvent(new Event('visibilitychange')); }
    else await f.run(1000);
    assert.equal(f.progress.at(-1).timeLeft ?? f.progress.at(-2).timeLeft, 0);
    assert.equal(f.jobs.size, 0); assert.equal(f.counts().probes, 0);
    f.watcher.stop();
  }
});

test('lote3a UI: limpar modal após sucesso cancela os 800ms e remove listeners', async () => {
  const f = recoveryFixture(); await f.run(1500);
  f.watcher.stop(); f.watcher.stop();
  f.window.dispatchEvent(new Event('mc:resume')); f.document.dispatchEvent(new Event('visibilitychange'));
  assert.equal(f.jobs.size, 0); assert.equal(f.counts().closes, 0); assert.equal(f.counts().probes, 1);
});

test('lote3a UI: lista descarta host/época/query antigos, compartilha refresh e aborta unmount', async () => {
  let destination = 'pc:epoch1', calls = 0;
  const loader = createLatestRequest(() => destination), first = deferred(), second = deferred(), committed = [], signals = [];
  const read = gate => async (signal, current) => { calls++; signals.push(signal); const data = await gate.promise; if (current()) committed.push(data); };
  const a = loader.run('query:a', read(first)), shared = loader.run('query:a', read(first));
  assert.equal(a, shared); await drain(); assert.equal(calls, 1);
  destination = 'android:epoch2';
  const b = loader.run('query:a', read(second)); await drain();
  assert.equal(signals[0].aborted, true);
  second.resolve('Android'); await b; first.resolve('PC'); await a;
  assert.deepEqual(committed, ['Android']);
  const query = deferred(), pending = loader.run('query:old', read(query)); await drain();
  await loader.run('query:new', read({ promise: Promise.resolve('Busca atual') }));
  query.resolve('Busca antiga'); await pending;
  assert.deepEqual(committed, ['Android', 'Busca atual']);
  const staleEpoch = deferred(), stale = loader.run('query:new', read(staleEpoch)); await drain();
  destination = 'android:epoch3'; staleEpoch.resolve('Época antiga'); await stale;
  assert.deepEqual(committed, ['Android', 'Busca atual']);
  const abandoned = deferred(), waiting = loader.run('query:new', read(abandoned)); await drain();
  loader.cancel(); assert.equal(signals.at(-1).aborted, true);
  abandoned.resolve('Unmount'); await waiting;
  assert.deepEqual(committed, ['Android', 'Busca atual']);
});

test('lote3a UI: StrictMode compartilha registro Back pendente e cleanup remove uma vez', async () => {
  const gate = deferred(); let installs = 0, removes = 0;
  const subscribe = createSharedListener(() => { installs++; return gate.promise; });
  const first = subscribe(); first(); const second = subscribe();
  await drain(); assert.equal(installs, 1);
  gate.resolve({ remove: () => removes++ }); await drain();
  assert.equal(removes, 0); second(); second(); await drain(); assert.equal(removes, 1);
  const third = subscribe(); await drain(); assert.equal(installs, 2);
  third(); await drain(); assert.equal(removes, 2);
});

test('lote3a UI: espera limitada limpa timer e listener no resolve, reject e abort', async () => {
  for (const outcome of ['resolve', 'reject', 'abort']) {
    const gate = deferred(), ac = new AbortController(), timers = new Map(); let adds = 0, removes = 0;
    const add = ac.signal.addEventListener.bind(ac.signal), remove = ac.signal.removeEventListener.bind(ac.signal);
    ac.signal.addEventListener = (...args) => { adds++; add(...args); };
    ac.signal.removeEventListener = (...args) => { removes++; remove(...args); };
    const waiting = waitForOperation(gate.promise, { signal: ac.signal, timeoutMs: 5000,
      timers: { setTimeout(fn) { timers.set(1, fn); return 1; }, clearTimeout(id) { timers.delete(id); } } });
    if (outcome === 'resolve') { gate.resolve('ready'); assert.equal(await waiting, 'ready'); }
    else { if (outcome === 'abort') ac.abort(); else gate.reject(Error('JNI')); await assert.rejects(waiting); }
    assert.equal(timers.size, 0); assert.equal(adds, removes);
  }
});

test('lote3a UI: falha de seleção antes do SSE não invalida epoch nem dispara perda de PC', async () => {
  let failures = 0, sent = 0;
  const transport = createApiTransport({ acquire: async () => { throw Object.assign(Error('Activate recusado'), { code: 'ACTIVATE_FAILED' }); },
    failed: () => failures++, fetch: async () => { sent++; return Response.json({}); } });
  await assert.rejects(transport.stream('GET', '/api/events', undefined, () => {}), { code: 'ACTIVATE_FAILED' });
  assert.equal(failures, 0); assert.equal(sent, 0);
});
