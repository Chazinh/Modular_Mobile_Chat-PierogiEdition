import test from 'node:test';
import assert from 'node:assert/strict';
import { isLocalAddress, normalizeServer, normalizePcServer } from '../client/src/server-address.js';

test('isLocalAddress: identifica loopback, redes locais e descarta IPs públicos', () => {
  // Loopback e mDNS
  assert.equal(isLocalAddress('localhost'), true);
  assert.equal(isLocalAddress('127.0.0.1'), true);
  assert.equal(isLocalAddress('::1'), true);
  assert.equal(isLocalAddress('[::1]'), true);
  assert.equal(isLocalAddress('pc.local'), true);
  assert.equal(isLocalAddress('meu-notebook.LOCAL'), true);

  // Faixa 10.0.0.0/8
  assert.equal(isLocalAddress('10.0.0.1'), true);
  assert.equal(isLocalAddress('10.200.1.50'), true);
  assert.equal(isLocalAddress('10.255.255.255'), true);

  // Faixa 192.168.0.0/16
  assert.equal(isLocalAddress('192.168.0.1'), true);
  assert.equal(isLocalAddress('192.168.1.105'), true);
  assert.equal(isLocalAddress('192.168.254.254'), true);

  // Faixa 172.16.0.0/12 (172.16.x.x até 172.31.x.x)
  assert.equal(isLocalAddress('172.16.0.1'), true);
  assert.equal(isLocalAddress('172.20.10.5'), true);
  assert.equal(isLocalAddress('172.31.255.255'), true);
  // Fora da faixa 172.16-31
  assert.equal(isLocalAddress('172.15.255.255'), false);
  assert.equal(isLocalAddress('172.32.0.1'), false);

  // IPs e domínios públicos
  assert.equal(isLocalAddress('8.8.8.8'), false);
  assert.equal(isLocalAddress('1.1.1.1'), false);
  assert.equal(isLocalAddress('api.openai.com'), false);
  assert.equal(isLocalAddress('modularchat.example.com'), false);

  // Casos nulos ou vazios
  assert.equal(isLocalAddress(null), false);
  assert.equal(isLocalAddress(''), false);
  assert.equal(isLocalAddress(undefined), false);
});

test('normalizeServer: validação de formato, esquemas e isolamento de caminhos', () => {
  // Valores vazios
  assert.equal(normalizeServer(''), '');
  assert.equal(normalizeServer(null), '');
  assert.equal(normalizeServer('   '), '');

  // Endereço HTTPS público normalizado
  assert.equal(normalizeServer('https://chat.meudominio.com'), 'https://chat.meudominio.com');
  assert.equal(normalizeServer('chat.meudominio.com'), 'https://chat.meudominio.com');

  // Endereço local com httpsOnly = false permite HTTP
  assert.equal(normalizeServer('http://192.168.1.50:3000', false), 'http://192.168.1.50:3000');
  assert.equal(normalizeServer('192.168.1.50:3000', false), 'http://192.168.1.50:3000');

  // Endereço público com HTTP é rejeitado
  assert.throws(
    () => normalizeServer('http://chat.meudominio.com', false),
    /Use HTTPS para endereços públicos/
  );
  // Endereço local com httpsOnly = true (padrão) exige HTTPS
  assert.throws(
    () => normalizeServer('http://192.168.1.50:3000', true),
    /Use HTTPS para endereços públicos/
  );

  // Rejeita caminhos adicionais, query params, hash e credenciais
  assert.throws(
    () => normalizeServer('https://chat.meudominio.com/api/v1'),
    /Informe somente o endereço do PC, sem senha, caminho ou parâmetros/
  );
  assert.throws(
    () => normalizeServer('https://chat.meudominio.com?foo=bar'),
    /Informe somente o endereço do PC, sem senha, caminho ou parâmetros/
  );
  assert.throws(
    () => normalizeServer('https://chat.meudominio.com#section'),
    /Informe somente o endereço do PC, sem senha, caminho ou parâmetros/
  );
  assert.throws(
    () => normalizeServer('https://user:pass@chat.meudominio.com'),
    /Informe somente o endereço do PC, sem senha, caminho ou parâmetros/
  );
});

test('normalizePcServer: impede que o celular aponte para si mesmo via loopback e orienta uso de Wi-Fi', () => {
  // Vazio
  assert.equal(normalizePcServer(''), '');
  assert.equal(normalizePcServer(null), '');

  // Tentativas de apontar para loopback local no celular
  const loopbackHosts = [
    'localhost',
    'LOCALHOST',
    'app.localhost',
    '127.0.0.1',
    '127.0.1.1',
    '127.255.255.255',
    '0.0.0.0',
    '[::1]',
    '[::]',
    '[::ffff:7f00:1]',
  ];

  for (const host of loopbackHosts) {
    assert.throws(
      () => normalizePcServer(`http://${host}:3000`, false),
      /Este endereço aponta para o próprio celular\. Use o IP do PC na rede Wi-Fi/,
      `Esperado erro para host loopback: ${host}`
    );
  }

  // Endereço válido do PC na rede Wi-Fi local
  assert.equal(
    normalizePcServer('http://192.168.1.100:3000', false),
    'http://192.168.1.100:3000'
  );
  assert.equal(
    normalizePcServer('192.168.1.100:3000', false),
    'http://192.168.1.100:3000'
  );

  // Endereço HTTPS do PC (ex: via proxy reverso ou túnel remoto seguro)
  assert.equal(
    normalizePcServer('https://pc.meudominio.com', true),
    'https://pc.meudominio.com'
  );
});
