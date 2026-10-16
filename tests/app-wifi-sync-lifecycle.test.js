import test from 'node:test';
import assert from 'node:assert/strict';
import http from 'node:http';
import { api, config, ApiError } from '../client/src/api.js';

function createMockWifiPcServer() {
  const requests = [];
  const activeGenerations = new Map();
  let failProbe = false;

  const server = http.createServer((req, res) => {
    let body = '';
    req.on('data', (c) => (body += c));
    req.on('end', () => {
      const url = new URL(req.url, 'http://192.168.1.100:3000');
      const reqInfo = {
        method: req.method,
        path: url.pathname,
        query: Object.fromEntries(url.searchParams),
        body: body ? JSON.parse(body) : null,
        headers: req.headers,
      };
      requests.push(reqInfo);

      // GET /api/health
      if (req.method === 'GET' && url.pathname === '/api/health') {
        if (failProbe) {
          res.writeHead(503, { 'Content-Type': 'application/json' });
          return res.end(JSON.stringify({ error: 'PC offline' }));
        }
        res.writeHead(200, { 'Content-Type': 'application/json' });
        return res.end(JSON.stringify({ ok: true, name: 'modularchat', encrypted: true }));
      }

      // POST /api/pair (Pareamento com PC na Wi-Fi)
      if (req.method === 'POST' && url.pathname === '/api/pair') {
        if (reqInfo.body.code === '123456') {
          res.writeHead(201, { 'Content-Type': 'application/json' });
          return res.end(JSON.stringify({ token: 'wifi-pc-auth-token-9876', deviceId: 'dev-phone-wifi' }));
        }
        res.writeHead(400, { 'Content-Type': 'application/json' });
        return res.end(JSON.stringify({ error: 'Código de pareamento incorreto' }));
      }

      // POST /api/chats/:id/message
      if (req.method === 'POST' && url.pathname.endsWith('/message')) {
        const chatId = url.pathname.split('/')[3];
        const genId = 'gen-wifi-1';
        activeGenerations.set(genId, {
          chatId,
          fullText: 'Mira: Resposta processada pelo PC no Wi-Fi.',
          status: 'complete',
        });

        res.writeHead(200, {
          'Content-Type': 'text/event-stream',
          'Cache-Control': 'no-cache',
          Connection: 'keep-alive',
        });

        res.write(`data: ${JSON.stringify({ ack: { generationId: genId, opId: reqInfo.body.opId, mode: 'send', partId: 'part-wifi-1' } })}\n\n`);
        res.write(`data: ${JSON.stringify({ delta: 'Mira: Resposta processada' })}\n\n`);

        // Simula queda de Wi-Fi antes de concluir: corta a conexão abruptamente
        setTimeout(() => {
          res.destroy(new Error('Wi-Fi connection reset by peer'));
        }, 15);
        return;
      }

      // GET /api/chats/:id/generations/:genId/stream (Reanexação após queda de Wi-Fi)
      if (req.method === 'GET' && url.pathname.includes('/generations/') && url.pathname.endsWith('/stream')) {
        const parts = url.pathname.split('/');
        const genId = parts[5];
        const from = Number(url.searchParams.get('from')) || 0;
        const gen = activeGenerations.get(genId);

        if (!gen) {
          res.writeHead(404, { 'Content-Type': 'application/json' });
          return res.end(JSON.stringify({ error: 'Geração não encontrada' }));
        }

        res.writeHead(200, {
          'Content-Type': 'text/event-stream',
          'Cache-Control': 'no-cache',
          Connection: 'keep-alive',
        });

        // Envia confirmação de ack
        res.write(`data: ${JSON.stringify({ ack: { generationId: genId, opId: 'op-reattach', mode: 'send', partId: 'part-wifi-1' } })}\n\n`);

        // Envia o trecho restante a partir do offset
        const remaining = gen.fullText.slice(from);
        res.write(`data: ${JSON.stringify({ delta: remaining })}\n\n`);
        res.write(`data: ${JSON.stringify({ done: true, status: 'complete', usage: { total_tokens: 35 } })}\n\n`);
        res.end();
        return;
      }

      // GET /api/chats/:id
      if (req.method === 'GET' && url.pathname.startsWith('/api/chats/')) {
        res.writeHead(200, { 'Content-Type': 'application/json' });
        return res.end(JSON.stringify({
          id: url.pathname.split('/')[3],
          name: 'Mira',
          history: [{ id: 'part-wifi-1', role: 'assistant', content: 'Mira: Resposta processada pelo PC no Wi-Fi.', status: 'complete' }],
          generation: null,
        }));
      }

      res.writeHead(404, { 'Content-Type': 'application/json' });
      res.end(JSON.stringify({ error: 'Não encontrado' }));
    });
  });

  return new Promise((resolve) => {
    server.listen(0, '127.0.0.1', () => {
      resolve({
        server,
        url: `http://127.0.0.1:${server.address().port}`,
        requests,
        setFailProbe: (v) => { failProbe = v; },
        close: () => new Promise((r) => server.close(r)),
      });
    });
  });
}

test('app wifi lifecycle: pareamento com PC via Wi-Fi e propagação do Bearer token', async () => {
  const mock = await createMockWifiPcServer();
  const originals = { window: globalThis.window, storage: globalThis.localStorage, fetch: globalThis.fetch };

  const store = new Map();
  globalThis.window = { Capacitor: { isNativePlatform: () => true }, dispatchEvent() {} };
  globalThis.localStorage = {
    getItem: (k) => store.get(k) || null,
    setItem: (k, v) => store.set(k, String(v)),
    removeItem: (k) => store.delete(k),
  };

  // Mapeia chamadas do PC Wi-Fi (192.168.1.100:3000) para o mock HTTP local
  globalThis.fetch = async (url, options) => {
    const parsed = new URL(url);
    return originals.fetch(`${mock.url}${parsed.pathname}${parsed.search}`, options);
  };

  try {
    const pcAddress = 'http://192.168.1.100:3000';
    config.configureConnection({ local: pcAddress });

    // Código incorreto rejeita
    await assert.rejects(
      () => api.pair('000000', 'Celular Dev Wi-Fi', pcAddress),
      /Código de pareamento incorreto/
    );

    // Pareamento correto gera token de acesso
    const pairRes = await api.pair('123456', 'Celular Dev Wi-Fi', pcAddress);
    assert.equal(pairRes.token, 'wifi-pc-auth-token-9876');
    assert.equal(config.token, 'wifi-pc-auth-token-9876');

    // Próxima chamada protegida ao chat carrega o token Bearer
    const chat = await api.getChat('chat-wifi-1');
    assert.equal(chat.id, 'chat-wifi-1');

    const lastReq = mock.requests.at(-1);
    assert.equal(lastReq.headers.authorization, 'Bearer wifi-pc-auth-token-9876');
  } finally {
    config.configureConnection({});
    if (originals.window) globalThis.window = originals.window; else delete globalThis.window;
    if (originals.storage) globalThis.localStorage = originals.storage; else delete globalThis.localStorage;
    globalThis.fetch = originals.fetch;
    await mock.close();
  }
});

test('app wifi lifecycle: queda de Wi-Fi no meio do streaming e reanexação com attachGeneration', async () => {
  const mock = await createMockWifiPcServer();
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
    const pcAddress = 'http://192.168.1.100:3000';
    config.configureConnection({ local: pcAddress });
    config.token = 'wifi-pc-auth-token-9876';

    const events = [];
    let receivedDelta = '';

    // Envio inicial cai devido à desconexão simulada do Wi-Fi
    await assert.rejects(
      () =>
        api.sendMessage('chat-wifi-1', { content: 'Pergunta importante', opId: 'op-reattach' }, (ev) => {
          events.push(ev);
          if (ev.delta) receivedDelta += ev.delta;
        }),
      (err) => err instanceof ApiError || err.message.includes('interrompido') || err.message.includes('conexão')
    );

    // O envio parcial registrou ack e parte do texto antes da queda
    assert.ok(events.some((e) => e.ack && e.ack.generationId === 'gen-wifi-1'));
    assert.ok(receivedDelta.includes('Resposta processada'));

    // Celular reconecta no Wi-Fi e reanexa à geração existente no PC
    const reattachEvents = [];
    let completeText = receivedDelta;

    await api.attachGeneration(
      'chat-wifi-1',
      'gen-wifi-1',
      receivedDelta.length,
      (ev) => {
        reattachEvents.push(ev);
        if (ev.delta) completeText += ev.delta;
      }
    );

    // Stream reatado entrega o restante até done: true
    assert.ok(reattachEvents.some((e) => e.done === true && e.status === 'complete'));
    assert.equal(completeText, 'Mira: Resposta processada pelo PC no Wi-Fi.');
  } finally {
    config.configureConnection({});
    if (originals.window) globalThis.window = originals.window; else delete globalThis.window;
    if (originals.storage) globalThis.localStorage = originals.storage; else delete globalThis.localStorage;
    globalThis.fetch = originals.fetch;
    await mock.close();
  }
});
