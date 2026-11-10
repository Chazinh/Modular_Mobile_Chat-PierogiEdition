import test from 'node:test';
import assert from 'node:assert/strict';
import { readSSE } from '../client/src/api.js';
import { createServerConnection } from '../client/src/server-connection.js';

test('regressão: readSSE aceita separadores mistos sem comer o início do próximo evento', async () => {
  // "\r\n\n" e "\n\r\n" têm 3 caracteres; antes o parser pulava 4 e perdia o "d" de "data:".
  for (const sep of ['\r\n\n', '\n\r\n', '\r\n\r\n', '\n\n']) {
    const events = [];
    await readSSE(new Response(`data: {"delta":"a"}${sep}data: {"delta":"b"}${sep}data: {"done":true}${sep}`), e => events.push(e), true);
    assert.deepEqual(events, [{ delta: 'a' }, { delta: 'b' }, { done: true }], JSON.stringify(sep));
  }
});

test('regressão: recuperação presa à origem não herda a falha de uma seleção automática em andamento', async () => {
  const embeddedTarget = { server: 'http://127.0.0.1:41234', token: 'embedded-token', kind: 'local' };
  const data = new Map([['mc.connection', JSON.stringify({ local: 'http://192.168.1.3:3000', remote: '' })]]);
  let releaseHealth;
  const health = new Promise(resolve => { releaseHealth = resolve; });
  const connection = createServerConnection({
    storage: { getItem: k => data.get(k) || null, setItem: (k, v) => data.set(k, v), removeItem: k => data.delete(k) },
    native: () => true, mode: 'auto', token: () => 'pc-token',
    embedded: { target: () => embeddedTarget, activate: async () => {}, standby: async () => {} },
    fetch: async () => { await health; return Response.json({ ok: false }, { status: 503 }); },
  });
  const switching = connection.switchToPc();
  const recovery = connection.acquireTarget({ origin: { server: embeddedTarget.server, kind: 'local' } });
  releaseHealth();
  await assert.rejects(switching, { code: 'PC_UNAVAILABLE' });
  const target = await recovery;
  assert.equal(target.kind, 'local');
  assert.equal(target.server, embeddedTarget.server);
  target.release();
});
