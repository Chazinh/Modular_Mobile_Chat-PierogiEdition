import test from 'node:test';
import assert from 'node:assert/strict';
import http from 'node:http';
import { api, config, ApiError } from '../client/src/api.js';

function createMockAppServer() {
  const chats = new Map();
  const versions = new Map();
  const requests = [];
  let stopCalled = false;

  // Conversa inicial de teste
  chats.set('chat-1', {
    id: 'chat-1',
    name: 'Mira',
    persona: 'Alex',
    routeId: 'cbai/deepseek-v4.1-flash',
    history: [
      { id: 'u1', role: 'user', content: 'Olá, Mira!' },
      { id: 'a1', role: 'assistant', content: 'Mira: Olá, Alex.', status: 'complete', versionCount: 1 },
    ],
    generation: null,
    revision: 1,
  });

  const server = http.createServer((req, res) => {
    let body = '';
    req.on('data', (c) => (body += c));
    req.on('end', () => {
      const url = new URL(req.url, 'http://127.0.0.1');
      const reqInfo = { method: req.method, path: url.pathname, query: Object.fromEntries(url.searchParams), body: body ? JSON.parse(body) : null, headers: req.headers };
      requests.push(reqInfo);

      // GET /api/health
      if (req.method === 'GET' && url.pathname === '/api/health') {
        res.writeHead(200, { 'Content-Type': 'application/json' });
        return res.end(JSON.stringify({ ok: true, name: 'modularchat', encrypted: true }));
      }

      // POST /api/chats/:id/message (Streaming SSE)
      if (req.method === 'POST' && url.pathname.endsWith('/message')) {
        const chatId = url.pathname.split('/')[3];
        const payload = reqInfo.body;

        res.writeHead(200, {
          'Content-Type': 'text/event-stream',
          'Cache-Control': 'no-cache',
          Connection: 'keep-alive',
        });

        // 1. Envia ACK
        const ackData = { ack: { generationId: 'gen-123', opId: payload.opId, mode: 'send', partId: 'part-456' } };
        res.write(`data: ${JSON.stringify(ackData)}\n\n`);

        // 2. Envia Deltas (pensamento e conteúdo)
        setTimeout(() => {
          res.write(`data: ${JSON.stringify({ choices: [{ delta: { reasoning_content: 'Analisando o pedido...' } }] })}\n\n`);
          res.write(`data: ${JSON.stringify({ delta: 'Mira: Eu estava lendo' })}\n\n`);
          res.write(`data: ${JSON.stringify({ delta: ' na biblioteca.' })}\n\n`);

          // 3. Conclui com Done
          setTimeout(() => {
            const chat = chats.get(chatId);
            if (chat) {
              chat.history.push({ id: payload.opId, role: 'user', content: payload.content });
              chat.history.push({ id: 'part-456', role: 'assistant', content: 'Mira: Eu estava lendo na biblioteca.', status: 'complete', versionCount: 1 });
              chat.revision++;
            }
            res.write(`data: ${JSON.stringify({ done: true, status: 'complete', usage: { prompt_tokens: 120, completion_tokens: 15 } })}\n\n`);
            res.end();
          }, 20);
        }, 10);
        return;
      }

      // POST /api/chats/:id/generations/:genId/stop
      if (req.method === 'POST' && url.pathname.includes('/generations/') && url.pathname.endsWith('/stop')) {
        stopCalled = true;
        res.writeHead(200, { 'Content-Type': 'application/json' });
        return res.end(JSON.stringify({ ok: true, stopped: true }));
      }

      // GET /api/chats/:id/messages/:msgId/versions
      if (req.method === 'GET' && url.pathname.endsWith('/versions')) {
        const parts = url.pathname.split('/');
        const msgId = parts[5];
        res.writeHead(200, { 'Content-Type': 'application/json' });
        return res.end(JSON.stringify({ versions: versions.get(msgId) || [] }));
      }

      // POST /api/chats/:id/duplicate
      if (req.method === 'POST' && url.pathname.endsWith('/duplicate')) {
        const parts = url.pathname.split('/');
        const chatId = parts[3];
        const original = chats.get(chatId);
        const dupId = `chat-dup-${Date.now()}`;
        const duplicate = { ...original, id: dupId, name: `${original.name} (Cópia)` };
        chats.set(dupId, duplicate);
        res.writeHead(201, { 'Content-Type': 'application/json' });
        return res.end(JSON.stringify(duplicate));
      }

      // GET /api/chats/:id
      if (req.method === 'GET' && url.pathname.startsWith('/api/chats/')) {
        const chatId = url.pathname.split('/')[3];
        const chat = chats.get(chatId);
        if (chat) {
          res.writeHead(200, { 'Content-Type': 'application/json' });
          return res.end(JSON.stringify(chat));
        }
        res.writeHead(404, { 'Content-Type': 'application/json' });
        return res.end(JSON.stringify({ error: 'Chat não encontrado' }));
      }

      // PUT /api/chats/:id/messages/:msgId (Edição)
      if (req.method === 'PUT' && url.pathname.includes('/messages/')) {
        const parts = url.pathname.split('/');
        const chatId = parts[3];
        const msgId = parts[5];
        const chat = chats.get(chatId);
        if (chat) {
          const msg = chat.history.find((m) => m.id === msgId);
          if (msg) {
            msg.content = reqInfo.body.content;
            // Cria registro de versão
            const list = versions.get(msgId) || [{ id: 'v1', content: 'Versão antiga', current: false }];
            list.push({ id: reqInfo.body.opId || 'v2', content: reqInfo.body.content, current: true });
            versions.set(msgId, list);
          }
          res.writeHead(200, { 'Content-Type': 'application/json' });
          return res.end(JSON.stringify({ ok: true, messageId: msgId, content: reqInfo.body.content }));
        }
      }

      // GET /api/search?q=...
      if (req.method === 'GET' && url.pathname === '/api/search') {
        const q = (url.searchParams.get('q') || '').toLowerCase();
        const matches = [];
        for (const chat of chats.values()) {
          for (const m of chat.history) {
            if (m.content.toLowerCase().includes(q)) {
              matches.push({ chatId: chat.id, messageId: m.id, snippet: m.content });
            }
          }
        }
        res.writeHead(200, { 'Content-Type': 'application/json' });
        return res.end(JSON.stringify({ results: matches }));
      }

      // Fallback 404
      res.writeHead(404, { 'Content-Type': 'application/json' });
      res.end(JSON.stringify({ error: 'Rota mock não implementada' }));
    });
  });

  return new Promise((resolve) => {
    server.listen(0, '127.0.0.1', () => {
      resolve({
        server,
        url: `http://127.0.0.1:${server.address().port}`,
        requests,
        chats,
        getStopCalled: () => stopCalled,
        close: () => new Promise((r) => server.close(r)),
      });
    });
  });
}

test('app chat flow: envio de mensagem com streaming SSE, ack e deltas', async () => {
  const mock = await createMockAppServer();
  const originals = { window: globalThis.window, storage: globalThis.localStorage, fetch: globalThis.fetch };

  const store = new Map();
  globalThis.window = { Capacitor: { isNativePlatform: () => true }, dispatchEvent() {} };
  globalThis.localStorage = {
    getItem: (k) => store.get(k) || null,
    setItem: (k, v) => store.set(k, String(v)),
    removeItem: (k) => store.delete(k),
  };
  globalThis.fetch = async (url, options) => {
    const parsed = new URL(url);
    return originals.fetch(`${mock.url}${parsed.pathname}${parsed.search}`, options);
  };

  try {
    config.configureConnection({ local: 'https://pc.example.com' });

    const events = [];
    await api.sendMessage(
      'chat-1',
      { content: 'Onde você estava?', opId: 'op-msg-1' },
      (ev) => events.push(ev)
    );

    // Confirma recepção de eventos SSE
    assert.ok(events.some((e) => e.ack && e.ack.generationId === 'gen-123'), 'ACK deve ser recebido');
    assert.ok(events.some((e) => e.delta && e.delta.includes('biblioteca')), 'Deltas de texto devem chegar no stream');
    assert.ok(events.some((e) => e.done === true && e.status === 'complete'), 'Done deve finalizar o stream');

    // Confirma atualização no servidor mock
    const chat = await api.getChat('chat-1');
    assert.ok(chat.history.some((m) => m.content === 'Onde você estava?'));
    assert.ok(chat.history.some((m) => m.content.includes('biblioteca')));
  } finally {
    config.configureConnection({});
    if (originals.window) globalThis.window = originals.window; else delete globalThis.window;
    if (originals.storage) globalThis.localStorage = originals.storage; else delete globalThis.localStorage;
    globalThis.fetch = originals.fetch;
    await mock.close();
  }
});

test('app chat flow: bloqueio de mensagens concorrentes (AMBIGUOUS_WRITE) e controle de stop', async () => {
  const mock = await createMockAppServer();
  const originals = { window: globalThis.window, storage: globalThis.localStorage, fetch: globalThis.fetch };

  const store = new Map();
  globalThis.window = { Capacitor: { isNativePlatform: () => true }, dispatchEvent() {} };
  globalThis.localStorage = {
    getItem: (k) => store.get(k) || null,
    setItem: (k, v) => store.set(k, String(v)),
    removeItem: (k) => store.delete(k),
  };
  globalThis.fetch = async (url, options) => {
    const parsed = new URL(url);
    return originals.fetch(`${mock.url}${parsed.pathname}${parsed.search}`, options);
  };

  try {
    config.configureConnection({ local: 'https://pc.example.com' });

    // Simula uma operação já pendente em andamento no chat
    await api.setPending('chat-1', { opId: 'op-em-andamento', generationId: 'gen-999', originServer: 'https://pc.example.com', originKind: 'pc' });

    // Tentativa de enviar outra mensagem com operação pendente deve ser barrada imediatamente
    await assert.rejects(
      () => api.sendMessage('chat-1', { content: 'Segunda mensagem' }, () => {}),
      (err) => err instanceof ApiError && err.code === 'AMBIGUOUS_WRITE'
    );

    // Usuário clica em parar (stopGeneration)
    const stopRes = await api.stopGeneration('chat-1', 'gen-999');
    assert.equal(stopRes.stopped, true);
    assert.equal(mock.getStopCalled(), true);

    // Limpa pendência após parada
    await api.setPending('chat-1', null);
  } finally {
    config.configureConnection({});
    if (originals.window) globalThis.window = originals.window; else delete globalThis.window;
    if (originals.storage) globalThis.localStorage = originals.storage; else delete globalThis.localStorage;
    globalThis.fetch = originals.fetch;
    await mock.close();
  }
});

test('app chat flow: edição de mensagem, ramificação (branching) e busca no histórico', async () => {
  const mock = await createMockAppServer();
  const originals = { window: globalThis.window, storage: globalThis.localStorage, fetch: globalThis.fetch };

  const store = new Map();
  globalThis.window = { Capacitor: { isNativePlatform: () => true }, dispatchEvent() {} };
  globalThis.localStorage = {
    getItem: (k) => store.get(k) || null,
    setItem: (k, v) => store.set(k, String(v)),
    removeItem: (k) => store.delete(k),
  };
  globalThis.fetch = async (url, options) => {
    const parsed = new URL(url);
    return originals.fetch(`${mock.url}${parsed.pathname}${parsed.search}`, options);
  };

  try {
    config.configureConnection({ local: 'https://pc.example.com' });

    // Edição de mensagem u1
    const editRes = await api.editMessage('chat-1', 'u1', 'Texto editado pelo usuário');
    assert.equal(editRes.content, 'Texto editado pelo usuário');

    // Consulta histórico de versões da mensagem
    const vers = await api.versions('chat-1', 'u1');
    assert.ok(vers.versions.length >= 2, 'Deve registrar ramificações de versão');
    assert.equal(vers.versions.at(-1).content, 'Texto editado pelo usuário');

    // Duplicação de conversa
    const duplicated = await api.duplicateChat('chat-1');
    assert.ok(duplicated.id.startsWith('chat-dup-'));
    assert.equal(duplicated.name, 'Mira (Cópia)');

    // Busca no histórico (FTS)
    const searchRes = await api.search('mira');
    assert.ok(searchRes.results.length > 0, 'Busca deve localizar termo mira');
    assert.ok(searchRes.results.some((r) => r.snippet.toLowerCase().includes('mira')));
  } finally {
    config.configureConnection({});
    if (originals.window) globalThis.window = originals.window; else delete globalThis.window;
    if (originals.storage) globalThis.localStorage = originals.storage; else delete globalThis.localStorage;
    globalThis.fetch = originals.fetch;
    await mock.close();
  }
});
