// CDP trace of the WebView on the phone while the side menu is pulled open and closed (needs the debuggable "perf" copy,
// see scripts/build-perf-android.ps1) and a chat open in the foreground.
// Prints where the time goes: total duration and count per event name, split by thread.
// Usage: node scripts/trace-device-ui.mjs [cycles=8] [out.json]   (MC_PERF_MODE=drawer|bar, as in measure-device-perf.mjs)
//   env: ADB, MC_PERF_PACKAGE, MC_TRACE_PORT (default 9333), MC_TRACE_TOP (rows per thread, default 14)
import { execFileSync } from 'node:child_process';
import fs from 'node:fs';
import path from 'node:path';
import os from 'node:os';

const env = process.env;
const adb = env.ADB || path.join(os.homedir(), 'RoleplayChat-local', 'android-sdk', 'platform-tools', 'adb.exe');
const pkg = env.MC_PERF_PACKAGE || 'io.github.modularchat.pierogiedition.perf';
const port = Number(env.MC_TRACE_PORT || 9333);
const top = Number(env.MC_TRACE_TOP || 14);
const cycles = Number(process.argv[2] || 8);
const outFile = process.argv[3];
const run = (...args) => execFileSync(adb, args, { encoding: 'utf8', windowsHide: true, maxBuffer: 16 * 1024 * 1024 });
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

const pid = run('shell', `pidof ${pkg}`).trim().split(/\s+/)[0];
if (!pid) throw new Error(`${pkg} is not running`);
const socket = run('shell', 'cat /proc/net/unix').match(/@?(webview_devtools_remote_\d+)/g)?.map((s) => s.replace('@', ''));
const mine = socket?.find((s) => s.endsWith(`_${pid}`));
if (!mine) throw new Error('WebView devtools socket not found: install the debuggable perf copy');
run('forward', `tcp:${port}`, `localabstract:${mine}`);
const targets = await (await fetch(`http://127.0.0.1:${port}/json`)).json();
const page = targets.find((t) => t.type === 'page');
if (!page) throw new Error('no page target');

const ws = new WebSocket(page.webSocketDebuggerUrl);
await new Promise((res, rej) => { ws.onopen = res; ws.onerror = rej; });
let nextId = 1;
const waiting = new Map();
const events = [];
let complete;
const done = new Promise((r) => { complete = r; });
ws.onmessage = (m) => {
  const msg = JSON.parse(m.data);
  if (msg.id && waiting.has(msg.id)) { waiting.get(msg.id)(msg); waiting.delete(msg.id); return; }
  if (msg.method === 'Tracing.dataCollected') events.push(...msg.params.value);
  if (msg.method === 'Tracing.tracingComplete') complete();
};
const send = (method, params = {}) => new Promise((res) => { const id = nextId++; waiting.set(id, res); ws.send(JSON.stringify({ id, method, params })); });

const categories = ['devtools.timeline', 'disabled-by-default-devtools.timeline', 'cc', 'gpu', 'viz', 'blink', 'toplevel', 'disabled-by-default-devtools.timeline.frame'].join(',');
await send('Tracing.start', { categories, transferMode: 'ReportEvents', options: 'sampling-frequency=10000' });

const size = run('shell', 'wm size').match(/(\d+)x(\d+)/);
const [w, h] = [Number(size[1]), Number(size[2])];
const mid = Math.round(h * 0.5);
const bar = env.MC_PERF_MODE === 'bar';
const density = Number((run('shell', 'wm density').match(/(\d+)\s*$/m) || [0, 450])[1]) / 160;
for (let i = 0; i < cycles; i++) {
  if (bar) {
    const dotY = Math.round(h * 0.06);
    run('shell', `input swipe ${Math.round(w / 2)} ${dotY} ${Math.round(w / 2)} ${dotY + Math.round(150 * density)} 280`);
    await sleep(600);
    run('shell', `input tap ${Math.round(w / 2)} ${Math.round(h * 0.6)}`);
  } else {
    run('shell', `input swipe ${w - 8} ${mid} ${Math.round(w * 0.2)} ${mid} 280`);
    await sleep(600);
    run('shell', 'input keyevent KEYCODE_BACK');
  }
  await sleep(850);
}
await send('Tracing.end');
await done;
ws.close();
run('forward', '--remove', `tcp:${port}`);

const threads = new Map();
for (const e of events) if (e.ph === 'M' && e.name === 'thread_name') threads.set(`${e.pid}:${e.tid}`, e.args.name);
const byThread = new Map();
for (const e of events) {
  if (e.ph !== 'X' || !e.dur) continue;
  const t = threads.get(`${e.pid}:${e.tid}`) || `tid ${e.tid}`;
  const rows = byThread.get(t) || new Map();
  const row = rows.get(e.name) || { ms: 0, n: 0, max: 0 };
  row.ms += e.dur / 1000; row.n++; row.max = Math.max(row.max, e.dur / 1000);
  rows.set(e.name, row);
  byThread.set(t, rows);
}
// How many elements each style recalculation touched (UpdateLayoutTree reports it): the cost is "elements x styles".
const recalcs = events.filter((e) => e.ph === 'X' && e.name === 'UpdateLayoutTree' && e.args?.elementCount != null);
if (recalcs.length) {
  const counts = recalcs.map((e) => e.args.elementCount).sort((a, b) => a - b);
  const sum = counts.reduce((a, b) => a + b, 0);
  const ms = recalcs.reduce((a, e) => a + e.dur / 1000, 0);
  console.log(`
UpdateLayoutTree: ${recalcs.length} runs, ${Math.round(ms)} ms (${(ms / recalcs.length).toFixed(2)} ms each), elements per run: avg ${Math.round(sum / counts.length)}, median ${counts[counts.length >> 1]}, p90 ${counts[Math.floor(counts.length * 0.9)]}, max ${counts.at(-1)}; ${(ms * 1000 / Math.max(1, sum)).toFixed(1)} us per element`);
}
// The slowest main-thread frames and what they were made of (children of BeginMainFrame on the same thread).
if (env.MC_TRACE_SLOW) {
  const main = events.filter((e) => e.ph === 'X' && e.name === 'ProxyMain::BeginMainFrame').sort((a, b) => b.dur - a.dur).slice(0, Number(env.MC_TRACE_SLOW));
  for (const f of main) {
    const kids = events.filter((e) => e.ph === 'X' && e.pid === f.pid && e.tid === f.tid && e.ts >= f.ts && e.ts + e.dur <= f.ts + f.dur && e !== f && e.dur > 400);
    const agg = new Map();
    for (const k of kids) agg.set(k.name, (agg.get(k.name) || 0) + k.dur / 1000);
    const top = [...agg].sort((a, b) => b[1] - a[1]).slice(0, 7).map(([n, ms]) => `${n} ${ms.toFixed(1)}`).join(' | ');
    console.log(`slow frame ${(f.dur / 1000).toFixed(1)} ms: ${top}`);
  }
}
const summary = {};
for (const [t, rows] of byThread) {
  const list = [...rows].sort((a, b) => b[1].ms - a[1].ms).slice(0, top).map(([name, r]) => ({ name, totalMs: Math.round(r.ms * 10) / 10, count: r.n, maxMs: Math.round(r.max * 10) / 10 }));
  if (list.length && list[0].totalMs >= 5) summary[t] = list;
}
for (const [t, list] of Object.entries(summary)) {
  console.log(`\n== ${t}`);
  for (const r of list) console.log(`${String(r.totalMs).padStart(9)} ms  x${String(r.count).padEnd(6)} max ${String(r.maxMs).padStart(6)}  ${r.name}`);
}
if (outFile) fs.writeFileSync(outFile, JSON.stringify({ cycles, summary }, null, 2));
