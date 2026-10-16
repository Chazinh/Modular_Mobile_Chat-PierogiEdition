import test from 'node:test';
import assert from 'node:assert/strict';
import { createLocalHost } from '../client/src/local-host.js';

test('localHost: supported reflete native e available', () => {
  const hostNotNative = createLocalHost({
    native: () => false,
    available: () => true,
    plugin: {},
    fetch: () => {},
  });
  assert.equal(hostNotNative.supported(), false);

  const hostNotAvailable = createLocalHost({
    native: () => true,
    available: () => false,
    plugin: {},
    fetch: () => {},
  });
  assert.equal(hostNotAvailable.supported(), false);

  const hostOk = createLocalHost({
    native: () => true,
    available: () => true,
    plugin: {},
    fetch: () => {},
  });
  assert.equal(hostOk.supported(), true);
});

test('localHost ensureStarted: validação de token, endereço e deduplicação de chamadas concorrentes', async () => {
  const notifications = [];
  let pluginCalls = 0;

  const validToken = 'super-secret-local-token-12345';
  const mockPlugin = {
    async ensureStarted() {
      pluginCalls++;
      return {
        baseUrl: 'http://127.0.0.1:3011',
        status: 2,
        token: validToken,
      };
    },
  };

  const host = createLocalHost({
    native: () => true,
    available: () => true,
    plugin: mockPlugin,
    fetch: async () => {},
  });

  const unsubscribe = host.subscribe((state) => notifications.push(state));

  // Chamadas concorrentes devem compartilhar a mesma Promise
  const [res1, res2] = await Promise.all([
    host.ensureStarted(),
    host.ensureStarted(),
  ]);

  assert.equal(pluginCalls, 1, 'Plugin ensureStarted deve ser chamado uma única vez');
  assert.deepEqual(res1, { server: 'http://127.0.0.1:3011', token: validToken, kind: 'local' });
  assert.equal(res1, res2);
  assert.deepEqual(host.target(), res1);

  // Notificações: starting e ready
  assert.equal(notifications.length, 2);
  assert.equal(notifications[0].phase, 'starting');
  assert.equal(notifications[1].phase, 'ready');

  // Próxima chamada síncrona/imediata reutiliza ready sem invocar plugin
  const cached = await host.ensureStarted();
  assert.equal(pluginCalls, 1);
  assert.equal(cached, res1);

  unsubscribe();
});

test('localHost ensureStarted: rejeita tokens maliciosos ou payload inválido com INVALID_LOCAL_HOST', async () => {
  const badPayloads = [
    { baseUrl: 'http://127.0.0.1:3011', status: 2, token: '' }, // token vazio
    { baseUrl: 'http://127.0.0.1:3011', status: 2, token: '   ' }, // apenas espaços
    { baseUrl: 'http://127.0.0.1:3011', status: 2, token: 'token\r\ninjection' }, // injeção de cabeçalho
    { baseUrl: 'http://127.0.0.1:3011', status: 1, token: 'valido' }, // status diferente de 2
    { baseUrl: 'http://localhost:3011', status: 2, token: 'valido' }, // baseUrl não exata
    { baseUrl: 'http://127.0.0.1:8080', status: 2, token: 'valido' }, // porta incorreta
    null,
  ];

  for (const bad of badPayloads) {
    const host = createLocalHost({
      native: () => true,
      available: () => true,
      plugin: { ensureStarted: async () => bad },
      fetch: async () => {},
    });

    await assert.rejects(
      () => host.ensureStarted(),
      (err) => err.code === 'INVALID_LOCAL_HOST',
      `Deveria rejeitar payload: ${JSON.stringify(bad)}`
    );
  }
});

test('localHost request, status, activate e standby: autenticação e transições de ciclo de vida', async () => {
  const fetchCalls = [];
  const mockFetch = async (url, options) => {
    fetchCalls.push({ url, options });
    return {
      ok: true,
      status: 200,
      async json() {
        if (url.endsWith('/status')) return { running: true, models: ['embed'] };
        return { success: true };
      },
    };
  };

  const host = createLocalHost({
    native: () => true,
    available: () => true,
    plugin: {
      async ensureStarted() {
        return { baseUrl: 'http://127.0.0.1:3011', status: 2, token: 'token-abc' };
      },
    },
    fetch: mockFetch,
  });

  // Chamar antes de iniciar deve falhar
  await assert.rejects(
    () => host.status(),
    (err) => err.code === 'LOCAL_HOST_NOT_READY'
  );

  await host.ensureStarted();

  // Teste de status
  const st = await host.status();
  assert.equal(st.running, true);
  assert.equal(fetchCalls[0].url, 'http://127.0.0.1:3011/api/local/status');
  assert.equal(fetchCalls[0].options.headers.Authorization, 'Bearer token-abc');

  // Transições de activate e standby
  await host.activate();
  assert.equal(fetchCalls[1].url, 'http://127.0.0.1:3011/api/local/activate');

  // Chamar activate novamente enquanto já ativo é deduplicado
  await host.activate();
  assert.equal(fetchCalls.length, 2);

  // Standby transiciona
  await host.standby();
  assert.equal(fetchCalls[2].url, 'http://127.0.0.1:3011/api/local/standby');

  // InvalidateLifecycle força nova transição
  host.invalidateLifecycle();
  await host.standby();
  assert.equal(fetchCalls[3].url, 'http://127.0.0.1:3011/api/local/standby');
});
