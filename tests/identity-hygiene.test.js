import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const __filename = fileURLToPath(import.meta.url);
const __dirname = path.dirname(__filename);
const rootDir = path.resolve(__dirname, '..');

test('identity-hygiene: scripts/build-android-llama.ps1 usa diretório genérico ModularChat-local', () => {
  const filePath = path.join(rootDir, 'scripts', 'build-android-llama.ps1');
  const content = fs.readFileSync(filePath, 'utf-8');
  assert.ok(!content.includes('RoleplayChat-local'), 'build-android-llama.ps1 não deve conter RoleplayChat-local');
  assert.ok(content.includes('ModularChat-local'), 'build-android-llama.ps1 deve conter ModularChat-local');
});

test('identity-hygiene: client/src/locales/en.json não contém termos legados de roleplay', () => {
  const filePath = path.join(rootDir, 'client', 'src', 'locales', 'en.json');
  const content = fs.readFileSync(filePath, 'utf-8');
  assert.ok(!content.toLowerCase().includes('roleplay'), 'en.json não deve conter roleplay');
  assert.ok(content.includes('Persona (your name in the chat)'), 'en.json deve conter Persona (your name in the chat)');
});

test('identity-hygiene: tests/fixtures/world-card.txt usa terminologia genérica', () => {
  const filePath = path.join(rootDir, 'tests', 'fixtures', 'world-card.txt');
  const content = fs.readFileSync(filePath, 'utf-8');
  assert.ok(!content.toLowerCase().includes('roleplay'), 'world-card.txt não deve conter roleplay');
  assert.ok(content.includes('Diretrizes de Conversação e Comportamento'), 'world-card.txt deve iniciar com Diretrizes de Conversação e Comportamento');
});

test('identity-hygiene: client/android/app/build.gradle usa namespace e applicationId genéricos', () => {
  const filePath = path.join(rootDir, 'client', 'android', 'app', 'build.gradle');
  const content = fs.readFileSync(filePath, 'utf-8');
  assert.ok(content.includes('io.github.modularchat.pierogiedition'), 'build.gradle deve usar namespace io.github.modularchat.pierogiedition');
  assert.ok(!content.toLowerCase().includes('victor'), 'build.gradle não deve conter victor');
  assert.ok(!content.toLowerCase().includes('roleplay'), 'build.gradle não deve conter roleplay');
});

test('identity-hygiene: client/android/private-build.properties se presente usa identificador e versão genéricos', () => {
  const filePath = path.join(rootDir, 'client', 'android', 'private-build.properties');
  if (fs.existsSync(filePath)) {
    const content = fs.readFileSync(filePath, 'utf-8');
    assert.ok(!content.toLowerCase().includes('victor'), 'private-build.properties não deve conter victor');
    assert.ok(!content.toLowerCase().includes('roleplay'), 'private-build.properties não deve conter roleplay');
    assert.ok(content.includes('io.github.modularchat.pierogiedition'), 'private-build.properties deve usar o identificador genérico');
  }
});
