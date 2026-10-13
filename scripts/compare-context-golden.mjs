// Compares the OUTPUT of the Rust store of two mc-contract executables (baseline x candidate) for the same calls.
// It is a safety net for optimizations: the result of getContextData/memoryOverview must not change.
// Usage: node scripts/compare-context-golden.mjs --baseline <mc-contract.exe> --candidate <mc-contract.exe> [--fixtures <dir>]
// Random IDs and timestamps are normalized (IDs become ordinals by order of appearance).
// INTENTIONAL changes to getContextData (token/IPC savings) are applied to the BASELINE output before comparing
// (see intended()); with --raw nothing is projected and any difference shows up. The assembled prompt does not change.
import fs from 'node:fs';
import path from 'node:path';
import os from 'node:os';
import assert from 'node:assert/strict';
import { RustStore } from '../tests/rust-store.js';

const args = Object.fromEntries(process.argv.slice(2).reduce((acc, a, i, all) => (a.startsWith('--') ? [...acc, [a.slice(2), all[i + 1]]] : acc), []));
if (!args.baseline || !args.candidate) {
  console.error('Uso: --baseline <exe> --candidate <exe> [--fixtures <dir>]');
  process.exit(2);
}
const UUID = /[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}/gi;
const TIME = /\d{4}-\d\d-\d\dT\d\d:\d\d:\d\d(?:\.\d+)?Z/g;
const HASH = /\b[0-9a-f]{64}\b/g;

// Projects the old output onto what the candidate must return: pairs without parts and with sources only on the last 3 (pending ones
// without sources); only the notes the timeline can read (before the window of the smallest step and not covered by an episode
// before the window of the preferred step). Same rule as notes_for_prompt (server-rust/src/store/context.rs).
function intended(r) {
  if (process.argv.includes('--raw') || !r || !Array.isArray(r.pairs)) return r;
  const out = JSON.parse(JSON.stringify(r));
  const slim = (p, keepSources) => {
    const { parts, sources, ...rest } = p;
    return keepSources ? { ...rest, sources } : rest;
  };
  out.pairs = out.pairs.map((p, i) => slim(p, i >= out.pairs.length - 3));
  out.unprocessed = out.unprocessed.map((p) => slim(p, false));
  const cfg = out.conv?.settings?.context ?? {};
  const maximum = Math.min(50, Math.max(20, Number(cfg.maxPairs) || 50));
  const minimum = Math.min(maximum, Math.max(20, Number(cfg.minPairs) || 20));
  const preferred = Math.min(maximum, Math.max(minimum, Number(cfg.preferredPairs) || 30));
  const windowOf = (take) => out.pairs[out.pairs.length - Math.min(take, out.pairs.length)]?.seq;
  const [widest, narrowest] = [windowOf(minimum), windowOf(preferred)];
  if (widest !== undefined && narrowest !== undefined) {
    const covered = new Set(out.episodes.filter((e) => e.toSeq < narrowest).flatMap((e) => e.interactionIds));
    out.notes = out.notes.filter((n) => n.seq < widest && !covered.has(n.interactionId));
  }
  return out;
}

function normalize(value) {
  const seen = new Map();
  const sortKeys = (v) => (Array.isArray(v) ? v.map(sortKeys) : v && typeof v === 'object' ? Object.fromEntries(Object.keys(v).sort().map((k) => [k, sortKeys(v[k])])) : v);
  const text = JSON.stringify(sortKeys(value))
    .replace(UUID, (m) => {
      const k = m.toLowerCase();
      if (!seen.has(k)) seen.set(k, `ID${seen.size}`);
      return seen.get(k);
    })
    .replace(HASH, 'HASH')
    .replace(TIME, 'TIME');
  return text;
}

async function open(exe) {
  process.env.MC_CONTRACT_EXE = exe;
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'mc-golden-'));
  const store = new RustStore(path.join(dir, 'g.db'));
  await store.ready();
  return { store, dir };
}

async function close(h) {
  await h.store.close();
  fs.rmSync(h.dir, { recursive: true, force: true });
}

// Sequence of calls applied to both executables; returns [label, result][].
async function scenario(store, fixture) {
  const out = [];
  const imported = await store.call('importConversation', JSON.parse(JSON.stringify(fixture)));
  const convId = imported.id;
  const exported = await store.call('exportConversation', convId);
  const inter = [...exported.interactions].sort((a, b) => a.seq - b.seq);
  const mid = inter[Math.floor(inter.length / 2)];
  const last = inter[inter.length - 1];
  const calls = [
    ['send50', { convId, pendingUserText: 'Onde está a chave dourada?', maxPairs: 50 }],
    ['send20', { convId, pendingUserText: 'x', maxPairs: 20 }],
    ['send30', { convId, pendingUserText: 'Outra pergunta', maxPairs: 30 }],
    ['edit-mid', { convId, interactionId: mid.id, overrideUserText: 'Texto editado', maxPairs: 50 }],
    ['regen-last', { convId, interactionId: last.id, dropLastPart: true, maxPairs: 50 }],
    ['current-last', { convId, maxPairs: 50 }],
  ];
  for (const [label, input] of calls) {
    const t0 = process.hrtime.bigint();
    const r = await store.call('getContextData', input);
    delete r.globalSettings; // key added after the baseline (global settings)
    out.push([label, r, Number(process.hrtime.bigint() - t0) / 1e6]);
  }
  out.push(['memoryOverview', await store.call('memoryOverview', convId), 0]);
  return out;
}

const fixtures = [];
if (args.fixtures) {
  for (const f of fs.readdirSync(args.fixtures).filter((n) => /^context-\d+\.json$/.test(n)).sort()) {
    fixtures.push([f, JSON.parse(fs.readFileSync(path.join(args.fixtures, f), 'utf8'))]);
  }
}

// Synthetic conversation with versions (Redo), continuation, edit and branch, with no external fixture.
async function synthetic(store) {
  const c = await store.call('createConversation', { name: 'Golden', persona: 'Alex', routeId: 'cbai/deepseek-v4.1-flash', presetId: 'deepseek-v41-flash', effort: 'high', prompt: 'Há 3 meses chegaram. Lia e Laura estudam magia.' });
  const begin = async (extra) => store.call('beginTurn', { convId: c.id, routeId: c.routeId, presetId: c.presetId, effort: 'high', ...extra });
  const fin = (b, text, status = 'complete') => store.call('finishGeneration', { generationId: b.generationId, text, status, usage: {} });
  for (let i = 1; i <= 24; i++) {
    const b = await begin({ mode: 'send', content: i === 8 ? 'Um dia depois, voltamos à biblioteca.' : i === 15 ? 'Uma semana se passa.' : `Examino a carta ${i}. 😀`, opId: `g-${i}` });
    await fin(b, `Lia: No turno ${i}, a chave dourada está no jardim.\n\n*Ela abre a carta azul ${i}.*`);
  }
  const rg = await begin({ mode: 'regenerate' });
  await fin(rg, 'Lia: Versão refeita do último turno.');
  const ct = await begin({ mode: 'continue' });
  await fin(ct, '*Continuação da resposta.*', 'length');
  const exported = await store.call('exportConversation', c.id);
  const firstTurn = [...exported.interactions].sort((a, b) => a.seq - b.seq)[3];
  const ed = await begin({ mode: 'edit', targetId: firstTurn.id, content: 'Texto editado do turno 4 (cria ramo).' });
  await fin(ed, 'Lia: Resposta do ramo novo.');
  return c.id;
}

let failures = 0;
const report = [];
async function compareOne(name, run) {
  const [a, b] = [await open(args.baseline), await open(args.candidate)];
  try {
    const [ra0, rb] = [await run(a.store), await run(b.store)];
    const ra = Array.isArray(ra0) ? ra0.map(([label, value, ms]) => [label, intended(value), ms]) : ra0;
    const [xa, xb] = [Array.isArray(ra) ? ra : [], Array.isArray(rb) ? rb : []];
    for (let i = 0; i < Math.max(xa.length, xb.length); i++) {
      const same = normalize(xa[i]?.[1]) === normalize(xb[i]?.[1]);
      report.push({ fixture: name, call: xa[i]?.[0], same, baselineMs: +xa[i]?.[2]?.toFixed(2), candidateMs: +xb[i]?.[2]?.toFixed(2) });
      if (!same) {
        failures++;
        const na = normalize(xa[i]?.[1]);
        const nb = normalize(xb[i]?.[1]);
        let at = 0;
        while (at < na.length && na[at] === nb[at]) at++;
        console.error(`DIVERGE ${name}/${xa[i]?.[0]} @${at}\n  base: ${na.slice(Math.max(0, at - 80), at + 160)}\n  cand: ${nb.slice(Math.max(0, at - 80), at + 160)}`);
      }
    }
  } finally {
    await close(a);
    await close(b);
  }
}

for (const [name, fixture] of fixtures) await compareOne(name, (store) => scenario(store, fixture));
await compareOne('synthetic', async (store) => {
  const id = await synthetic(store);
  const out = [];
  for (const [label, input] of [
    ['send', { convId: id, pendingUserText: 'E agora?', maxPairs: 50 }],
    ['send-small', { convId: id, pendingUserText: 'E agora?', maxPairs: 5 }],
    ['current', { convId: id, maxPairs: 50 }],
  ]) {
    const r = await store.call('getContextData', input);
    delete r.globalSettings;
    out.push([label, r, 0]);
  }
  out.push(['memoryOverview', await store.call('memoryOverview', id), 0]);
  out.push(['getConversation', await store.call('getConversation', id), 0]);
  return out;
});

console.table(report.map((r) => ({ ...r, same: r.same ? 'IGUAL' : 'DIFERENTE' })));
assert.equal(failures, 0, `${failures} saída(s) divergente(s) entre baseline e candidato`);
console.log(`OK: ${report.length} comparações idênticas`);
