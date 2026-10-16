import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import http from 'node:http';
import { spawn, execFileSync } from 'node:child_process';

// Optional real-Caddy test with a fictional HTTP backend, random loopback ports and no TLS issuance.
const exe = process.env.MC_CADDY_EXE;
test('gateway Caddy: Bearer, login web, CORS, bloqueio de pareamento e SSE preservados', { skip: !exe && 'Defina MC_CADDY_EXE para validar com o Caddy instalado.' }, async t => {
  const dir = await fs.mkdtemp(path.join(os.tmpdir(), 'mc-gateway-test-'));
  let releaseStream, child;
  const operations = [];
  const backend = http.createServer(async (req, res) => {
    operations.push({ method: req.method, path: req.url, authorization: req.headers.authorization });
    if (req.method === 'OPTIONS') {
      res.statusCode = req.headers.origin === 'https://localhost' ? 204 : 403;
      return res.end();
    }
    if (req.url === '/api/privacy/cache-key') {
      res.writeHead(req.headers.authorization === 'Bearer fixture-token' ? 200 : 401, { 'Content-Type': 'application/json' });
      return res.end(JSON.stringify(req.headers.authorization === 'Bearer fixture-token' ? { key: 'fixture' } : { code: 'UNPAIRED' }));
    }
    if (req.url === '/api/events') {
      res.writeHead(200, { 'Content-Type': 'text/event-stream' });
      res.write('data: {"delta":"primeiro"}\n\n');
      await new Promise(resolve => { releaseStream = resolve; });
      return res.end('data: {"done":true}\n\n');
    }
    let body = '';
    for await (const chunk of req) body += chunk;
    res.writeHead(200, { 'Content-Type': 'application/json', 'Set-Cookie': 'mc_session=fixture; HttpOnly; SameSite=Strict; Path=/' });
    res.end(JSON.stringify({ ok: true, body }));
  });
  await new Promise(resolve => backend.listen(0, '127.0.0.1', resolve));
  const reservation = http.createServer();
  await new Promise(resolve => reservation.listen(0, '127.0.0.1', resolve));
  const port = reservation.address().port;
  await new Promise(resolve => reservation.close(resolve));
  t.after(async () => {
    releaseStream?.();
    if (child && child.exitCode === null) {
      const stopped = new Promise(resolve => child.once('exit', resolve));
      child.kill();
      await Promise.race([stopped, new Promise(resolve => setTimeout(resolve, 5000))]);
    }
    backend.closeAllConnections();
    await new Promise(resolve => backend.close(resolve));
    await fs.rm(dir, { recursive: true, force: true });
  });
  const hash = execFileSync(exe, ['hash-password', '--algorithm', 'argon2id'], { input: 'fixture-password\n', encoding: 'utf8', windowsHide: true, timeout: 15000 }).trim();
  const template = await fs.readFile(new URL('../hosting/Caddyfile.template', import.meta.url), 'utf8');
  let config = template;
  for (const [key, value] of Object.entries({
    ADMIN_ADDRESS: 'off', RUNTIME: dir.replaceAll('\\', '/'), BACKEND_PORT: backend.address().port,
    WEB_USER: 'fixture', PASSWORD_HASH: hash, SESSION_TOKEN: 'fixture-session',
    // Bootstrap endpoints must also work for a client outside the configured LAN.
    LAN_SUBNET: '', LOCAL_SITE: `http://127.0.0.1:${port} {\n bind 127.0.0.1\n import modularchat_gateway\n}`, LAN_SITE: '', PUBLIC_SITE: '',
  })) config = config.replaceAll(`__${key}__`, String(value));
  config = config.replace('remote_ip 127.0.0.1 ::1 ', 'remote_ip 192.0.2.0/24 ');
  const file = path.join(dir, 'Caddyfile');
  await fs.writeFile(file, config);
  execFileSync(exe, ['adapt', '--config', file, '--adapter', 'caddyfile'], { windowsHide: true, stdio: 'pipe', timeout: 15000 });
  child = spawn(exe, ['run', '--config', file, '--adapter', 'caddyfile'], { windowsHide: true, stdio: ['ignore', 'ignore', 'pipe'] });
  let logs = '';
  child.stderr.on('data', chunk => { logs = (logs + chunk).slice(-6000); });
  const base = `http://127.0.0.1:${port}`;
  const request = (endpoint, options = {}) => fetch(base + endpoint, { redirect: 'manual', signal: AbortSignal.timeout(5000), ...options });
  // This template writes startup messages to its private runtime log, not stderr.
  let ready = false;
  for (let attempt = 0; attempt < 100 && !ready; attempt++) {
    if (child.exitCode !== null) throw Error(`Caddy fixture exit ${child.exitCode}: ${logs}`);
    try {
      const check = await request('/api/health', { signal: AbortSignal.timeout(500) });
      ready = check.status === 200 && (await check.json()).ok === true;
    } catch { /* Only the disposable gateway is being awaited. */ }
    if (!ready) await new Promise(resolve => setTimeout(resolve, 50));
  }
  assert.ok(ready, `Caddy fixture startup timeout: ${logs}`);
  // No prior device credential is needed, including when the app retains a revoked token.
  for (const authorization of [undefined, 'Bearer revoked-device']) {
    for (const endpoint of ['/api/health', '/api/pair/info', '/api/pair']) {
      const beforeAuth = operations.filter(o => o.path === '/api/privacy/cache-key').length;
      const response = await request(endpoint, { method: endpoint === '/api/pair' ? 'POST' : 'GET',
        headers: { ...(authorization ? { Authorization: authorization } : {}), 'Content-Type': 'application/json' },
        ...(endpoint === '/api/pair' ? { body: JSON.stringify({ code: '123456' }) } : {}) });
      assert.equal(response.status, 200, `${endpoint} must reach the code verifier without prior pairing`);
      await response.arrayBuffer();
      assert.equal(operations.filter(o => o.path === '/api/privacy/cache-key').length, beforeAuth);
      assert.equal(operations.at(-1).path, endpoint);
    }
  }
  let response = await request('/api/chats');
  assert.equal(response.status, 401);
  assert.equal((await response.json()).code, 'WEB_LOGIN_REQUIRED');
  response = await request('/');
  assert.equal(response.status, 303);
  assert.equal(response.headers.get('location'), '/_host/login');
  response = await request('/api/chats', { headers: { Authorization: 'Bearer invalid-token' } });
  assert.equal(response.status, 401);
  assert.equal((await response.json()).code, 'UNPAIRED');
  const body = JSON.stringify({ opId: 'once', content: 'Texto fictício' });
  response = await request('/api/chats', { method: 'POST', headers: { Authorization: 'Bearer fixture-token' }, body });
  assert.equal(response.status, 200);
  assert.equal((await response.json()).body, body);
  assert.match(response.headers.get('set-cookie'), /Secure/);
  assert.equal(operations.filter(o => o.method === 'POST' && o.path === '/api/chats').length, 1);
  response = await request('/api/chats', { method: 'OPTIONS', headers: { Origin: 'https://localhost', 'Access-Control-Request-Headers': 'authorization' } });
  assert.equal(response.status, 204);
  response = await request('/api/chats', { method: 'OPTIONS', headers: { Origin: 'https://invalid.example.com' } });
  assert.equal(response.status, 403);
  response = await request('/api/pair/start', { method: 'POST', headers: { Authorization: 'Bearer fixture-token' } });
  assert.equal(response.status, 403);
  assert.equal(operations.filter(o => o.path === '/api/pair/start').length, 0);
  response = await request('/_host/login');
  assert.equal(response.status, 401);
  response = await request('/_host/login', { headers: { Authorization: `Basic ${Buffer.from('fixture:fixture-password').toString('base64')}` } });
  assert.equal(response.status, 303, `login returned ${response.status}, location=${response.headers.get('location')}, body=${await response.text()}; backend=${JSON.stringify(operations.slice(-3))}`);
  assert.match(response.headers.get('set-cookie'), /__Host-modularchat_gateway=fixture-session/);
  for (const flag of ['Secure', 'HttpOnly', 'SameSite=Strict']) assert.ok(response.headers.get('set-cookie').includes(flag));
  response = await request('/api/chats', { headers: { Cookie: '__Host-modularchat_gateway=fixture-session', Authorization: 'Basic fixture' } });
  assert.equal(response.status, 200);
  await response.arrayBuffer();
  assert.equal(operations.at(-1).authorization, undefined, 'Basic do login não substitui token do app');
  response = await request('/api/events', { headers: { Authorization: 'Bearer fixture-token' } });
  const reader = response.body.getReader();
  const first = await reader.read();
  assert.match(new TextDecoder().decode(first.value), /primeiro/);
  releaseStream();
  while (!(await reader.read()).done) { /* drain */ }
  reader.releaseLock();
});
