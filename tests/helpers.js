import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import http from 'node:http';
import { RustStore } from './rust-store.js';

// The Store under test is always the Rust executable (mc-contract); Node only orchestrates fixtures and assertions.
export async function tempStore() {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'mc-test-'));
  const store = new RustStore(path.join(dir, 'test.db'));
  await store.ready();
  return {
    store,
    dir,
    async cleanup() {
      await store.close();
      fs.rmSync(dir, { recursive: true, force: true });
    },
  };
}

export async function newConv(store, over = {}) {
  return store.call('createConversation', {
    name: 'Teste', persona: 'Alex', routeId: 'cbai/deepseek-v4.1-flash', presetId: 'deepseek-v41-flash', effort: 'high',
    prompt: 'Mira é uma maga arrogante. Nora é sua melhor amiga.', ...over,
  });
}

// Runs a full turn (send + saved reply) without going through a provider.
export async function turn(store, convId, userText, reply, extra = {}) {
  const begin = await store.call('beginTurn', { convId, opId: extra.opId, mode: extra.mode || 'send', content: userText, targetId: extra.targetId, routeId: 'cbai/deepseek-v4.1-flash', presetId: 'deepseek-v41-flash', effort: 'high' });
  if (begin.replay) return begin;
  const fin = await store.call('finishGeneration', { generationId: begin.generationId, text: reply, status: extra.status || 'complete', usage: {}, flags: extra.flags || [] });
  return { ...begin, fin };
}

// Fake, reconfigurable provider: answers /models and /chat/completions (SSE) and records the requests.
export async function mockRouter(initial = {}) {
  const requests = [];
  const correctionRequests = [];
  const behavior = {
    chunks: ['*Mira sorri.*\n\n', 'Mira: Oi, Alex.'],
    reasoning: [],
    delayMs: 5,
    usage: { prompt_tokens: 1234, completion_tokens: 20, completion_tokens_details: { reasoning_tokens: 300 } },
    status: 200,
    hold: null,
    finish: 'stop',
    ...initial,
  };
  const server = http.createServer((req, res) => {
    if (req.url.endsWith('/models')) {
      res.setHeader('Content-Type', 'application/json');
      return res.end(JSON.stringify({ data: [{ id: 'cbai/deepseek-v4.1-flash' }, { id: 'cc/claude-sonnet-5-5' }, { id: 'ag/gemini-3.8-flash-low' }, { id: 'nvidia/parakeet-ctc-1.1b-asr' }] }));
    }
    let body = '';
    req.on('data', (d) => (body += d));
    req.on('end', async () => {
      const b = { ...behavior };
      const hit = { url: req.url, headers: req.headers, body: JSON.parse(body || '{}'), at: Date.now() };
      if (hit.body.stream === false) {
        correctionRequests.push(hit);
        res.setHeader('Content-Type', 'application/json');
        if (b.correctionStatus) { res.statusCode=b.correctionStatus; return res.end('{}'); }
        return res.end(JSON.stringify({ choices: [{ message: { content: b.correctionText ?? b.chunks.join('') } }], usage: b.usage }));
      }
      requests.push(hit);
      b.onRequest?.(hit);
      if (b.status !== 200) {
        res.statusCode = b.status;
        return res.end('boom');
      }
      res.writeHead(200, { 'Content-Type': 'text/event-stream' });
      if (b.hold) await b.hold;
      for (const r of b.reasoning) res.write(`data: ${JSON.stringify({ choices: [{ delta: { reasoning_content: r } }] })}\n\n`);
      for (const c of b.chunks) {
        if (res.destroyed) return;
        res.write(`data: ${JSON.stringify({ choices: [{ delta: { content: c } }] })}\n\n`);
        await new Promise((r) => setTimeout(r, b.delayMs));
      }
      res.write(`data: ${JSON.stringify({ choices: [{ delta: {}, finish_reason: b.finish }], usage: b.usage })}\n\n`);
      res.write('data: [DONE]\n\n');
      res.end();
    });
  });
  await new Promise((r) => server.listen(0, '127.0.0.1', r));
  return {
    server,
    requests,
    correctionRequests,
    behavior,
    url: `http://127.0.0.1:${server.address().port}/v1`,
    set(over) {
      Object.assign(behavior, over);
    },
    close: () => new Promise((r) => { server.closeAllConnections?.(); server.close(r); }),
  };
}
