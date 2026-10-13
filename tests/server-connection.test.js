import test from 'node:test';
import assert from 'node:assert/strict';
import { createServerConnection, normalizeServer } from '../client/src/server-connection.js';

const local = 'https://pc.example.com:8443', remote = 'https://chat.example.com';
function fixture(overrides = {}) {
  const data = new Map(), calls = [], changes = [];
  const storage = { getItem: key => data.get(key), setItem: (key, value) => data.set(key, value), removeItem: key => data.delete(key) };
  const fetch = async (url, options) => {
    calls.push({ url, options });
    if (overrides.fetch) return overrides.fetch(url, options);
    return Response.json({ ok: true, name: 'modularchat', encrypted: true });
  };
  const connection = createServerConnection({ storage, native: () => true, token: () => 'fixture-token', fetch, onChange: value => changes.push(value), ...overrides, fetch });
  return { connection, data, calls, changes };
}

test('conexão: HTTPS sem credenciais ou caminhos; nenhum host pessoal embutido', () => {
  assert.equal(normalizeServer(' https://chat.example.com/ '), remote);
  for (const value of ['http://pc.example.com', 'https://user:pass@chat.example.com', `${remote}/api`, `${remote}?token=x`, `${remote}#x`, 'file:///tmp']) {
    assert.throws(() => normalizeServer(value));
  }
  const f = fixture();
  assert.deepEqual(f.connection.configured(), { local: '', remote: '' });
  assert.equal(f.connection.hasServer(), false);
});

test('conexão: prefere LAN, compartilha sondagem e mantém escolha por 30 segundos', async () => {
  let now = 0;
  const f = fixture({ now: () => now });
  f.connection.configure({ local, remote });
  assert.deepEqual(await Promise.all([f.connection.resolve(), f.connection.resolve()]), [local, local]);
  assert.equal(f.calls.length, 1);
  assert.equal(f.calls[0].options.headers.Authorization, 'Bearer fixture-token');
  assert.equal(f.calls[0].options.redirect, 'error');
  now = 29999;
  assert.equal(await f.connection.resolve(), local);
  assert.equal(f.calls.length, 1);
  now = 30001;
  await f.connection.resolve();
  assert.equal(f.calls.length, 2);
});

test('conexão: usa internet quando LAN falha; falha/rede nova invalida escolha', async () => {
  const f = fixture({ fetch: async url => {
    if (url.startsWith(local)) throw Error('offline');
    return Response.json({ ok: true, name: 'modularchat', encrypted: true });
  } });
  f.connection.configure({ local, remote });
  assert.equal(await f.connection.resolve(), remote);
  assert.deepEqual(f.calls.map(c => c.url), [`${local}/api/health`, `${remote}/api/health`]);
  f.connection.invalidate();
  await f.connection.resolve();
  assert.equal(f.calls.length, 4);
});

test('conexão: revogação conserva o host e não provoca troca para o domínio', async () => {
  const f = fixture({ fetch: async () => Response.json({ code: 'UNPAIRED' }, { status: 401 }) });
  f.connection.configure({ local, remote });
  assert.equal(await f.connection.resolve(), local);
  assert.equal(f.calls.length, 1);
});

test('conexão: outro PC não herda domínio; tentativa malsucedida restaura configuração legada', async () => {
  const f = fixture();
  f.data.set('mc.server', 'http://192.168.0.10:3000');
  assert.equal(f.connection.hasServer(), false, 'HTTP antigo exige configuração HTTPS explícita');
  const undo = f.connection.configure({ local, remote });
  undo();
  assert.deepEqual(f.connection.configured(), { local: 'http://192.168.0.10:3000', remote: '' });
  f.connection.configure({ local, remote });
  f.connection.configure({ local: 'https://other.example.com' });
  assert.deepEqual(f.connection.configured(), { local: 'https://other.example.com', remote: '' });
});

test('conexão: mudança durante sondagem impede requisição com configuração obsoleta', async () => {
  let release;
  const f = fixture({ fetch: () => new Promise(resolve => { release = () => resolve(Response.json({ ok: true, name: 'modularchat', encrypted: true })); }) });
  f.connection.configure({ local, remote });
  const pending = f.connection.resolve();
  f.connection.configure({ local: 'https://other.example.com' });
  release();
  await assert.rejects(pending, /conexão mudou/);
  assert.equal(f.connection.current(), 'https://other.example.com');
});

test('conexão: cancelar um consumidor não cancela a sondagem compartilhada', async () => {
  let release;
  const f = fixture({ fetch: () => new Promise(resolve => { release = () => resolve(Response.json({ ok: true, name: 'modularchat', encrypted: true })); }) });
  f.connection.configure({ local });
  const abort = new AbortController();
  const first = f.connection.resolve(abort.signal), second = f.connection.resolve();
  abort.abort();
  await assert.rejects(first, { name: 'AbortError' });
  release();
  assert.equal(await second, local);
  assert.equal(f.calls.length, 1);
});

test('conexão: rejeita serviço diferente ou sem cofre e mantém origem do navegador', async () => {
  for (const body of [{ ok: true, name: 'another-app', encrypted: true }, { ok: true, name: 'modularchat', encrypted: false }]) {
    const f = fixture({ fetch: async () => Response.json(body) });
    f.connection.configure({ local });
    await assert.rejects(f.connection.resolve(), /Não consegui conectar/);
  }
  const browser = fixture({ native: () => false });
  assert.equal(await browser.connection.resolve(), '');
  assert.equal(browser.calls.length, 0);
});
