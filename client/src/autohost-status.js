// Isolated status polling: reads never activate the host, and teardown invalidates late replies.
export function downloadProgress(model = {}) {
  const download = model.download || {};
  const downloaded = Math.max(0, Number(download.downloaded ?? download.downloadedBytes) || 0);
  const total = Math.max(0, Number(download.total ?? download.totalBytes) || 0);
  return { phase: download.phase || download.state || 'pending', downloaded, total,
    percent: total ? Math.min(100, Math.floor(downloaded * 100 / total)) : null, error: download.error || '' };
}
export function statusPollDelay(status, transitioning = false) {
  return transitioning || ['chat', 'embed'].some(k => status?.[k]?.starting
    || status?.activated && ['pending', 'downloading', 'verifying'].includes(downloadProgress(status[k]).phase)) ? 2000 : 10000;
}
export function startStatusPolling({ getStatus, enabled, onStatus, onError, onConnection = () => {},
  document: doc, window: win, timers = globalThis }) {
  let stopped = false, controller, timer, epoch = 0, status, transitioning = false;
  const visible = () => doc.visibilityState !== 'hidden' && enabled();
  function cancel() { epoch++; timers.clearTimeout(timer); controller?.abort(); }
  async function poll() {
    if (stopped || !visible()) return;
    const version = epoch;
    const ac = controller = new AbortController();
    try {
      const next = await getStatus({ signal: ac.signal });
      if (stopped || ac.signal.aborted || version !== epoch) return;
      status = next;
      onStatus(next);
    } catch (error) {
      if (!stopped && !ac.signal.aborted && version === epoch) onError(error);
    } finally {
      if (!stopped && !ac.signal.aborted && version === epoch && visible()) timer = timers.setTimeout(poll, statusPollDelay(status, transitioning));
    }
  }
  function refresh() { cancel(); void poll(); }
  function connection(event) {
    transitioning = event.detail?.code === 'SYNCING';
    onConnection(event.type, event.detail || {});
    refresh();
  }
  doc.addEventListener('visibilitychange', refresh);
  win.addEventListener('mc:resume', refresh);
  win.addEventListener('mc:connection-change', connection);
  win.addEventListener('mc:connection-health', connection);
  void poll();
  return { refresh, stop() {
    stopped = true; cancel();
    doc.removeEventListener('visibilitychange', refresh);
    win.removeEventListener('mc:resume', refresh);
    win.removeEventListener('mc:connection-change', connection);
    win.removeEventListener('mc:connection-health', connection);
  } };
}
