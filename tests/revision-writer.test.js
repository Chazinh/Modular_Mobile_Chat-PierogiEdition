import test from 'node:test';
import assert from 'node:assert/strict';
import { createRevisionWriter } from '../client/src/storage-writes.js';

test('createRevisionWriter: incrementa revisão, persiste em lote e sincroniza chamadores', async () => {
  let writes = 0;
  let inFlight = false;
  const writer = createRevisionWriter(async () => {
    assert.equal(inFlight, false, 'Não deve haver escritas concorrentes desordenadas');
    inFlight = true;
    writes++;
    await new Promise((r) => setTimeout(r, 10));
    inFlight = false;
  });

  // save(false) sem alterações prévias resolve imediatamente sem chamar write
  await writer.save(false);
  assert.equal(writes, 0);

  // Primeiro save com alteração
  const p1 = writer.save(true);
  // Concorrentes durante a escrita do p1
  const p2 = writer.save(true);
  const p3 = writer.save(true);

  await Promise.all([p1, p2, p3]);

  // Deve ter agrupado as escritas subsequentes em um número mínimo de rodadas
  assert.ok(writes >= 1 && writes <= 2, `Escritas executadas: ${writes}`);

  // Novo save(false) com tudo já durável não dispara nova escrita
  await writer.save(false);
  assert.ok(writes <= 2);
});

test('createRevisionWriter: falha rejeita os waiters pendentes e preserva revisão suja para retry', async () => {
  let attempts = 0;
  let fail = true;

  const writer = createRevisionWriter(async () => {
    attempts++;
    if (fail) throw new Error('Falha de disco simulada');
  });

  // Tentativa com falha
  await assert.rejects(
    () => writer.save(true),
    /Falha de disco simulada/
  );
  assert.equal(attempts, 1);

  // Nova tentativa mesmo com save(false) deve tentar novamente persistir a revisão suja
  fail = false;
  await writer.save(false);
  assert.equal(attempts, 2, 'Deveria retentar a escrita pendente');
});
