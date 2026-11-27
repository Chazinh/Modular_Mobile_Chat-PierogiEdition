import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import http from 'node:http';
import { spawn } from 'node:child_process';
import { mockRouter } from './helpers.js';
import { startRust } from './rust-http.js';
import { rustExe } from './rust-target.js';

async function readSse(res) {
  const text = await res.text();
  return text
    .split(/\n\n+/)
    .map((block) => block.trim())
    .filter(Boolean)
    .map((block) => {
      const data = block
        .split('\n')
        .filter((line) => line.startsWith('data:'))
        .map((line) => line.slice(5).trim())
        .join('\n');
      return data ? JSON.parse(data) : null;
    })
    .filter(Boolean);
}

test('429 com mensagem de limite de tokens faz 1 POST sem poda e retorna erro HTTP 429', async (t) => {
  const dir = await fs.mkdtemp(path.join(os.tmpdir(), 'mc-m2-429-'));
  let posts = 0;
  const upstream = http.createServer(async (req, res) => {
    for await (const _ of req) {}
    if (req.url?.endsWith('/models')) {
      res.setHeader('content-type', 'application/json');
      return res.end(JSON.stringify({ data: [{ id: 'custom/m2-model' }] }));
    }
    posts += 1;
    res.statusCode = 429;
    res.setHeader('content-type', 'application/json');
    res.end(JSON.stringify({ error: { message: 'Token limit exceeded: too many tokens per minute (TPM)' } }));
  });
  await new Promise((r) => upstream.listen(0, '127.0.0.1', r));
  let runtime = null;
  t.after(async () => {
    if (runtime) await runtime.close();
    upstream.closeAllConnections();
    await new Promise((r) => upstream.close(r));
    await fs.rm(dir, { recursive: true, force: true });
  });
  const url = `http://127.0.0.1:${upstream.address().port}/v1`;
  runtime = await startRust({ dataDir: dir, upstream: url, review: false });

  const cRes = await fetch(`${runtime.base}/api/chats`, {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify({ name: '429 test', routeId: 'custom/m2-model', presetId: 'generic' }),
  });
  assert.equal(cRes.status, 201);
  const chat = await cRes.json();

  const res = await fetch(`${runtime.base}/api/chats/${chat.id}/message`, {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify({ mode: 'send', content: 'Olá', opId: 'm2-429-op' }),
  });
  assert.equal(res.status, 200);
  const events = await readSse(res);
  const done = events.at(-1);
  assert.equal(done.status, 'error');
  assert.match(done.error, /HTTP 429/);
  assert.equal(posts, 1, '429 TPM must not trigger retry/pruning');
});

test('multi-servidor MC_PROVIDERS_FILE serializa servidor A (maxConcurrent=1) sem bloquear servidor B (R3)', async (t) => {
  const dir = await fs.mkdtemp(path.join(os.tmpdir(), 'mc-m2-multi-'));
  const routerA = await mockRouter();
  const routerB = await mockRouter();
  let child = null;
  t.after(async () => {
    if (child && child.exitCode === null && child.signalCode === null) {
      child.kill();
      await new Promise((r) => child.once('exit', r));
    }
    await routerA.close();
    await routerB.close();
    await fs.rm(dir, { recursive: true, force: true });
  });
  const providersFile = path.join(dir, 'providers.json');
  await fs.writeFile(
    providersFile,
    JSON.stringify({
      default: 'srvA',
      providers: [
        { id: 'srvA', baseUrl: routerA.url, apiKey: 'key-a', maxConcurrent: 1, models: ['custom/alpha', 'custom/beta'] },
        { id: 'srvB', baseUrl: routerB.url, apiKey: 'key-b', maxConcurrent: 1, models: ['custom/gamma'] },
      ],
    }),
  );

  const exe = process.env.MC_SERVER_EXE || rustExe('modularchat-server');
  child = spawn(exe, [], {
    windowsHide: true,
    stdio: ['ignore', 'pipe', 'pipe'],
    env: {
      ...process.env,
      DATA_DIR: dir,
      PORT: '0',
      HOST: '127.0.0.1',
      MC_RUST_MODE: 'experimental',
      REQUIRE_PAIRING: '1',
      MC_RUST_UPSTREAM_ENABLED: '1',
      MC_PROVIDERS_FILE: providersFile,
      NINEROUTER_DB: path.join(dir, 'absent-router.db'),
      MC_LOCAL_MODELS_ENABLED: '0',
      MC_MAX_GENERATIONS: '4',
      MC_REVIEW: '0',
    },
  });
  let log = '';
  child.stderr.on('data', (c) => (log = (log + c.toString()).slice(-10000)));
  const base = await new Promise((resolve, reject) => {
    let out = '';
    const timer = setTimeout(() => {
      child.kill();
      reject(new Error(`Startup timeout: ${log}`));
    }, 30000);
    child.stdout.on('data', (chunk) => {
      out += chunk.toString();
      const m = out.match(/em (http:\/\/127\.0\.0\.1:\d+)/);
      if (m) {
        clearTimeout(timer);
        resolve(m[1]);
      }
    });
    child.once('exit', (code) => {
      clearTimeout(timer);
      reject(new Error(`Exit ${code}: ${log}`));
    });
  });

  const mk = async (routeId) => {
    const r = await fetch(`${base}/api/chats`, {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({ name: routeId, routeId, presetId: 'generic' }),
    });
    assert.equal(r.status, 201);
    return r.json();
  };
  const chatA1 = await mk('custom/alpha');
  const chatA2 = await mk('custom/beta');
  const chatB = await mk('srvB:custom/gamma');

  let releaseA1;
  routerA.set({ hold: new Promise((r) => (releaseA1 = r)), chunks: ['Lia: A ok.'] });
  routerB.set({ chunks: ['Lia: B paralelo ok.'] });

  const send = (id, opId) =>
    fetch(`${base}/api/chats/${id}/message`, {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({ mode: 'send', content: 'oi', opId }),
    }).then(readSse);

  const pA1 = send(chatA1.id, 'op-a1');
  const deadline = Date.now() + 15000;
  while (routerA.requests.length < 1 && Date.now() < deadline) {
    await new Promise((r) => setTimeout(r, 20));
  }
  assert.equal(routerA.requests.length, 1);

  const pA2 = send(chatA2.id, 'op-a2');
  await new Promise((r) => setTimeout(r, 120));
  assert.equal(routerA.requests.length, 1, 'second request to srvA must queue behind first');

  const evB = await send(chatB.id, 'op-b');
  assert.equal(evB.at(-1).status, 'complete');
  assert.equal(routerB.requests.length, 1);
  assert.equal(routerB.requests[0].body.model, 'custom/gamma');
  assert.equal(routerA.requests.length, 1);

  routerA.set({ hold: null, chunks: ['Lia: A2 ok.'] });
  releaseA1();
  const [evA1, evA2] = await Promise.all([pA1, pA2]);
  assert.equal(evA1.at(-1).status, 'complete');
  assert.equal(evA2.at(-1).status, 'complete');
  assert.equal(routerA.requests.length, 2);
});

test('resposta SSE vazia (200 OK sem delta) faz 1 POST, retorna empty e libera permissão de concorrência', async (t) => {
  const dir = await fs.mkdtemp(path.join(os.tmpdir(), 'mc-m2-empty-'));
  const router = await mockRouter();
  let runtime = null;
  t.after(async () => {
    if (runtime) await runtime.close();
    await router.close();
    await fs.rm(dir, { recursive: true, force: true });
  });
  runtime = await startRust({ dataDir: dir, upstream: router.url, review: false });

  const cRes = await fetch(`${runtime.base}/api/chats`, {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify({ name: 'Empty stream test', routeId: 'cbai/deepseek-v4.1-flash', presetId: 'deepseek-v41-flash' }),
  });
  assert.equal(cRes.status, 201);
  const c = await cRes.json();

  router.set({ finish: 'stop', chunks: [] });
  const r1 = await fetch(`${runtime.base}/api/chats/${c.id}/message`, {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify({ mode: 'send', content: 'oi', opId: 'empty-1' }),
  });
  const ev1 = await readSse(r1);
  assert.equal(ev1.at(-1).status, 'empty');
  assert.equal(router.requests.length, 1, 'empty stream must not retry');

  router.set({ finish: 'stop', chunks: ['Lia: Recuperou.'] });
  const r2 = await fetch(`${runtime.base}/api/chats/${c.id}/message`, {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify({ mode: 'send', content: 'oi 2', opId: 'empty-2' }),
  });
  const ev2 = await readSse(r2);
  assert.equal(ev2.at(-1).status, 'complete');
  assert.equal(router.requests.length, 2);
});
