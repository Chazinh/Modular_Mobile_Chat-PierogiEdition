import test from 'node:test';
import assert from 'node:assert/strict';
import { execFile } from 'node:child_process';
import { promisify } from 'node:util';
import { fileURLToPath } from 'node:url';

test('start-server: suporte e inicialização do 9Router em start-server.ps1 e start-server.bat', { skip: process.platform !== 'win32' }, async () => {
  const result = await promisify(execFile)(
    process.env.MC_POWERSHELL_EXE || 'powershell.exe',
    ['-NoProfile', '-NonInteractive', '-ExecutionPolicy', 'Bypass', '-File', fileURLToPath(new URL('./start-server-9router.ps1', import.meta.url))],
    { windowsHide: true, timeout: 60000, maxBuffer: 128 * 1024 }
  );
  assert.match(result.stdout, /9Router start server tests passed/);
});
