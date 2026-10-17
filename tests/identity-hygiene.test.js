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

test('identity-hygiene: .gitignore bloqueia binarios compilados, dados de chat e relatorios locais', () => {
  const gitignorePath = path.join(rootDir, '.gitignore');
  const content = fs.readFileSync(gitignorePath, 'utf-8');

  // Binarios compilados do servidor e Android
  assert.ok(content.includes('*.exe'), '.gitignore deve conter *.exe');
  assert.ok(content.includes('*.pdb'), '.gitignore deve conter *.pdb');
  assert.ok(content.includes('*.dll'), '.gitignore deve conter *.dll');
  assert.ok(content.includes('*.so'), '.gitignore deve conter *.so');
  assert.ok(content.includes('target/'), '.gitignore deve conter target/');
  assert.ok(content.includes('server-rust/target/'), '.gitignore deve conter server-rust/target/');
  assert.ok(content.includes('*.apk'), '.gitignore deve conter *.apk');
  assert.ok(content.includes('*.aab'), '.gitignore deve conter *.aab');
  assert.ok(content.includes('modularchat-server'), '.gitignore deve conter modularchat-server');
  assert.ok(content.includes('client/android/app/src/main/jniLibs/'), '.gitignore deve bloquear jniLibs');

  // Dados pessoais de chat, cofre e bancos de dados
  assert.ok(content.includes('data/'), '.gitignore deve conter data/');
  assert.ok(content.includes('chats/'), '.gitignore deve conter chats/');
  assert.ok(content.includes('backups/'), '.gitignore deve conter backups/');
  assert.ok(content.includes('*.vault'), '.gitignore deve conter *.vault');
  assert.ok(content.includes('*.key'), '.gitignore deve conter *.key');
  assert.ok(content.includes('*.db'), '.gitignore deve conter *.db');
  assert.ok(content.includes('*.sqlite'), '.gitignore deve conter *.sqlite');
  assert.ok(content.includes('*-dados/'), '.gitignore deve conter *-dados/');
  assert.ok(content.includes('*-local/'), '.gitignore deve conter *-local/');

  // Relatorios de sessoes e dispositivos
  assert.ok(content.includes('*relatorio*') || content.includes('*Relatorio*'), '.gitignore deve conter regras de relatorios');

  // Pastas de backup, copias e arquivos temporarios
  assert.ok(content.includes('*backup*/'), '.gitignore deve conter *backup*/');
  assert.ok(content.includes('*copia*/'), '.gitignore deve conter *copia*/');
  assert.ok(content.includes('*.bak'), '.gitignore deve conter *.bak');
  assert.ok(content.includes('*.tmp'), '.gitignore deve conter *.tmp');
  assert.ok(content.includes('tmp/'), '.gitignore deve conter tmp/');
  assert.ok(content.includes('temp/'), '.gitignore deve conter temp/');
});

function getFilesRecursively(dir, filterFn) {
  let results = [];
  if (!fs.existsSync(dir)) return results;
  const list = fs.readdirSync(dir);
  for (const file of list) {
    const fullPath = path.join(dir, file);
    const stat = fs.statSync(fullPath);
    if (stat.isDirectory()) {
      results = results.concat(getFilesRecursively(fullPath, filterFn));
    } else if (!filterFn || filterFn(fullPath)) {
      results.push(fullPath);
    }
  }
  return results;
}

test('identity-hygiene: server-rust/src/local.rs usa caminho genérico ModularChat-local e não RoleplayChat-local', () => {
  const filePath = path.join(rootDir, 'server-rust', 'src', 'local.rs');
  const content = fs.readFileSync(filePath, 'utf-8');
  assert.ok(!content.includes('RoleplayChat-local'), 'local.rs não deve conter RoleplayChat-local');
  assert.ok(content.includes('ModularChat-local'), 'local.rs deve conter ModularChat-local');
});

test('identity-hygiene: CHANGELOG.md não contém RoleplayChat ou termos proibidos', () => {
  const filePath = path.join(rootDir, 'CHANGELOG.md');
  const content = fs.readFileSync(filePath, 'utf-8');
  assert.ok(!content.includes('RoleplayChat'), 'CHANGELOG.md não deve conter RoleplayChat');
  assert.ok(!content.toLowerCase().includes('victor'), 'CHANGELOG.md não deve conter victor');
});

test('identity-hygiene: código-fonte e scripts não contêm termos proibidos (victor ou roleplay)', () => {
  const targetDirs = [
    path.join(rootDir, 'server-rust', 'src'),
    path.join(rootDir, 'client', 'src'),
    path.join(rootDir, 'scripts'),
  ];
  const forbidden = ['victor', 'roleplay'];
  for (const dir of targetDirs) {
    const files = getFilesRecursively(dir, (f) => /\.(rs|js|jsx|ts|tsx|mjs|ps1|bat|json)$/.test(f));
    for (const file of files) {
      const content = fs.readFileSync(file, 'utf-8').toLowerCase();
      for (const term of forbidden) {
        assert.ok(
          !content.includes(term),
          `Arquivo ${path.relative(rootDir, file)} não deve conter termo proibido: ${term}`
        );
      }
    }
  }
});

