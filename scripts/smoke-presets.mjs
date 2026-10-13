// Real, limited end-to-end test: one message per route through the ModularChat server.
// Usage: node scripts/smoke-presets.mjs [http://127.0.0.1:3101]
import { parseScene } from '../client/src/format.js';

const base = process.argv[2] || 'http://127.0.0.1:3000';
const ROUTES = process.argv[3] ? process.argv[3].split(',') : ['cbai/deepseek-v4.1-flash', 'nvidia/deepseek-ai/deepseek-v4.1-flash', 'cc/claude-sonnet-5-5', 'ag/gemini-3.8-flash-low'];
const CARD = `Nome: Mira
Você é Mira, jovem maga de 19 anos da Academia Aurora. Arrogante por fora, insegura por dentro; provoca Nora, mas protege quem ama. Fala de forma elegante e sarcástica.

Nora: melhor amiga de Mira, divertida, leal e desastrada.

Cenário: pátio da Academia Aurora, fim de tarde, banco de pedra perto da fonte.`;
const j = async (method, path, body) => {
  const r = await fetch(base + path, { method, headers: { 'Content-Type': 'application/json' }, body: body ? JSON.stringify(body) : undefined });
  return { status: r.status, body: await r.json().catch(() => ({})) };
};

for (const route of ROUTES) {
  const c = (await j('POST', '/api/chats', { name: `Smoke ${route}`, prompt: CARD, routeId: route })).body;
  const t0 = Date.now();
  const res = await fetch(`${base}/api/chats/${c.id}/message`, {
    method: 'POST', headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ mode: 'send', content: '*Sento no banco entre as duas.* Oi, meninas. O que vocês estavam cochichando?', opId: `smoke-${Date.now()}` }),
  });
  let ackAt = null, firstDelta = null, done = null, text = '';
  const dec = new TextDecoder();
  let buf = '';
  for await (const chunk of res.body) {
    buf += dec.decode(chunk, { stream: true });
    let i;
    while ((i = buf.indexOf('\n\n')) >= 0) {
      const line = buf.slice(0, i).split('\n').find((l) => l.startsWith('data:'));
      buf = buf.slice(i + 2);
      if (!line) continue;
      const e = JSON.parse(line.slice(5));
      if (e.ack && !ackAt) ackAt = Date.now() - t0;
      if (e.delta) { firstDelta ??= Date.now() - t0; text += e.delta; }
      if (e.done) done = e;
    }
  }
  const lines = parseScene(text);
  const speakers = [...new Set(lines.filter((l) => l.kind === 'speech').map((l) => l.name))];
  // heuristic agency flags (speech/action of Alex written by the AI), computed by the Rust server itself
  const flags = (await j('GET', `/api/chats/${c.id}`)).body.history?.[1]?.flags || [];
  console.log(`\n== ${route}`);
  console.log(`  ack ${ackAt} ms | 1º texto ${firstDelta} ms | total ${Date.now() - t0} ms | status ${done?.status} ${done?.error || ''}`);
  console.log(`  tokens: entrada ${done?.usage?.input} (${done?.usage?.method?.input}), saída ${done?.usage?.output} (${done?.usage?.method?.output}), raciocínio ${done?.usage?.reasoning} (${done?.usage?.method?.reasoning})`);
  console.log(`  formato: ${lines.length} linhas (${lines.filter((l) => l.kind === 'action').length} ações, ${lines.filter((l) => l.kind === 'speech').length} falas, ${lines.filter((l) => l.kind === 'narration').length} narração sem asteriscos); falantes: ${speakers.join(', ')}`);
  console.log(`  agência de Alex: ${flags.length ? `PROBLEMA ${JSON.stringify(flags)}` : 'ok (nada escrito por ele)'}`);
  console.log(`  prévia: ${text.replace(/\n+/g, ' ⏎ ').slice(0, 260)}`);
  // check of the request sent to the provider (request log), done by the server right after the reply
  let prov = null;
  for (let i = 0; i < 20 && !prov; i++) {
    await new Promise((r) => setTimeout(r, 1000));
    prov = (await j('GET', `/api/chats/${c.id}`)).body.history?.[1]?.usage?.provider || null;
  }
  console.log(`  provedor: ${prov ? (prov.notFound ? 'não encontrado no registro' : `esforço enviado=${prov.effortSent} esperado=${prov.effortExpected} ${prov.effortOk ? 'OK' : 'DIVERGENTE'} · entrada real ${prov.usage?.input} (cache ${prov.usage?.cacheRead ?? 0})`) : 'sem verificação'}`);
  await j('DELETE', `/api/chats/${c.id}`);
}
