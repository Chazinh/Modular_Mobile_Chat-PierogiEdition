import test from 'node:test';
import assert from 'node:assert/strict';
import { execFile } from 'node:child_process';
import { promisify } from 'node:util';
import { fileURLToPath } from 'node:url';

test('firewall: suporte LAN pública/privada, escopo mínimo e regra idempotente sem alterar o Windows', { skip: process.platform !== 'win32' }, async () => {
  const result = await promisify(execFile)('powershell.exe', ['-NoProfile', '-NonInteractive', '-ExecutionPolicy', 'Bypass', '-File', fileURLToPath(new URL('./firewall-config.ps1', import.meta.url))], { windowsHide: true, timeout: 15000, maxBuffer: 16000 });
  assert.match(result.stdout, /Firewall fixture passed/);
});
