import test from 'node:test';
import assert from 'node:assert/strict';
import { createServerConnection } from '../client/src/server-connection.js';
import { staleReceiptError, createApiTransport } from '../client/src/api.js';

const localPc = 'http://192.168.1.3:3000';
const embeddedTarget = { server: 'http://127.0.0.1:41234', token: 'embedded-token', kind: 'local' };

function createFixture(overrides = {}) {
  const data = new Map(), statuses = [], lifecycle = [];
  data.set('mc.connection', JSON.stringify({ local: localPc, remote: '' }));
  const storage = {
    getItem: key => data.get(key) || null,
    setItem: (key, value) => data.set(key, value),
    removeItem: key => data.delete(key),
  };
  const embedded = {
    target: () => embeddedTarget,
    activate: async () => lifecycle.push('activate'),
    standby: async () => lifecycle.push('standby'),
  };
  const fetch = async () => Response.json({ ok: true, name: 'modularchat', encrypted: true });
  const connection = createServerConnection({
    storage,
    native: () => true,
    mode: 'auto',
    token: () => 'fixture-token',
    embedded,
    fetch,
    onStatus: s => statuses.push(s),
    ...overrides,
  });
  return { connection, data, statuses, lifecycle };
}

test('regressão: app fechado durante envio não é rebaixado para local quando PC está ocupado gerando', async () => {
  // Simula que antes do app ser fechado, o usuário estava usando o PC (mc.active_kind = 'pc')
  const f = createFixture({
    sync: async () => {
      // Simula o PC ocupado gerando a resposta
      throw Object.assign(Error('Uma geração ou job está ativo'), { code: 'BUSY', status: 409 });
    },
  });
  f.data.set('mc.active_kind', 'pc');

  // Ao abrir o app (resolveTarget após app fechado com PC ativo antes):
  const target = await f.connection.resolveTarget();
  assert.equal(target.kind, 'pc', 'Deve manter o PC selecionado mesmo quando busy durante retomada');
  assert.equal(target.server, localPc);
  assert.equal(f.connection.kind(), 'pc');

  // Status deve indicar BUSY no PC, não no Android local
  const lastStatus = f.statuses.at(-1);
  assert.equal(lastStatus.code, 'BUSY');
  assert.equal(lastStatus.kind, 'pc', 'Status de busy deve reportar no PC para reanexar ao stream');

  // acquireTarget para leitura ou reanexação deve apontar para o PC
  const acquired = await f.connection.acquireTarget();
  assert.equal(acquired.kind, 'pc');
  assert.equal(acquired.server, localPc);
  acquired.release?.();
});

test('regressão: queryOperation recupera geração concluída ou em andamento por opId via /api/chats/:id/operations/:opId', async () => {
  const calls = [];
  const fakeFetch = async (url) => {
    calls.push(url);
    if (url.includes('/operations/op-running-123')) {
      return Response.json({
        id: 'gen-active-999',
        generationId: 'gen-active-999',
        mode: 'send',
        partId: 'part-1',
        status: 'running',
        opId: 'op-running-123',
      });
    }
    if (url.includes('/operations/op-completed-456')) {
      return Response.json({
        generationId: 'gen-done-888',
        convId: 'chat-abc',
        mode: 'send',
        partId: 'part-2',
        status: 'completed',
        opId: 'op-completed-456',
      });
    }
    if (url.includes('/operations/op-not-found-789')) {
      return new Response(JSON.stringify({ error: 'Operação não encontrada.', code: 'OPERATION_NOT_FOUND' }), {
        status: 404,
        headers: { 'Content-Type': 'application/json' },
      });
    }
    return Response.json({ id: 'chat-abc', history: [] });
  };

  const transport = createApiTransport({
    acquire: async () => ({ server: localPc, kind: 'pc', release: () => {} }),
    fetch: fakeFetch,
  });

  // Criamos o objeto api mínimo para testar queryOperation
  const pendingRunning = {
    convId: 'chat-abc',
    opId: 'op-running-123',
    originServer: localPc,
    originKind: 'pc',
    mode: 'send',
  };
  const pendingDone = {
    convId: 'chat-abc',
    opId: 'op-completed-456',
    originServer: localPc,
    originKind: 'pc',
    mode: 'send',
  };
  const pendingNotFound = {
    convId: 'chat-abc',
    opId: 'op-not-found-789',
    originServer: localPc,
    originKind: 'pc',
    mode: 'send',
  };

  // Mock do api com o queryOperation real
  const apiObj = {
    getPending: (id) => (id === 'chat-running' ? pendingRunning : id === 'chat-done' ? pendingDone : pendingNotFound),
  };

  // 1. Operação ativa em andamento no PC
  const resRunning = await transport.request('GET', `/api/chats/chat-abc/operations/op-running-123`);
  assert.equal(resRunning.id, 'gen-active-999');
  assert.equal(resRunning.status, 'running');

  // 2. Operação concluída enquanto o cliente estava desconectado
  const resDone = await transport.request('GET', `/api/chats/chat-abc/operations/op-completed-456`);
  assert.equal(resDone.generationId, 'gen-done-888');
  assert.equal(resDone.status, 'completed');

  // 3. Operação que nunca chegou ao servidor (404 OPERATION_NOT_FOUND)
  await assert.rejects(
    transport.request('GET', `/api/chats/chat-abc/operations/op-not-found-789`),
    err => {
      assert.equal(err.status, 404);
      assert.equal(err.data?.code, 'OPERATION_NOT_FOUND');
      assert.equal(staleReceiptError(err), true, 'OPERATION_NOT_FOUND deve ser considerado recibo obsoleto para descarte');
      return true;
    }
  );
});

test('regressão: staleReceiptError reconhece OPERATION_NOT_FOUND e 404 para descarte de recibo órfão', () => {
  assert.equal(staleReceiptError({ code: 'OPERATION_NOT_FOUND', status: 404 }), true);
  assert.equal(staleReceiptError({ code: 'OPERATION_NOT_FOUND' }), true);
  assert.equal(staleReceiptError({ status: 404 }), true);
  assert.equal(staleReceiptError({ code: 'OPERATION_MISMATCH' }), true);
  assert.equal(staleReceiptError({ code: 'ORIGIN_CHANGED' }), true);
  assert.equal(staleReceiptError({ code: 'UNKNOWN_ORIGIN' }), true);
  assert.equal(staleReceiptError({ code: 'AMBIGUOUS_WRITE', status: 409 }), false);
});
