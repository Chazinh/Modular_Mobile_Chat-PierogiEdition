import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import { composerHoldsSent } from '../client/src/ui-helpers.js';

const read = path => fs.readFileSync(new URL(`../${path}`, import.meta.url), 'utf8');

test('regressão: confirmação do envio só limpa a caixa se ela ainda mostra a mensagem enviada', () => {
  assert.equal(composerHoldsSent('Oi', 'Oi'), true);
  assert.equal(composerHoldsSent('  Oi \n', 'Oi'), true);
  assert.equal(composerHoldsSent('Oi\nE mais uma coisa', 'Oi'), false, 'texto digitado durante o envio não pode sumir');
  assert.equal(composerHoldsSent('', 'Oi'), false);
  const chat = read('client/src/components/ChatView.jsx');
  assert.match(chat, /composerRef\.current\?\.clear\(ctx\.payload\.content\)/);
  assert.match(chat, /if \(sent != null && !composerHoldsSent\(valueRef\.current, sent\)\) return;/);
});

test('regressão: falha ao limpar o recibo não deixa o chat travado em "ocupado"', () => {
  const chat = read('client/src/components/ChatView.jsx');
  const finish = chat.slice(chat.indexOf('async function finishRun'), chat.indexOf('async function reattach'));
  assert.match(finish, /await api\.setPending\(chatId, null\)\.catch\(/);
  assert.ok(finish.indexOf('.catch(') < finish.indexOf('busyRef.current = false'));
});

test('regressão: pareamento guarda o token antes de abrir a proteção do cache', () => {
  const api = read('client/src/api.js');
  const pair = api.slice(api.indexOf('async pair('), api.indexOf('startPairing'));
  assert.ok(pair.indexOf('privateSet(LS_TOKEN') < pair.indexOf('initializePrivacy('), 'token salvo primeiro');
});

test('regressão: start-server.ps1 não derruba outros servidores e ignora conexões em TIME_WAIT', () => {
  const script = read('start-server.ps1');
  assert.doesNotMatch(script, /Filter "Name='modularchat-server\.exe'"/, 'matava a produção ao subir o -Sandbox');
  for (const line of script.split(/\r?\n/).filter(l => /Get-NetTCPConnection -LocalPort \$Port/.test(l))) {
    assert.match(line, /-State Listen/, line);
  }
  assert.match(script, /\$env:MC_ALLOWED_HOSTS = /, 'domínio do gateway liberado contra DNS rebinding');
});

test('regressão: "Sair" na biometria encerra o host local do Android', () => {
  const activity = read('client/android/app/src/main/java/io/github/modularchat/pierogiedition/MainActivity.java');
  assert.match(activity, /private void exitLocked\(\) \{\s*stopLocalHostService\(\);\s*finish\(\);/);
  assert.match(activity, /ERROR_USER_CANCELED\)\) exitLocked\(\);/);
  assert.match(activity, /"Sair", "Exit"\), \(d, w\) -> exitLocked\(\)\)/);
});
