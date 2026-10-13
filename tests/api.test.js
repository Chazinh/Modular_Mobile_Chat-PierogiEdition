import test, { before, after } from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { DatabaseSync } from 'node:sqlite';
import { startRust } from './rust-http.js';
import { mockRouter } from './helpers.js';
import { randomUUID } from 'node:crypto';
import { mira, previousBot, currentUser, falseBot } from './provenance-fixture.js';

const dataDir = fs.mkdtempSync(path.join(os.tmpdir(), 'mc-api-'));
const router = await mockRouter();
process.env.DATA_DIR = dataDir;
process.env.NINEROUTER_URL = router.url;
process.env.NINEROUTER_API_KEY = 'sk-test-key';

let base, gen, rustServer, logDb;
const providerOverride = { effort: undefined };

before(async () => {
  const file=path.join(dataDir,'mock-router.db');logDb=new DatabaseSync(file);logDb.exec('CREATE TABLE requestDetails(timestamp TEXT,provider TEXT,model TEXT,data TEXT)');
  router.set({onRequest:hit=>{const effort=providerOverride.omitControls?undefined:providerOverride.effort??hit.body.reasoning_effort??null;const off=hit.body.reasoning_effort==='none';logDb.prepare('INSERT INTO requestDetails VALUES(?,?,?,?)').run(new Date().toISOString(),'fake',hit.body.model,JSON.stringify({request:hit.body,providerRequest:{reasoning_effort:effort,output_config:{effort},thinking:off&&!providerOverride.omitControls?{type:hit.body.model.includes('sonnet')?'between_tools':'disabled'}:undefined,max_tokens:hit.body.max_tokens},tokens:{prompt_tokens:952,completion_tokens:20,reasoning_tokens:off?0:300,cached_tokens:800}}));}});
  rustServer=await startRust({dataDir,upstream:router.url,routerDb:file});base=rustServer.base;
  gen={isBusy:async id=>!!(await j('GET',`/api/chats/${id}`)).body.generation,get:async id=>{
    const chat=(await j('GET',`/api/chats/${id}`)).body;if(!chat.generation)return null;
    const db=new DatabaseSync(path.join(dataDir,'modularchat.db'),{readOnly:true});try{const v=db.prepare('SELECT content FROM versions WHERE generation_id=?').get(chat.generation.id);return {...chat.generation,buffer:v?.content||''};}finally{db.close();}
  }};
});

after(async () => {
  await rustServer?.close();logDb?.close();await router.close();fs.rmSync(dataDir,{recursive:true,force:true});
});

const DEFAULTS = () => ({
  chunks: ['*Mira sorri.*\n\n', 'Mira: Oi, Alex.'], reasoning: [], delayMs: 5, status: 200, hold: null, finish: 'stop',
  usage: { prompt_tokens: 1234, completion_tokens: 20, completion_tokens_details: { reasoning_tokens: 300 } },
});
const DEFAULT_STYLE = { botChars: 8000, dialoguePct: 65, content: '', lengthMode: 'maximum', monologueChars: 300, turnPacing: 'interactive' };

const j = async (method, p, body, headers = {}) => {
  const r = await fetch(base + p, { method, headers: { 'Content-Type': 'application/json', ...headers }, body: body ? JSON.stringify(body) : undefined });
  return { status: r.status, body: await r.json().catch(() => ({})), headers: r.headers };
};

// Reads the SSE to the end (or until abortAfterAck).
async function sse(p, body, { abortAfterAck = false } = {}) {
  const ctrl = new AbortController();
  const timer = setTimeout(()=>ctrl.abort(new Error('SSE fixture deadline')),15000);
  timer.unref();
  const res = await fetch(base + p, { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify(body), signal: ctrl.signal });
  if (!res.ok) return { status: res.status, error: (await res.json()).error, events: [] };
  const out = { status: 200, events: [] };
  const reader = res.body.getReader();
  const dec = new TextDecoder();
  let buf = '';
  try {
    for (;;) {
      const { value, done } = await reader.read();
      if (done) break;
      buf += dec.decode(value, { stream: true });
      let i;
      while ((i = buf.indexOf('\n\n')) >= 0) {
        const line = buf.slice(0, i).split('\n').find((l) => l.startsWith('data:'));
        buf = buf.slice(i + 2);
        if (!line) continue;
        const e = JSON.parse(line.slice(5));
        out.events.push(e);
        if (abortAfterAck && e.ack) {
          ctrl.abort();
          return out;
        }
      }
    }
  } catch (e) {
    if (e.name !== 'AbortError') throw e;
  } finally {
    clearTimeout(timer);
    await reader.cancel().catch(()=>{});
  }
  return out;
}

const mkChat = async (over = {}) => (await j('POST', '/api/chats', { name: 'Chat', prompt: 'Nome: Mira\nMira é uma maga arrogante.', ...over })).body;

test('Sonnet low e configuração global: uma revisão na mesma rota ajusta tamanho, proporção e idioma antes do done', async () => {
  const corrected='Mira: '+'Vamos examinar o endereço da carta antes de sair. '.repeat(9);
  router.set({ ...DEFAULTS(), correctionText: corrected, correctionStatus: null,
    chunks:['Mira: You said that she was there with the letter.'] });
  try {
    await j('PUT','/api/settings/global',{botChars:500,dialoguePct:100,content:'',lengthMode:'target',monologueChars:0});
    const c=await mkChat({routeId:'cc/claude-sonnet-5-5',effort:'low'});
    assert.equal(c.effort,'low');
    const n=router.correctionRequests.length;
    const result=await sse(`/api/chats/${c.id}/message`,{mode:'send',content:'O que encontrou?',opId:'style-repair-low'});
    const done=result.events.find(e=>e.done);
    assert.equal(done.message.content,corrected.trim());
    assert.equal(done.usage.style.after.ok,true);
    assert.equal(router.correctionRequests.length,n+1);
    const request=router.correctionRequests.at(-1).body;
    assert.equal(request.model,'cc/claude-sonnet-5-5');assert.equal(request.reasoning_effort,'low');
    assert.ok(request.messages.at(-1).content.includes('português do Brasil'));
    assert.ok(request.messages.at(-1).content.includes('425 e 500'));
  } finally {
    await j('PUT','/api/settings/global',DEFAULT_STYLE);
    router.set({...DEFAULTS(),correctionText:null,correctionStatus:null});
  }
});
const waitFor = async (fn, ms = 5000) => {
  const t0 = Date.now();
  for (;;) {
    const v = await fn();
    if (v) return v;
    if (Date.now() - t0 > ms) throw new Error('timeout aguardando condição');
    await new Promise((r) => setTimeout(r, 30));
  }
};

test('pensamento: chave persiste por conversa, orçamento cai quando desligado e esforço volta ao religar', async () => {
  router.set({ ...DEFAULTS(), chunks: ['Lia: Achei a carta.'] });
  try {
    await j('PUT', '/api/settings/global', { ...DEFAULT_STYLE, botChars: 600, dialoguePct: 100 });
    for (const [routeId, presetId, effort, reserve] of [
      ['cc/claude-sonnet-5-5', 'claude-sonnet-5-5', 'medium', 4096],
      ['cbai/deepseek-v4.1-flash', 'deepseek-v41-flash', 'high', 8192],
    ]) {
      const chat = await mkChat({routeId,presetId,effort,settings:{personaNotes:'Notas preservadas'}});
      const saved = await j('PUT', `/api/chats/${chat.id}`, { settings: { thinkingEnabled: false } });
      assert.equal(saved.body.thinkingEnabled, false);
      assert.equal(saved.body.effort, effort);
      assert.equal(saved.body.settings.personaNotes, 'Notas preservadas');
      assert.equal((await j('GET', `/api/chats/${chat.id}`)).body.settings.thinkingEnabled, false);
      const off = await sse(`/api/chats/${chat.id}/message`, {mode:'send',content:'O que encontrou?',opId:`off-${presetId}`});
      assert.equal(router.requests.at(-1).body.reasoning_effort, 'none');
      assert.equal(router.requests.at(-1).body.max_tokens, 240);
      assert.equal(off.events.at(-1).usage.length.thinkingEnabled, false);
      await waitFor(async () => (await j('GET', `/api/chats/${chat.id}`)).body.history.at(-1)?.usage?.provider?.thinkingOk === true);
      await j('PUT', `/api/chats/${chat.id}`, { settings: { thinkingEnabled: true } });
      const on = await sse(`/api/chats/${chat.id}/message`, {mode:'continue',opId:`on-${presetId}`});
      assert.equal(router.requests.at(-1).body.reasoning_effort, effort);
      assert.equal(on.events[0].ack.length.reasoningReserve, reserve);
    }
    const fixed = await mkChat({routeId:'cc/claude-opus-5-5',presetId:'claude-opus-5-5',settings:{thinkingEnabled:false}});
    assert.equal(fixed.thinkingEnabled, true, 'uma preferência antiga não desliga modelo obrigatório');
    providerOverride.omitControls = true;
    const opaque = await mkChat({routeId:'cbai/deepseek-v4.1-flash',settings:{thinkingEnabled:false}});
    await sse(`/api/chats/${opaque.id}/message`,{mode:'send',content:'Oi',opId:'opaque-thinking'});
    const provider = await waitFor(async () => (await j('GET', `/api/chats/${opaque.id}`)).body.history.at(-1)?.usage?.provider);
    assert.equal(provider.thinkingOk,null,'ausência de campo é falta de confirmação, não rejeição');
    assert.equal(provider.effortOk,null);
  } finally { providerOverride.omitControls=false;await j('PUT','/api/settings/global',DEFAULT_STYLE);router.set(DEFAULTS()); }
});

for (const [presetId, routeId] of [['deepseek-v41-flash','cbai/deepseek-v4.1-flash'],['claude-sonnet-5-5','cc/claude-sonnet-5-5']]) {
  test(`HTTP Mira/Alex: ${presetId} envia original integral antes da imitação e origem identificada`, async () => {
    router.set(DEFAULTS());
    const c = await mkChat({persona:'Alex',presetId,routeId});
    const exported = (await j('GET',`/api/chats/${c.id}/export`)).body;
    const data = exported.conversations[0];
    const iid=randomUUID(),pid=randomUUID(),vid=randomUUID(),time='2026-10-12T00:00:00.000Z';
    data.conversation.head_interaction_id=iid;
    data.interactions=[{id:iid,conv_id:c.id,parent_id:null,seq:1,kind:'opening',user_text:null,created_at:time}];
    data.parts=[{id:pid,interaction_id:iid,idx:0,selected_version_id:vid,created_at:time}];
    data.versions=[{id:vid,part_id:pid,content:previousBot,status:'complete',origin:'legacy',created_at:time,finished_at:time}];
    const imported=await j('POST','/api/import',exported);assert.equal(imported.status,201);
    const cid=imported.body.imported[0];
    const actual=(await j('GET',`/api/chats/${cid}/export`)).body.conversations[0];
    const sourceId=actual.parts[0].selected_version_id;
    router.set({chunks:[falseBot]});const before=router.requests.length;
    const reply=await sse(`/api/chats/${cid}/message`,{mode:'send',content:currentUser,opId:`mira-${presetId}`});
    assert.equal(reply.status,200);
    assert.equal(router.requests.length,before+1);
    const messages=router.requests.at(-1).body.messages;
    const originalAt=messages.findIndex(m=>m.role==='assistant'&&m.content.endsWith(previousBot));
    const currentAt=messages.findIndex(m=>m.role==='user'&&m.content.includes(currentUser));
    assert.ok(originalAt>=0&&currentAt>originalAt,'raw original is sent before current user');
    const all=messages.map(m=>m.content).join('\n');
    assert.ok(!/\{\{\s*persona\s*\}\}/i.test(all));
    assert.ok(all.includes('quem pronunciou originalmente')&&all.includes('ROLE=user identifica a fonte'));
    const provenance=JSON.parse(all.split('\n').find(line=>line.startsWith('[{')&&line.includes('"originalSpeaker"')));
    assert.equal(provenance[0].sourceId,sourceId);
    assert.equal(provenance[0].originalSpeaker,'Mira');
    assert.equal(provenance[0].citingSpeaker,null);
    assert.equal(provenance[0].use,'citation_or_imitation');
    assert.equal(provenance[0].quote,mira.quote);
    assert.ok(!provenance[0].quote.includes('Engraçadinha'));
    const saved=(await j('GET',`/api/chats/${cid}`)).body.history;
    assert.deepEqual(saved.map(m=>({role:m.role,content:m.content})),mira.messages);
  });
}

// Bounded style review: a worse result is rejected and a provider failure keeps the original reply.
for (const [name, over, accepted] of [
  ['correção pior que o original é rejeitada', { correctionText: 'x'.repeat(501), correctionStatus: null }, false],
  ['falha do provedor na correção preserva a resposta', { correctionText: null, correctionStatus: 503 }, false],
]) test(`revisão de estilo: ${name}`, async () => {
  const original = '*Ela abre uma carta.*';
  router.set({ ...DEFAULTS(), ...over, chunks: [original] });
  try {
    await j('PUT', '/api/settings/global', { botChars: 500, dialoguePct: 100, content: '' });
    const c = await mkChat({ routeId: 'cc/claude-sonnet-5-5', effort: 'low' });
    const n = router.correctionRequests.length;
    const result = await sse(`/api/chats/${c.id}/message`, { mode: 'send', content: 'O que encontrou?', opId: `style-repair-${accepted}-${over.correctionStatus}` });
    const done = result.events.find((e) => e.done);
    assert.equal(done.message.content, original, 'o texto original continua sendo a resposta salva');
    assert.equal(router.correctionRequests.length, n + 1, 'uma única tentativa de correção');
    assert.equal(done.usage.style.attempted, true);
    assert.equal(done.usage.style.accepted, accepted);
    assert.equal(done.usage.style.after.ok, false);
    if (over.correctionText) assert.equal(done.usage.style.candidate.characters, over.correctionText.length, 'o candidato rejeitado fica registrado');
    else assert.equal(done.usage.style.candidate, undefined);
  } finally {
    await j('PUT', '/api/settings/global', DEFAULT_STYLE);
    router.set({ ...DEFAULTS(), correctionText: null, correctionStatus: null });
  }
});

test('criação de chat: preset pela rota, esforço padrão, arquivo character.txt e elenco heurístico', async () => {
  const c = await mkChat();
  assert.equal(c.presetId, 'deepseek-v41-flash');
  assert.equal(c.effort, 'high');
  assert.equal(c.persona, 'Alex');
  const file = path.join(dataDir, 'chats', c.id, 'character.txt');
  assert.ok(fs.readFileSync(file, 'utf8').includes('maga arrogante'));
  const cast = (await j('GET', `/api/chats/${c.id}/cast`)).body.cast;
  assert.ok(cast.some((x) => x.name === 'Mira'));
  assert.ok(cast.some((x) => x.name === 'Alex' && x.kind === 'user'));
});

test('edição externa do character.txt é importada como nova revisão antes da próxima leitura', async () => {
  const c = await mkChat();
  fs.writeFileSync(path.join(dataDir, 'chats', c.id, 'character.txt'), 'Texto novo escrito no Bloco de Notas.');
  const chat = (await j('GET', `/api/chats/${c.id}`)).body;
  assert.equal(chat.prompt, 'Texto novo escrito no Bloco de Notas.');
  assert.equal(chat.characterRev, 2);
});

test('envio: confirmação (ack) chega ANTES da inferência; resposta em streaming é salva; tokens medidos', async () => {
  router.set(DEFAULTS());
  const c = await mkChat({ routeId: 'cbai/deepseek-v4.1-flash' });
  const before = router.requests.length;
  const r = await sse(`/api/chats/${c.id}/message`, { mode: 'send', content: 'Olá, Mira.', opId: 'op-a' });
  const kinds = r.events.map((e) => (e.ack ? 'ack' : e.delta ? 'delta' : e.done ? 'done' : 'other'));
  assert.equal(kinds[0], 'ack');
  assert.equal(kinds.at(-1), 'done');
  assert.ok(kinds.includes('delta'));
  const ack = r.events[0].ack;
  assert.equal(ack.userMessage.content, 'Olá, Mira.');
  assert.equal(ack.preset.id, 'deepseek-v41-flash');
  const done = r.events.at(-1);
  assert.equal(done.status, 'complete');
  assert.equal(done.message.content, '*Mira sorri.*\n\nMira: Oi, Alex.');
  assert.equal(done.usage.input, 1234);
  assert.equal(done.usage.method.input, 'measured');
  assert.equal(done.usage.reasoning, 300);
  const req = router.requests[before];
  assert.equal(req.body.model, 'cbai/deepseek-v4.1-flash');
  assert.equal(req.body.reasoning_effort, 'high');
  assert.equal('temperature' in req.body, false);
  assert.equal(req.body.stream, true);
  assert.equal(req.body.messages[0].role, 'user'); // cbai/ drops system: rules go as the first turn
  assert.match(req.body.messages[0].content, /Nome: fala/);
  assert.match(req.body.messages[0].content, /Mira é uma maga arrogante/);
  const last = req.body.messages.at(-1);
  assert.equal(last.role, 'user');
  assert.match(last.content, /^Olá, Mira\./);
  assert.match(last.content, /não escreva nada por Alex/);
  const chat = (await j('GET', `/api/chats/${c.id}`)).body;
  assert.deepEqual(chat.history.map((m) => m.role), ['user', 'assistant']);
  assert.equal(chat.history[0].content, 'Olá, Mira.', 'âncora de recência não é gravada na mensagem');
});

test('mesmo opId não duplica a mensagem nem chama o modelo de novo', async () => {
  router.set(DEFAULTS());
  const c = await mkChat();
  await sse(`/api/chats/${c.id}/message`, { mode: 'send', content: 'Oi', opId: 'dup-1' });
  const n = router.requests.length;
  const again = await sse(`/api/chats/${c.id}/message`, { mode: 'send', content: 'Oi', opId: 'dup-1' });
  assert.equal(router.requests.length, n);
  assert.ok(again.events.at(-1).done);
  const chat = (await j('GET', `/api/chats/${c.id}`)).body;
  assert.equal(chat.history.length, 2);
});

test('conexão do aparelho cai no meio: a geração continua no PC e a resposta fica salva', async () => {
  router.set(DEFAULTS());
  const c = await mkChat();
  let release;
  router.set({ hold: new Promise((r) => (release = r)) });
  const r = await sse(`/api/chats/${c.id}/message`, { mode: 'send', content: 'Fala comigo', opId: 'drop-1' }, { abortAfterAck: true });
  assert.ok(r.events[0].ack);
  assert.ok(await gen.isBusy(c.id), 'continua gerando mesmo sem o cliente');
  release();
  const chat = await waitFor(async () => {
    const x = (await j('GET', `/api/chats/${c.id}`)).body;
    return x.history.length === 2 && x.history[1].status === 'complete' ? x : null;
  });
  assert.match(chat.history[1].content, /Oi, Alex/);
  router.set(DEFAULTS());
});

test('reconexão: o cliente reanexa à geração em andamento e recebe o texto completo', async () => {
  const c = await mkChat();
  router.set({ chunks: ['Mira: um ', 'dois ', 'três ', 'quatro.'], delayMs: 120 });
  const r1 = await sse(`/api/chats/${c.id}/message`, { mode: 'send', content: 'Conte', opId: 'rc-1' }, { abortAfterAck: true });
  const gid = r1.events[0].ack.generationId;
  await new Promise((r) => setTimeout(r, 200));
  const res = await fetch(`${base}/api/chats/${c.id}/generations/${gid}/stream?from=0`);
  const text = await res.text();
  const events = text.split('\n\n').map((b) => b.split('\n').find((l) => l.startsWith('data:'))).filter(Boolean).map((l) => JSON.parse(l.slice(5)));
  assert.equal(events.filter((e) => e.delta).map((e) => e.delta).join(''), 'Mira: um dois três quatro.');
  assert.ok(events.at(-1).done);
  router.set(DEFAULTS());
});

test('parar guarda o trecho recebido como interrompido', async () => {
  const c = await mkChat();
  router.set({ chunks: Array.from({ length: 80 }, (_, i) => `trecho${i} `), delayMs: 40 });
  const p = sse(`/api/chats/${c.id}/message`, { mode: 'send', content: 'Conte uma história longa', opId: 'stop-1' });
  const g = await waitFor(async () => {
    const x = await gen.get(c.id);
    return x && x.buffer.length > 20 ? x : null;
  });
  const stopped = await j('POST', `/api/chats/${c.id}/generations/${g.id}/stop`);
  assert.equal(stopped.status, 200);
  const r = await p;
  assert.equal(r.events.at(-1).status, 'interrupted');
  const chat = (await j('GET', `/api/chats/${c.id}`)).body;
  assert.equal(chat.history[1].status, 'interrupted');
  assert.match(chat.history[1].content, /^trecho0 trecho1/);
  assert.ok(chat.history[1].content.length < 80 * 9, 'não chegou ao fim');
  router.set(DEFAULTS());
});

test('conversa ocupada: segundo envio é 409; limpeza global e do chat são recusadas indicando a conversa', async () => {
  const c = await mkChat({ name: 'Ocupada' });
  let release;
  router.set({ hold: new Promise((r) => (release = r)) });
  const first = sse(`/api/chats/${c.id}/message`, { mode: 'send', content: 'primeira', opId: 'busy-1' });
  await waitFor(() => gen.isBusy(c.id));
  const second = await j('POST', `/api/chats/${c.id}/message`, { mode: 'send', content: 'outra', opId: 'busy-2' });
  assert.equal(second.status, 409);
  assert.ok(second.body.generationId);
  const clear = await j('DELETE', '/api/history');
  assert.equal(clear.status, 409);
  assert.match(clear.body.error, /Ocupada/);
  assert.equal(clear.body.busy[0].id, c.id);
  assert.equal((await j('DELETE', `/api/chats/${c.id}/history`)).status, 409);
  assert.equal((await j('DELETE', `/api/chats/${c.id}`)).status, 409);
  release();
  await first;
  router.set(DEFAULTS());
  assert.equal((await j('DELETE', `/api/chats/${c.id}/history`)).status, 200, 'depois de terminar, a limpeza passa');
});

test('configurações salvas durante a geração permanecem (API)', async () => {
  const c = await mkChat();
  let release;
  router.set({ hold: new Promise((r) => (release = r)) });
  const run = sse(`/api/chats/${c.id}/message`, { mode: 'send', content: 'Oi', opId: 'cfg-1' });
  await waitFor(() => gen.isBusy(c.id));
  const put = await j('PUT', `/api/chats/${c.id}`, { name: 'Renomeado em voo', effort: 'max', persona: 'Vic', settings: { context: { preferredPairs: 25 } } });
  assert.equal(put.status, 200);
  release();
  await run;
  const chat = (await j('GET', `/api/chats/${c.id}`)).body;
  assert.equal(chat.name, 'Renomeado em voo');
  assert.equal(chat.effort, 'high', 'pedido de max ignorado: esforço travado do preset');
  assert.equal(chat.persona, 'Vic');
  assert.equal(chat.settings.context.preferredPairs, 25);
  assert.equal(chat.history.length, 2);
  router.set(DEFAULTS());
});

test('contexto grande demais: 422 com explicação e NADA é gravado', async () => {
  router.set(DEFAULTS());
  const c = await mkChat({ settings: { context: { hardLimit: 2000 } } });
  const huge = 'palavra '.repeat(700);
  const n = router.requests.length;
  const r = await j('POST', `/api/chats/${c.id}/message`, { mode: 'send', content: huge, opId: 'big-1' });
  assert.equal(r.status, 422);
  assert.equal(r.body.code, 'CONTEXT_TOO_LARGE');
  assert.match(r.body.error, /Nada foi enviado/);
  assert.equal(router.requests.length, n, 'nenhuma chamada ao modelo');
  const chat = (await j('GET', `/api/chats/${c.id}`)).body;
  assert.equal(chat.history.length, 0);
});

test('erro do provedor: mensagem do usuário preservada, erro informado e Refazer funciona', async () => {
  const c = await mkChat();
  router.set({ status: 500 });
  const r = await sse(`/api/chats/${c.id}/message`, { mode: 'send', content: 'Alô?', opId: 'err-1' });
  const done = r.events.at(-1);
  assert.equal(done.status, 'error');
  assert.match(done.error, /Provedor \S+ HTTP 500/);
  let chat = (await j('GET', `/api/chats/${c.id}`)).body;
  assert.deepEqual(chat.history.map((m) => m.role), ['user']);
  router.set(DEFAULTS());
  const retry = await sse(`/api/chats/${c.id}/message`, { mode: 'regenerate', opId: 'err-2' });
  assert.equal(retry.events.at(-1).status, 'complete');
  chat = (await j('GET', `/api/chats/${c.id}`)).body;
  assert.deepEqual(chat.history.map((m) => m.role), ['user', 'assistant']);
});

test('estados distintos: limite atingido, resposta vazia e raciocínio oculto', async () => {
  const c1 = await mkChat();
  router.set({ finish: 'length', chunks: ['Mira: texto cortado pelo limite'] });
  const len = await sse(`/api/chats/${c1.id}/message`, { mode: 'send', content: 'a', opId: 'len-1' });
  assert.equal(len.events.at(-1).status, 'length');
  const c2 = await mkChat();
  router.set({ finish: 'stop', chunks: [] });
  const empty = await sse(`/api/chats/${c2.id}/message`, { mode: 'send', content: 'a', opId: 'empty-1' });
  assert.equal(empty.events.at(-1).status, 'empty');
  assert.match(empty.events.at(-1).error, /vazia/);
  const c3 = await mkChat();
  router.set({ finish: 'stop', reasoning: ['pensando em segredo '], chunks: ['<think>planejo isso</think>', 'Mira: ', 'Oi.'] });
  const th = await sse(`/api/chats/${c3.id}/message`, { mode: 'send', content: 'a', opId: 'think-1' });
  assert.equal(th.events.filter((e) => e.delta).map((e) => e.delta).join(''), 'Mira: Oi.');
  assert.doesNotMatch(JSON.stringify(th.events), /pensando em segredo|planejo isso/);
  const chat = (await j('GET', `/api/chats/${c3.id}`)).body;
  assert.equal(chat.history[1].content, 'Mira: Oi.');
  assert.equal(chat.history[1].usage.reasoning, 300);
  router.set(DEFAULTS());
});

test('presets distintos enviados ao mesmo tempo mantêm seus parâmetros próprios', async () => {
  let release;
  router.set({ hold: new Promise((r) => (release = r)) });
  const a = await mkChat({ routeId: 'cbai/deepseek-v4.1-flash', effort: 'high' });
  const b = await mkChat({ routeId: 'cc/claude-sonnet-5-5', effort: 'medium' });
  const c = await mkChat({ routeId: 'ag/gemini-3.8-flash-high', effort: 'high' });
  const n = router.requests.length;
  const runs = [a, b, c].map((x, i) => sse(`/api/chats/${x.id}/message`, { mode: 'send', content: 'oi', opId: `par-${i}` }));
  await waitFor(() => router.requests.length >= n + 3);
  release();
  await Promise.all(runs);
  const got = Object.fromEntries(router.requests.slice(n).map((r) => [r.body.model, r.body]));
  assert.equal(got['cbai/deepseek-v4.1-flash'].reasoning_effort, 'high');
  assert.equal(got['cc/claude-sonnet-5-5'].reasoning_effort, 'medium');
  assert.equal(got['ag/gemini-3.8-flash-high'].reasoning_effort, 'high');
  assert.equal(got['cc/claude-sonnet-5-5'].max_tokens, 7_297);
  assert.equal(got['cbai/deepseek-v4.1-flash'].max_tokens, 11_393);
  assert.equal(got['cc/claude-sonnet-5-5'].messages[0].role, 'system');
  router.set(DEFAULTS());
});

test('trocar de modelo é manual e vale para as próximas respostas (sem fallback silencioso)', async () => {
  router.set(DEFAULTS());
  const c = await mkChat();
  const r = await j('PUT', `/api/chats/${c.id}`, { routeId: 'cc/claude-sonnet-5-5' });
  assert.equal(r.body.presetId, 'claude-sonnet-5-5');
  assert.equal(r.body.effort, 'medium', 'esforço volta ao padrão do novo preset');
  const bad = await j('PUT', `/api/chats/${c.id}`, { routeId: 'cbai/deepseek-v4.1-flash', presetId: 'gemini-38-flash' });
  assert.equal(bad.status, 400);
  router.set({ status: 500 });
  const n = router.requests.length;
  await sse(`/api/chats/${c.id}/message`, { mode: 'send', content: 'oi', opId: 'nofb-1' });
  assert.equal(router.requests.length, n + 1, 'uma única tentativa, na rota escolhida');
  assert.equal(router.requests.at(-1).body.model, 'cc/claude-sonnet-5-5');
  router.set(DEFAULTS());
});

test('CORS: origem desconhecida é barrada; origem do APK (Capacitor) é aceita', async () => {
  const bad = await fetch(`${base}/api/health`, { headers: { Origin: 'https://site-malicioso.example' } });
  assert.equal(bad.status, 403);
  const ok = await fetch(`${base}/api/health`, { headers: { Origin: 'http://localhost' } });
  assert.equal(ok.status, 200);
  assert.equal(ok.headers.get('access-control-allow-origin'), 'http://localhost');
  const pre = await fetch(`${base}/api/chats`, { method: 'OPTIONS', headers: { Origin: 'http://localhost', 'Access-Control-Request-Method': 'GET' } });
  assert.equal(pre.status, 204);
});

test('pareamento: código de uso único, token guardado como hash, clientes da LAN exigem token', async () => {
  const {code}=(await j('POST','/api/pair/start',{})).body;
  assert.equal((await j('POST','/api/pair',{code:'invalid'})).status,401);
  const {token,deviceId}=(await j('POST','/api/pair',{code,deviceName:'Celular'})).body;
  assert.equal((await j('POST','/api/pair',{code})).status,401);
  const db=new DatabaseSync(path.join(dataDir,'modularchat.db'),{readOnly:true});try{const row=db.prepare('SELECT token_hash FROM devices WHERE id=?').get(deviceId);assert.ok(row);assert.notEqual(row.token_hash,token);}finally{db.close();}
  assert.equal((await j('DELETE',`/api/devices/${deviceId}`)).status,200);
});

test('exportação e importação pela API', async () => {
  router.set(DEFAULTS());
  const c = await mkChat({ name: 'Para exportar' });
  await sse(`/api/chats/${c.id}/message`, { mode: 'send', content: 'Oi', opId: 'exp-1' });
  const data = (await j('GET', `/api/chats/${c.id}/export`)).body;
  assert.equal(data.format, 'modularchat-export');
  const imp = await j('POST', '/api/import', data);
  assert.equal(imp.status, 201);
  assert.equal(imp.body.imported.length, 1);
  const list = (await j('GET', '/api/chats')).body.chats;
  assert.ok(list.filter((x) => x.name === 'Para exportar').length >= 2);
});

test('cada resposta é conferida no pedido ao provedor: esforço travado e tokens reais', async () => {
  router.set(DEFAULTS());
  const c = await mkChat({ routeId: 'cc/claude-sonnet-5-5' });
  await sse(`/api/chats/${c.id}/message`, { mode: 'send', content: 'Oi', opId: 'ver-1' });
  const msg = await waitFor(async () => {
    const x = (await j('GET', `/api/chats/${c.id}`)).body.history[1];
    return x?.usage?.provider ? x : null;
  }, 8000);
  assert.equal(msg.usage.provider.effortExpected, 'medium');
  assert.equal(msg.usage.provider.effortSent, 'medium');
  assert.equal(msg.usage.provider.effortOk, true);
  assert.equal(msg.usage.provider.usage.input, 952, 'contagem real, sem o acréscimo do gateway');
  // a gateway that swapped the effort is reported in the reply
  providerOverride.effort = 'max';
  const c2 = await mkChat({ routeId: 'cc/claude-sonnet-5-5' });
  await sse(`/api/chats/${c2.id}/message`, { mode: 'send', content: 'Oi de novo', opId: 'ver-2' });
  const bad = await waitFor(async () => {
    const x = (await j('GET', `/api/chats/${c2.id}`)).body.history[1];
    return x?.usage?.provider ? x : null;
  }, 8000);
  providerOverride.effort = undefined;
  assert.equal(bad.usage.provider.effortOk, false, JSON.stringify(bad.usage.provider));
  assert.equal(bad.usage.provider.effortSent, 'max');
});


test('limites obrigatórios: usuário 6000, bot 8000 com emoji, persistencia e HTTP', async () => {
  router.set(DEFAULTS());
  const c=await mkChat();
  const n=router.requests.length;
  const tooLong=await j('POST',`/api/chats/${c.id}/message`,{mode:'send',content:'a'.repeat(6001)});
  assert.equal(tooLong.status,422);assert.equal(router.requests.length,n);
  assert.equal((await j('GET',`/api/chats/${c.id}`)).body.history.length,0);
  router.set({...DEFAULTS(),chunks:['<think>interno</think>'+ 'x'.repeat(7997)+'🚀','🚀excesso','mais texto']});
  const result=await sse(`/api/chats/${c.id}/message`,{mode:'send',content:'a'.repeat(6000)});
  const visible=result.events.map(e=>e.delta || '').join('');
  assert.equal(visible.length,7999);assert.ok(visible.endsWith('🚀'));assert.ok(!/[\uD800-\uDBFF]$/.test(visible));
  const chat=(await j('GET',`/api/chats/${c.id}`)).body;
  assert.equal(chat.history.at(-1).content,visible);assert.equal(chat.history.at(-1).status,'length');
  router.set(DEFAULTS());
});

test('ajustes globais pelo app: teto de caracteres por texto gerado, proporção e conteúdo valem na próxima resposta', async () => {
  router.set(DEFAULTS());
  const initial = await j('GET', '/api/settings/global');
  assert.equal(initial.status, 200);
  assert.deepEqual(initial.body.settings, DEFAULT_STYLE);
  const bad = await j('PUT', '/api/settings/global', { botChars: 'lixo', dialoguePct: 9999, content: 42 });
  assert.equal(bad.status, 200);
  assert.deepEqual(bad.body.settings, { ...DEFAULT_STYLE, dialoguePct: 100 }, 'inválidos nunca gravam lixo');
  const put = await j('PUT', '/api/settings/global', { botChars: 600, dialoguePct: 40, content: 'Conteúdo +18: proibido.' });
  assert.deepEqual(put.body.settings, { ...DEFAULT_STYLE, botChars: 600, dialoguePct: 40, content: 'Conteúdo +18: proibido.' });
  assert.deepEqual((await j('GET', '/api/settings/global')).body.settings, put.body.settings, 'persistiu no PC');
  const c = await mkChat();
  router.set({ ...DEFAULTS(), chunks: ['x'.repeat(2000)] });
  const before = router.requests.length;
  const result = await sse(`/api/chats/${c.id}/message`, { mode: 'send', content: 'Oi', opId: 'global-1' });
  const visible = result.events.map((e) => e.delta || '').join('');
  assert.equal(visible.length, 600, 'texto gerado limitado ao teto escolhido');
  const chat = (await j('GET', `/api/chats/${c.id}`)).body;
  assert.equal(chat.history.at(-1).content.length, 600);
  assert.equal(chat.history.at(-1).status, 'length');
  const system = router.requests[before].body.messages.map((m) => m.content).join('\n');
  assert.ok(system.includes('Limite obrigatório: 600 unidades UTF-16'));
  assert.ok(system.includes('aproximadamente 40% de diálogo e 60% de narração'));
  assert.ok(system.includes('CONTEÚDO E EXPLICITUDE') && system.includes('Conteúdo +18: proibido.'));
  // back to the default: long replies are only limited by the 8,000 cap
  await j('PUT', '/api/settings/global', { botChars: 8000, dialoguePct: 65, content: '' });
  router.set(DEFAULTS());
});

test('orçamento acompanha os caracteres antes da API em enviar, continuar e refazer; máximo não força texto longo', async () => {
  router.set({ ...DEFAULTS(), chunks: ['Lia: Achei a carta.'], correctionText: null, correctionStatus: null });
  try {
    await j('PUT', '/api/settings/global', { ...DEFAULT_STYLE, botChars: 600, dialoguePct: 100 });
    const c = await mkChat({ routeId: 'ag/gemini-3.8-flash-low', presetId: 'gemini-38-flash' });
    const corrections = router.correctionRequests.length;
    const send = await sse(`/api/chats/${c.id}/message`, { mode: 'send', content: 'O que encontrou?', opId: 'budget-send' });
    const ack = send.events[0].ack;
    assert.equal(router.requests.at(-1).body.max_tokens, 2288);
    assert.equal(ack.length.apiMaxTokens, 2288);
    assert.equal(ack.controls.sentParams.max_tokens, 2288);
    assert.equal(send.events.at(-1).usage.style.attempted, false);
    assert.equal(send.events.at(-1).usage.length.apiMaxTokens, 2288);
    assert.equal(router.correctionRequests.length, corrections, 'não tenta preencher uma resposta curta e concluída');
    await j('PUT', '/api/settings/global', { botChars: 1200, monologueChars: 150, turnPacing: 'balanced' });
    for (const mode of ['continue', 'regenerate']) {
      const result = await sse(`/api/chats/${c.id}/message`, { mode, opId: `budget-${mode}` });
      const request = router.requests.at(-1).body;
      assert.equal(request.max_tokens, 2528);
      assert.equal(result.events[0].ack.length.monologueChars, 150);
      const all = request.messages.map(m => m.content).join('\n');
      assert.ok(all.includes('Tamanho máximo: até 1.200'));
      assert.ok(all.includes('No máximo 150 unidades UTF-16 de falas seguidas'));
      assert.ok(all.includes('Ritmo equilibrado'));
      assert.ok(all.includes('orçamento visível estimado de 400 tokens'));
    }
    const saved = (await j('GET', `/api/chats/${c.id}`)).body.history.at(-1);
    assert.equal(saved.usage.length.apiMaxTokens, 2528, 'o orçamento fica disponível nos detalhes após recarregar');
  } finally {
    await j('PUT', '/api/settings/global', DEFAULT_STYLE);
    router.set({ ...DEFAULTS(), correctionText: null, correctionStatus: null });
  }
});

test('monólogo longo recebe uma única reescrita; a correção aceita respeita o limite de falas', async () => {
  const original = 'Lia: ' + 'Encontramos uma carta com o endereço. '.repeat(10);
  const corrected = 'Lia: Achei uma carta com o endereço. Podemos conferir.';
  router.set({ ...DEFAULTS(), chunks: [original], correctionText: corrected, correctionStatus: null });
  try {
    await j('PUT', '/api/settings/global', { ...DEFAULT_STYLE, botChars: 600, dialoguePct: 100, monologueChars: 150 });
    const c = await mkChat({ routeId: 'cc/claude-sonnet-5-5', effort: 'low' });
    const n = router.correctionRequests.length;
    const result = await sse(`/api/chats/${c.id}/message`, { mode: 'send', content: 'O que encontrou?', opId: 'short-monologue' });
    const done = result.events.at(-1);
    assert.equal(done.message.content, corrected);
    assert.equal(done.usage.style.before.monologueOk, false);
    assert.equal(done.usage.style.after.monologueOk, true);
    assert.equal(done.usage.style.accepted, true);
    assert.equal(router.correctionRequests.length, n + 1);
    assert.equal(router.correctionRequests.at(-1).body.max_tokens, 2288);
    assert.ok(router.correctionRequests.at(-1).body.messages.at(-1).content.includes('No máximo 150 unidades UTF-16'));
  } finally {
    await j('PUT', '/api/settings/global', DEFAULT_STYLE);
    router.set({ ...DEFAULTS(), correctionText: null, correctionStatus: null });
  }
});

test('rotas de persona, elenco, memória, versões, ramos, duplicação, fonte e lixeira preservam contratos',async()=>{
  router.set(DEFAULTS());
  const p=await j('POST','/api/personas',{name:'Persona HTTP',description:'Descrição inicial'});assert.equal(p.status,201);
  const c=await mkChat({personaId:p.body.id,name:'Contratos ampliados'});
  assert.equal(c.persona,'Persona HTTP');
  assert.equal((await j('PUT',`/api/personas/${p.body.id}`,{name:'Persona HTTP nova',description:'Nova descrição'})).status,200);
  assert.equal((await j('GET',`/api/chats/${c.id}`)).body.persona,'Persona HTTP nova');
  const a=(await j('POST',`/api/chats/${c.id}/cast`,{name:'Ari',description:'Guarda'})).body.cast.find(x=>x.name==='Ari');
  const b=(await j('POST',`/api/chats/${c.id}/cast`,{name:'Aria',description:'Outra grafia'})).body.cast.find(x=>x.name==='Aria');
  assert.equal((await j('POST',`/api/chats/${c.id}/cast/${b.id}/merge`,{intoId:a.id})).status,200);
  assert.ok((await j('GET',`/api/chats/${c.id}/cast`)).body.cast.find(x=>x.id===a.id).aliases.includes('Aria'));
  const m=await j('POST',`/api/chats/${c.id}/memory`,{kind:'FACT',content:'Uma nota manual importante.',pinned:true});assert.equal(m.status,201);
  assert.equal((await j('PUT',`/api/chats/${c.id}/memory/${m.body.id}`,{content:'Uma nota manual corrigida.'})).status,200);
  assert.equal((await j('DELETE',`/api/chats/${c.id}/memory/${m.body.id}`)).status,200);
  const r=await sse(`/api/chats/${c.id}/message`,{mode:'send',content:'Olá',opId:'expanded-send'});const first=r.events.at(-1).message;
  const edited=await j('PUT',`/api/chats/${c.id}/messages/${first.id}`,{content:'Mira: Texto editado.'});assert.equal(edited.status,200);
  const versions=(await j('GET',`/api/chats/${c.id}/messages/${first.id}/versions`)).body.versions;assert.ok(versions.length>=2);
  assert.equal((await j('POST',`/api/chats/${c.id}/messages/${first.id}/versions/${versions[0].id}/select`,{})).status,200);
  const source=(await j('GET',`/api/chats/${c.id}/interactions/${r.events[0].ack.interactionId}/source`)).body;assert.equal(source.user,'Olá');assert.ok(source.assistant);
  const chat=(await j('GET',`/api/chats/${c.id}`)).body;
  await sse(`/api/chats/${c.id}/message`,{mode:'edit',messageId:chat.history[0].interactionId,content:'Outro ramo',opId:'expanded-edit'});
  const branches=(await j('GET',`/api/chats/${c.id}/branches`)).body.branches;assert.ok(branches.length>=1);
  assert.equal((await j('POST',`/api/chats/${c.id}/branches/activate`,{interactionId:chat.history[0].interactionId})).status,200);
  const copy=await j('POST',`/api/chats/${c.id}/duplicate`,{});assert.equal(copy.status,201);assert.notEqual(copy.body.id,c.id);
  assert.equal((await j('DELETE',`/api/chats/${copy.body.id}`)).status,200);
  assert.equal((await j('POST','/api/trash/restore',{type:'conversation',id:copy.body.id})).status,200);
  assert.equal((await j('GET',`/api/chats/${copy.body.id}`)).status,200);
  assert.equal((await j('POST',`/api/chats/${c.id}/jobs/retry`,{})).status,200);
  assert.equal((await j('POST',`/api/chats/${c.id}/cast/reanalyze`,{})).status,200);
  assert.equal((await j('POST','/api/backup',{})).status,200);
});

test('HTTP aceita importação legada acima de 2 MiB, recusa corpo comum grande e faz rollback de referências inválidas',async()=>{
  router.set(DEFAULTS());const c=await mkChat();
  const r=await sse(`/api/chats/${c.id}/message`,{mode:'send',content:'Fixture legado',opId:'import-large'});
  const payload=(await j('GET',`/api/chats/${c.id}/export`)).body;
  payload.conversations[0].versions[0].content='x'.repeat(2*1024*1024+100)+' CAUDA_IMPORTADA';
  assert.ok(JSON.stringify(payload).length>2*1024*1024);
  const imported=await j('POST','/api/import',payload);assert.equal(imported.status,201);assert.equal(imported.body.imported.length,1);
  const saved=(await j('GET',`/api/chats/${imported.body.imported[0]}`)).body;assert.ok(saved.history.at(-1).content.endsWith('CAUDA_IMPORTADA'));
  const large=await j('POST','/api/chats',{name:'Corpo grande',prompt:'x'.repeat(2*1024*1024+10)});assert.equal(large.status,413);
  const n=(await j('GET','/api/chats')).body.chats.length;payload.conversations[0].parts[0].selected_version_id='bad-reference';
  const invalid=await j('POST','/api/import',payload);assert.equal(invalid.status,201);assert.equal(invalid.body.imported.length,0);assert.equal(invalid.body.errors.length,1);
  assert.equal((await j('GET','/api/chats')).body.chats.length,n);assert.ok(r.events.at(-1).done);
});

test('reconexão Unicode reenvia apenas o sufixo UTF-16 sem perder emojis',async()=>{
  router.set({...DEFAULTS(),chunks:['Mira: 🚀 ', '✨ fim.'],delayMs:60});const c=await mkChat();
  const r=await sse(`/api/chats/${c.id}/message`,{mode:'send',content:'Unicode',opId:'unicode-reconnect'});
  const id=r.events[0].ack.generationId,offset='Mira: 🚀 '.length;
  const response=await fetch(`${base}/api/chats/${c.id}/generations/${id}/stream?from=${offset}`);assert.equal(response.status,200);
  const ev=(await response.text()).split('\n\n').map(b=>b.split('\n').find(l=>l.startsWith('data:'))).filter(Boolean).map(l=>JSON.parse(l.slice(5)));
  assert.equal(ev.map(e=>e.delta||'').join(''),'✨ fim.');assert.equal(ev.at(-1).done,true);router.set(DEFAULTS());
});
