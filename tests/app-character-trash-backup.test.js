import test from 'node:test';
import assert from 'node:assert/strict';
import http from 'node:http';
import { api, config } from '../client/src/api.js';

function createMockCharacterServer() {
  const chats = new Map();
  const castMap = new Map();
  const trashItems = [];

  const initialChat = {
    id: 'chat-card-1',
    name: 'Mira',
    persona: 'Alex',
    prompt: 'Mira é uma maga sarcástica e inteligente.',
    history: [
      { id: 'm1', role: 'user', content: 'Oi Mira' },
      { id: 'm2', role: 'assistant', content: 'Mira: O que foi?', status: 'complete' },
    ],
  };
  chats.set(initialChat.id, initialChat);
  castMap.set(initialChat.id, [
    { id: 'c1', name: 'Nora', role: 'Amiga' },
  ]);

  const server = http.createServer((req, res) => {
    let body = '';
    req.on('data', (c) => (body += c));
    req.on('end', () => {
      const url = new URL(req.url, 'http://192.168.1.100:3000');
      const reqInfo = { method: req.method, path: url.pathname, body: body ? JSON.parse(body) : null };

      // GET /api/health
      if (req.method === 'GET' && url.pathname === '/api/health') {
        res.writeHead(200, { 'Content-Type': 'application/json' });
        return res.end(JSON.stringify({ ok: true, name: 'modularchat', encrypted: true }));
      }

      // GET /api/chats/:id
      if (req.method === 'GET' && url.pathname === `/api/chats/${initialChat.id}`) {
        res.writeHead(200, { 'Content-Type': 'application/json' });
        return res.end(JSON.stringify(chats.get(initialChat.id)));
      }

      // PUT /api/chats/:id (Atualização de ficha/prompt)
      if (req.method === 'PUT' && url.pathname === `/api/chats/${initialChat.id}`) {
        const c = chats.get(initialChat.id);
        Object.assign(c, reqInfo.body);
        res.writeHead(200, { 'Content-Type': 'application/json' });
        return res.end(JSON.stringify(c));
      }

      // GET /api/chats/:id/cast
      if (req.method === 'GET' && url.pathname.endsWith('/cast')) {
        const id = url.pathname.split('/')[3];
        res.writeHead(200, { 'Content-Type': 'application/json' });
        return res.end(JSON.stringify({ cast: castMap.get(id) || [] }));
      }

      // POST /api/chats/:id/cast (Adicionar ao elenco)
      if (req.method === 'POST' && url.pathname.endsWith('/cast')) {
        const id = url.pathname.split('/')[3];
        const list = castMap.get(id) || [];
        const newCast = { id: `c-${Date.now()}`, ...reqInfo.body };
        list.push(newCast);
        castMap.set(id, list);
        res.writeHead(201, { 'Content-Type': 'application/json' });
        return res.end(JSON.stringify(newCast));
      }

      // DELETE /api/chats/:id (Mover chat para a lixeira)
      if (req.method === 'DELETE' && url.pathname.startsWith('/api/chats/')) {
        const id = url.pathname.split('/')[3];
        const c = chats.get(id);
        if (c) {
          chats.delete(id);
          trashItems.push({ id: `trash-${id}`, type: 'conversation', name: c.name, originalId: id });
        }
        res.writeHead(200, { 'Content-Type': 'application/json' });
        return res.end(JSON.stringify({ ok: true, deleted: id }));
      }

      // GET /api/trash (Consultar lixeira)
      if (req.method === 'GET' && url.pathname === '/api/trash') {
        res.writeHead(200, { 'Content-Type': 'application/json' });
        return res.end(JSON.stringify({ days: 30, items: trashItems }));
      }

      // POST /api/trash/restore (Restaurar da lixeira)
      if (req.method === 'POST' && url.pathname === '/api/trash/restore') {
        const item = reqInfo.body;
        const idx = trashItems.findIndex((t) => t.id === item.id);
        if (idx !== -1) {
          trashItems.splice(idx, 1);
          chats.set(initialChat.id, initialChat);
        }
        res.writeHead(200, { 'Content-Type': 'application/json' });
        return res.end(JSON.stringify({ ok: true, restored: item.id }));
      }

      // POST /api/trash/purge (Purga permanente da lixeira)
      if (req.method === 'POST' && url.pathname === '/api/trash/purge') {
        trashItems.length = 0;
        res.writeHead(200, { 'Content-Type': 'application/json' });
        return res.end(JSON.stringify({ ok: true, purged: true }));
      }

      // GET /api/export ou /api/chats/:id/export
      if (req.method === 'GET' && url.pathname.endsWith('/export')) {
        res.writeHead(200, { 'Content-Type': 'application/json' });
        return res.end(JSON.stringify({
          format: 'modularchat-encrypted-export',
          version: 2,
          chats: [initialChat],
        }));
      }

      // POST /api/import
      if (req.method === 'POST' && url.pathname === '/api/import') {
        res.writeHead(201, { 'Content-Type': 'application/json' });
        return res.end(JSON.stringify({
          ok: true,
          imported: reqInfo.body.chats || [],
        }));
      }

      res.writeHead(404, { 'Content-Type': 'application/json' });
      res.end(JSON.stringify({ error: 'Endpoint não implementado no mock' }));
    });
  });

  return new Promise((resolve) => {
    server.listen(0, '127.0.0.1', () => {
      resolve({
        server,
        url: `http://127.0.0.1:${server.address().port}`,
        close: () => new Promise((r) => server.close(r)),
      });
    });
  });
}

test('app character and trash flow: atualização de ficha, elenco, ciclo de lixeira e restauração', async () => {
  const mock = await createMockCharacterServer();
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

    // 1. Atualização da ficha de personagem (Character Card Prompt)
    const updated = await api.updateChat('chat-card-1', {
      prompt: 'Mira é uma arquimaga do reino de Elyria com domínio sobre fogo e tempo.',
    });
    assert.ok(updated.prompt.includes('arquimaga do reino de Elyria'));

    // 2. Gerenciamento do elenco (Cast)
    const newCast = await api.addCast('chat-card-1', { name: 'Kaelen', role: 'Bibliotecário' });
    assert.equal(newCast.name, 'Kaelen');
    assert.equal(newCast.role, 'Bibliotecário');

    const castList = await api.cast('chat-card-1');
    assert.ok(castList.cast.some((c) => c.name === 'Kaelen'));
    assert.ok(castList.cast.some((c) => c.name === 'Nora'));

    // 3. Exclusão do chat e verificação na lixeira (Soft Delete)
    await api.deleteChat('chat-card-1');

    const trash = await api.trash();
    assert.equal(trash.days, 30);
    assert.ok(trash.items.some((item) => item.originalId === 'chat-card-1'));

    // 4. Restauração do chat a partir da lixeira
    const trashItem = trash.items.find((item) => item.originalId === 'chat-card-1');
    const restoreRes = await api.restoreTrash(trashItem);
    assert.equal(restoreRes.ok, true);

    // Confirma que a lixeira esvaziou o item restaurado
    const trashAfter = await api.trash();
    assert.ok(!trashAfter.items.some((item) => item.originalId === 'chat-card-1'));

    // 5. Exportação e Importação de backup
    const exported = await api.exportAll('chat-card-1');
    assert.equal(exported.format, 'modularchat-encrypted-export');
    assert.equal(exported.chats.length, 1);

    const imported = await api.importData(exported);
    assert.equal(imported.ok, true);
    assert.equal(imported.imported.length, 1);
  } finally {
    config.configureConnection({});
    if (originals.window) globalThis.window = originals.window; else delete globalThis.window;
    if (originals.storage) globalThis.localStorage = originals.storage; else delete globalThis.localStorage;
    globalThis.fetch = originals.fetch;
    await mock.close();
  }
});
