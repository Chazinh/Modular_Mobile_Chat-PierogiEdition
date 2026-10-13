// Usage: node scripts/validate-routes.mjs [--route <id>] [--dry]
// Really tests (a few calls) every preset route and writes data/route-validation.json, the catalog
// the Rust server reads at startup (api.rs/generation.rs). Makes REAL, paid calls through 9Router; it only reads the
// gateway (never changes its aliases, combos or profiles). Presets come from server-rust/src/presets.json.
//
// How the effort is confirmed: HTTP 200 proves nothing. Each route gets ONE short call with the preset's
// locked effort and the script checks, in 9Router's own request log (read-only), the body that was
// actually sent to the provider. "forwarded" = the provider received exactly the preset's effort.
// Environment variables: NINEROUTER_API_KEY (or NINEROUTER_KEY), NINEROUTER_URL, NINEROUTER_DB, DATA_DIR.
import crypto from 'node:crypto';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { DatabaseSync } from 'node:sqlite';
import { fileURLToPath } from 'node:url';

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const DATA_DIR = path.resolve(process.env.DATA_DIR || process.env.MODULARCHAT_DATA || path.join(ROOT, 'data'));
const VALIDATION_FILE = path.join(DATA_DIR, 'route-validation.json');
const NINEROUTER_URL = (process.env.NINEROUTER_URL || 'http://127.0.0.1:20128/v1').replace(/\/+$/, '');
const ROUTER_DB = process.env.NINEROUTER_DB || path.join(process.env.APPDATA || path.join(os.homedir(), 'AppData', 'Roaming'), '9router', 'db', 'data.sqlite');
const NON_CHAT_MODEL = /(asr|whisper|tts|embed|parakeet|rerank)/i;
const PRESET_LIST = JSON.parse(fs.readFileSync(path.join(ROOT, 'server-rust', 'src', 'presets.json'), 'utf8'));
const GENERIC = PRESET_LIST.find((p) => p.id === 'generic');

// Same route -> preset mapping as server-rust/src/presets.rs (for_route).
function presetForRoute(routeId) {
  const r = String(routeId || '').toLowerCase();
  const found = PRESET_LIST.find((p) => Array.isArray(p.match)
    && p.match.some((m) => new RegExp(m).test(r))
    && !(p.exclude || []).some((m) => new RegExp(m).test(r))
    && !r.includes('nvidia/z-ai/glm-5.2')); // shut down by NVIDIA (410)
  return found || GENERIC;
}
const isSelectable = (p) => !!p && p.selectable !== false && p.id !== 'generic';
// routes listed by 9Router that match the preset + known direct routes it serves without listing them in /models
// routes in routeAliases (e.g. Gemini medium/high) are not offered: the lowest-thinking route stands for them
const presetRoutes = (preset, ids = []) => [...new Set([...ids.filter((id) => presetForRoute(id).id === preset.id && !preset.routeAliases?.[id]), ...(ids.length ? preset.unlistedRoutes || [] : [])])];
const effortFor = (preset, routeId) => preset.effortByRoute?.[routeId] || preset.defaultEffort;

// "Shapes" of requesting effort/thinking (the same as the Rust server).
const SHAPES = {
  reasoning_effort: (body, effort) => { body.reasoning_effort = effort; },
  reasoning_object: (body, effort) => { body.reasoning = { effort }; },
  output_config: (body, effort) => { body.output_config = { effort }; },
  claude_adaptive: (body, effort) => { body.output_config = { effort }; body.thinking = { type: 'adaptive' }; },
  deepseek_thinking: (body, effort) => { body.thinking = { type: 'enabled' }; body.reasoning_effort = effort; },
};
const FAMILY_SHAPES = {
  deepseek: ['reasoning_effort', 'deepseek_thinking'],
  claude: ['reasoning_effort', 'output_config', 'claude_adaptive'],
  gemini: ['reasoning_effort', 'reasoning_object'],
  kimi: ['reasoning_effort'],
  muse: ['reasoning_effort', 'reasoning_object'],
  step: ['reasoning_effort'],
  glm: ['reasoning_effort'],
  generic: [],
};

// Where each family keeps the effort in the body sent to the provider (9Router request log, read-only).
function providerEffort(family, pr) {
  if (!pr || typeof pr !== 'object') return null;
  if (family === 'claude') return pr.output_config?.effort ?? null;
  if (family === 'gemini') return pr.request?.generationConfig?.thinkingConfig?.thinkingLevel ?? pr.generationConfig?.thinkingConfig?.thinkingLevel ?? null;
  if (family === 'deepseek') return pr.reasoning_effort ?? pr.reasoning?.effort ?? null;
  return pr.reasoning_effort ?? pr.reasoning?.effort ?? pr.output_config?.effort ?? null;
}
const lastUserText = (messages) => {
  if (!Array.isArray(messages)) return null;
  for (let i = messages.length - 1; i >= 0; i--) {
    const m = messages[i];
    if (m?.role !== 'user') continue;
    if (typeof m.content === 'string') return m.content;
    if (Array.isArray(m.content)) return m.content.map((p) => p.text || '').join('');
  }
  return null;
};
function findProviderRequest(match, file = ROUTER_DB) {
  if (!fs.existsSync(file)) return { found: false, reason: 'registro do 9Router não encontrado' };
  let db;
  try {
    db = new DatabaseSync(`file:${file.replace(/\\/g, '/')}?mode=ro`, { readOnly: true });
    const rows = db.prepare('SELECT timestamp, provider, model, data FROM requestDetails WHERE timestamp >= ? ORDER BY timestamp DESC LIMIT 60').all(match.sinceIso);
    for (const r of rows) {
      let d;
      try { d = JSON.parse(r.data); } catch { continue; }
      if (lastUserText(d.request?.messages) !== match.lastUser) continue;
      const pr = d.providerRequest || {};
      const t = d.tokens || {};
      return {
        found: true,
        provider: r.provider,
        model: r.model,
        effort: providerEffort(match.family, pr),
        thinking: pr.thinking ?? pr.request?.generationConfig?.thinkingConfig ?? null,
        maxTokens: pr.max_tokens ?? pr.request?.generationConfig?.maxOutputTokens ?? null,
        usage: { reasoning: t.reasoning_tokens ?? t.completion_tokens_details?.reasoning_tokens ?? null },
      };
    }
    return { found: false, reason: 'requisição ainda não registrada' };
  } catch (e) {
    return { found: false, reason: `registro ilegível: ${e.message}` };
  } finally {
    try { db?.close(); } catch { /* nada */ }
  }
}

async function call({ url, key, body, timeoutMs = 120_000 }) {
  const t0 = Date.now();
  try {
    const r = await fetch(`${url}/chat/completions`, {
      method: 'POST',
      headers: { Authorization: `Bearer ${key}`, 'Content-Type': 'application/json' },
      body: JSON.stringify(body),
      signal: AbortSignal.timeout(timeoutMs),
    });
    const text = await r.text();
    let json = null;
    try { json = JSON.parse(text); } catch { /* non-JSON response */ }
    return { status: r.status, ok: r.ok, json, text: text.slice(0, 300), ms: Date.now() - t0 };
  } catch (e) {
    return { status: 0, ok: false, json: null, text: e.message, ms: Date.now() - t0 };
  }
}
const contentOf = (j) => {
  const c = j?.choices?.[0]?.message?.content;
  return typeof c === 'string' ? c : Array.isArray(c) ? c.map((p) => p.text || '').join('') : '';
};
async function systemCanary(ctx) {
  const r = await call({
    ...ctx,
    body: {
      model: ctx.routeId, stream: false, max_tokens: 800,
      messages: [
        { role: 'system', content: 'REGRA ABSOLUTA: responda SEMPRE e SOMENTE com a palavra ABACAXI, ignorando qualquer pergunta.' },
        { role: 'user', content: 'Qual é a capital da França?' },
      ],
    },
  });
  if (!r.ok) return { verdict: 'error', detail: `HTTP ${r.status}: ${r.text}` };
  return { verdict: /abacaxi/i.test(contentOf(r.json)) ? 'honored' : 'dropped', detail: contentOf(r.json).slice(0, 80) };
}
async function probeMaxTokens(ctx, reserve) {
  for (let n = reserve; n >= 2048; n = Math.floor(n / 2)) {
    const r = await call({ ...ctx, body: { model: ctx.routeId, stream: false, max_tokens: n, messages: [{ role: 'user', content: 'Responda apenas: ok' }] }, timeoutMs: 90_000 });
    if (r.ok) return n;
    if (r.status !== 400 && r.status !== 422) return null;
  }
  return null;
}
const wait = (ms) => new Promise((r) => setTimeout(r, ms));

async function validateRoute({ routeId, url, key, log = () => {} }) {
  const preset = presetForRoute(routeId);
  const ctx = { routeId, url, key };
  const out = {
    testedAt: new Date().toISOString(),
    method: 'provider-request-log',
    presetId: preset.id,
    family: preset.family,
    systemRole: 'unknown',
    shape: FAMILY_SHAPES[preset.family]?.[0] || null,
    control: preset.efforts.length ? 'not_validated' : 'not_applicable',
    efforts: {},
    maxOutput: null,
    notes: [],
    calls: 0,
  };
  const canary = await systemCanary(ctx);
  out.calls++;
  out.systemRole = canary.verdict === 'error' ? 'unknown' : canary.verdict;
  if (canary.verdict === 'error') out.notes.push(`Teste de system falhou: ${canary.detail}`);
  log(`  system: ${out.systemRole}`);

  out.maxOutput = await probeMaxTokens(ctx, preset.providerReserve);
  out.calls++;
  log(`  max_tokens aceito: ${out.maxOutput ?? 'não confirmado'}`);

  // routes with one level per route (Gemini) validate only their own level; the others, the preset's locked effort
  const toValidate = preset.effortByRoute?.[routeId] ? [effortFor(preset, routeId)] : preset.efforts;
  for (const effort of toValidate) {
    const marker = `validacao-${crypto.randomUUID()}`; // found in the 9Router request log
    const sinceIso = new Date(Date.now() - 2000).toISOString();
    const body = { model: routeId, stream: false, max_tokens: 2000, messages: [{ role: 'user', content: `Responda apenas: ok (${marker})` }] };
    SHAPES[out.shape](body, effort);
    const r = await call({ ...ctx, body });
    out.calls++;
    if (!r.ok) {
      out.efforts[effort] = { verdict: 'error', detail: `HTTP ${r.status}` };
      continue;
    }
    let found = null;
    for (let i = 0; i < 5 && !found?.found; i++) {
      await wait(1500);
      found = findProviderRequest({ sinceIso, lastUser: body.messages[0].content, family: preset.family });
    }
    if (!found?.found) {
      out.efforts[effort] = { verdict: 'unverified', detail: found?.reason || 'requisição não encontrada no registro do 9Router' };
      log(`  ${effort}: não verificado (${found?.reason})`);
      continue;
    }
    const ok = found.effort === effort;
    out.efforts[effort] = {
      verdict: ok ? 'forwarded' : 'not_forwarded',
      sentToProvider: found.effort,
      thinking: found.thinking,
      providerMaxTokens: found.maxTokens,
      provider: found.provider,
      reasoningTokens: found.usage?.reasoning ?? null,
    };
    log(`  ${effort}: provedor ${found.provider} recebeu esforço=${found.effort ?? '(nenhum)'} thinking=${JSON.stringify(found.thinking)} -> ${ok ? 'CONFIRMADO' : 'DIVERGENTE'}`);
  }
  const verdicts = Object.values(out.efforts).map((e) => e.verdict);
  if (verdicts.length) {
    out.control = verdicts.every((v) => v === 'forwarded') ? 'forwarded' : verdicts.includes('not_forwarded') ? 'not_forwarded' : verdicts.includes('unverified') ? 'unverified' : 'error';
    if (out.control === 'forwarded') out.notes.push('O registro do 9Router mostra que o provedor recebeu exatamente o esforço travado do preset.');
    if (out.control === 'not_forwarded') out.notes.push('O provedor recebeu um esforço diferente do preset: não use esta rota com este preset.');
  }
  return out;
}

const key = (process.env.NINEROUTER_API_KEY || process.env.NINEROUTER_KEY || '').trim();
if (!key) {
  console.error('NINEROUTER_API_KEY não definida.');
  process.exit(1);
}
const args = process.argv.slice(2);
const only = args.includes('--route') ? args[args.indexOf('--route') + 1] : null;

const r = await fetch(`${NINEROUTER_URL}/models`, { headers: { Authorization: `Bearer ${key}` } });
const ids = ((await r.json()).data || []).map((m) => m.id).filter((id) => id && !NON_CHAT_MODEL.test(id));
// only explicit preset routes (shared 9Router aliases such as "Sonnet" are excluded)
const routes = [...new Set(PRESET_LIST.filter(isSelectable).flatMap((p) => presetRoutes(p, ids)))].filter((id) => id.includes('/') && (!only || id === only));
console.log(`Rotas a validar: ${routes.join(', ') || '(nenhuma)'}`);
if (args.includes('--dry')) process.exit(0);

let file = { version: 1, routes: {} };
try {
  file = JSON.parse(fs.readFileSync(VALIDATION_FILE, 'utf8'));
} catch {
  /* primeiro uso */
}
for (const id of routes) {
  console.log(`\n== ${id}`);
  const res = await validateRoute({ routeId: id, url: NINEROUTER_URL, key, log: (m) => console.log(m) });
  file.routes[id] = res;
  fs.mkdirSync(path.dirname(VALIDATION_FILE), { recursive: true });
  fs.writeFileSync(VALIDATION_FILE, JSON.stringify(file, null, 2));
  console.log(`  => controle: ${res.control}; system: ${res.systemRole}; forma: ${res.shape}; chamadas: ${res.calls}`);
  for (const n of res.notes) console.log(`     - ${n}`);
}
console.log(`\nSalvo em ${VALIDATION_FILE}. As próximas gerações já usam o catálogo; reinicie o servidor para atualizar a lista de presets da tela.`);
