import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs/promises';
import { createApiTransport } from '../client/src/api.js';
import { emptyProviderForm, emptyPolicyForm, redactProviderConfig, providerForm, policyForm,
  buildProviderPayload, removeProviderPayload, buildRoutesPayload, buildPolicyPayload, shouldShowServerSetup } from '../client/src/mobile-settings.js';
import { downloadProgress, statusPollDelay, startStatusPolling } from '../client/src/autohost-status.js';

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

test('providers: campos incompletos, HTTP, URL com secrets e Anthropic Messages são recusados', () => {
  for (const patch of [{ id: '' }, { models: '' }, { models: 'model,\n' }, { baseUrl: '' }, { baseUrl: 'http://api.example.com/v1' },
    { baseUrl: 'https://user:secret@api.example.com/v1' }, { baseUrl: 'https://api.example.com/v1?key=secret' },
    { baseUrl: 'https://api.example.com/v1#key' }, { protocol: 'anthropic-messages' }, { baseUrl: 'https://api.anthropic.com/v1' },
    { baseUrl: 'https://api.example.com/v1/messages' }, { apiKey: 'bad\nkey' }]) {
    assert.throws(() => buildProviderPayload(config(), form(patch)), JSON.stringify(Object.keys(patch)));
  }
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
