import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { execFileSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';
import { api, config, mergeWithDefaults, normalizeChatPreset } from '../client/src/api.js';

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const projectRoot = path.resolve(__dirname, '..');

test('Regressão: normalizeChatPreset promove Hydrogen e Argon salvos sob preset genérico sem afetar modelos desconhecidos', () => {
  assert.equal(
    normalizeChatPreset({ id: 'h1', presetId: 'generic', routeId: 'Hydrogen' }).presetId,
    'gemini-4-hydrogen'
  );
  assert.equal(
    normalizeChatPreset({ id: 'h2', presetId: null, routeId: 'ag/gemini-4-hydrogen-low' }).presetId,
    'gemini-4-hydrogen'
  );
  assert.equal(
    normalizeChatPreset({ id: 'a1', presetId: 'generic', routeId: 'Argon' }).presetId,
    'gemini-4-argon-medium'
  );
  assert.equal(
    normalizeChatPreset({ id: 'o1', presetId: 'generic', routeId: 'other/model' }).presetId,
    'generic'
  );
});

test('Regressão: api.presets não injeta presets ausentes do servidor conectado e createChat/updateChat não fazem fallback silencioso para generic', async () => {
  const originals = { window: globalThis.window, storage: globalThis.localStorage, fetch: globalThis.fetch };
  const store = new Map();
  const requests = [];

  globalThis.window = { Capacitor: { isNativePlatform: () => true }, dispatchEvent() {} };
  globalThis.localStorage = {
    getItem: (k) => store.get(k) || null,
    setItem: (k, v) => store.set(k, String(v)),
    removeItem: (k) => store.delete(k),
  };

  globalThis.fetch = async (url, options = {}) => {
    const parsed = new URL(url);
    requests.push({
      method: options.method || 'GET',
      path: parsed.pathname,
      body: options.body ? JSON.parse(options.body) : null,
    });

    if (parsed.pathname === '/api/health') {
      return new Response(JSON.stringify({ ok: true, name: 'modularchat', encrypted: true }), {
        status: 200,
        headers: { 'Content-Type': 'application/json' },
      });
    }

    if (parsed.pathname === '/api/presets') {
      return new Response(JSON.stringify({
        presets: [
          {
            id: 'deepseek-v41-flash',
            groupRoute: 'Deepseek',
            selectable: true,
            label: 'DeepSeek V4.1 Flash',
            family: 'deepseek',
            efforts: ['low', 'medium', 'high'],
            defaultEffort: 'high',
            routes: [{ id: 'Deepseek' }],
          },
          {
            id: 'generic',
            selectable: true,
            label: 'Outros modelos do servidor',
            family: 'generic',
            efforts: [],
            routes: [{ id: 'custom/model' }],
          },
        ],
      }), {
        status: 200,
        headers: { 'Content-Type': 'application/json' },
      });
    }

    if (parsed.pathname === '/api/chats' || parsed.pathname.startsWith('/api/chats/')) {
      return new Response(JSON.stringify({ error: 'Preset desconhecido.' }), {
        status: 400,
        headers: { 'Content-Type': 'application/json' },
      });
    }

    return new Response(JSON.stringify({ error: 'not found' }), {
      status: 404,
      headers: { 'Content-Type': 'application/json' },
    });
  };

  try {
    config.configureConnection({ local: 'https://pc.example.com' });

    const liveCatalog = await api.presets();
    assert.ok(
      !liveCatalog.presets.some((p) => p.id === 'gemini-4-hydrogen'),
      'api.presets não deve anunciar gemini-4-hydrogen quando o servidor conectado não o possui'
    );
    assert.ok(
      mergeWithDefaults({ presets: [{ id: 'deepseek-v41-flash', routes: [{ id: 'Deepseek' }] }] })
        .presets.some((p) => p.id === 'gemini-4-hydrogen'),
      'mergeWithDefaults padrão (cache offline) continua completando defaults'
    );

    const reqBeforeCreate = requests.length;
    await assert.rejects(
      () => api.createChat({ name: 'Teste', presetId: 'gemini-4-hydrogen', routeId: 'Hydrogen', effort: 'low' }),
      (err) => err.status === 400 && String(err.message).includes('Preset desconhecido')
    );
    const createCalls = requests.slice(reqBeforeCreate).filter((r) => r.path === '/api/chats');
    assert.equal(createCalls.length, 1, 'createChat não deve repetir a chamada fazendo fallback silencioso para generic');

    const reqBeforeUpdate = requests.length;
    await assert.rejects(
      () => api.updateChat('chat-hydrogen-1', { presetId: 'gemini-4-hydrogen', routeId: 'Hydrogen', effort: 'low' }),
      (err) => err.status === 400 && String(err.message).includes('Preset desconhecido')
    );
    const updateCalls = requests.slice(reqBeforeUpdate).filter((r) => r.path === '/api/chats/chat-hydrogen-1');
    assert.equal(updateCalls.length, 1, 'updateChat não deve repetir a chamada fazendo fallback silencioso para generic');
    assert.equal(await api.cachedChat('chat-hydrogen-1'), null, 'updateChat rejeitado não deve gravar presetId falso no cache');
  } finally {
    globalThis.window = originals.window;
    globalThis.localStorage = originals.storage;
    globalThis.fetch = originals.fetch;
  }
});

test('Regressão: start-server.ps1 detecta binário desatualizado quando fontes Rust ou presets.json mudam', { skip: process.platform !== 'win32' }, () => {
  const tmpDir = fs.mkdtempSync(path.join(os.tmpdir(), 'mc-stale-build-'));
  try {
    const startPs1 = path.join(projectRoot, 'start-server.ps1');
    const srcDir = path.join(tmpDir, 'server-rust', 'src');
    const releaseDir = path.join(tmpDir, 'target', 'release');
    fs.mkdirSync(srcDir, { recursive: true });
    fs.mkdirSync(releaseDir, { recursive: true });

    const presetsJson = path.join(srcDir, 'presets.json');
    const exePath = path.join(releaseDir, 'modularchat-server.exe');
    fs.writeFileSync(presetsJson, '[]');

    const tOld = new Date(Date.now() - 60_000);
    const tBin = new Date(Date.now() - 30_000);
    const tNew = new Date();
    fs.utimesSync(presetsJson, tOld, tOld);

    // 1. Missing binary -> stale ($true)
    // 2. Binary newer than presets.json -> fresh ($false)
    // 3. presets.json newer than binary -> stale ($true)
    const psScript = `
      $ErrorActionPreference = 'Stop'
      $tokens = $null; $errors = $null
      $ast = [Management.Automation.Language.Parser]::ParseFile('${startPs1.replace(/'/g, "''")}', [ref]$tokens, [ref]$errors)
      if ($errors.Count) { throw $errors[0].Message }
      $fn = $ast.Find({ param($n) $n -is [Management.Automation.Language.FunctionDefinitionAst] -and $n.Name -eq 'Test-ServerBuildStale' }, $true)
      if (-not $fn) { throw 'Test-ServerBuildStale nao encontrada' }
      Invoke-Expression $fn.Extent.Text
      $r1 = Test-ServerBuildStale -BuildPath '${exePath.replace(/'/g, "''")}' -ProjectRoot '${tmpDir.replace(/'/g, "''")}'
      [IO.File]::WriteAllText('${exePath.replace(/'/g, "''")}', 'exe')
      [IO.File]::SetLastWriteTimeUtc('${exePath.replace(/'/g, "''")}', [DateTime]::Parse('${tBin.toISOString()}').ToUniversalTime())
      $r2 = Test-ServerBuildStale -BuildPath '${exePath.replace(/'/g, "''")}' -ProjectRoot '${tmpDir.replace(/'/g, "''")}'
      [IO.File]::SetLastWriteTimeUtc('${presetsJson.replace(/'/g, "''")}', [DateTime]::Parse('${tNew.toISOString()}').ToUniversalTime())
      $r3 = Test-ServerBuildStale -BuildPath '${exePath.replace(/'/g, "''")}' -ProjectRoot '${tmpDir.replace(/'/g, "''")}'
      if (-not $r1 -or $r2 -or -not $r3) { throw "Unexpected results: $r1 $r2 $r3" }
      Write-Output "OK:$r1,$r2,$r3"
    `;
    const out = execFileSync('powershell.exe', ['-NoProfile', '-ExecutionPolicy', 'Bypass', '-Command', psScript], { encoding: 'utf8' });
    assert.match(out, /OK:True,False,True/);
  } finally {
    fs.rmSync(tmpDir, { recursive: true, force: true });
  }
});
