// Shared by endpoint selection and vault synchronization; internet endpoints always use HTTPS.
export function isLocalAddress(hostname) {
  if (!hostname) return false;
  const h = String(hostname).toLowerCase().replace(/^\[|\]$/g, '');
  if (h === 'localhost' || h === '127.0.0.1' || h === '::1' || h.endsWith('.local')) return true;
  if (/^10\.\d{1,3}\.\d{1,3}\.\d{1,3}$/.test(h)) return true;
  if (/^192\.168\.\d{1,3}\.\d{1,3}$/.test(h)) return true;
  const match172 = h.match(/^172\.(\d{1,3})\.\d{1,3}\.\d{1,3}$/);
  return !!match172 && Number(match172[1]) >= 16 && Number(match172[1]) <= 31;
}

export function normalizeServer(value, httpsOnly = true) {
  if (!String(value || '').trim()) return '';
  let raw = String(value).trim();
  if (!/^https?:\/\//i.test(raw) && !raw.startsWith('//')) {
    let hostname;
    try { hostname = new URL('https://' + raw).hostname; } catch { throw Error('Informe o endereço completo do PC.'); }
    raw = (!httpsOnly && isLocalAddress(hostname) ? 'http://' : 'https://') + raw;
  }
  let url;
  try { url = new URL(raw); } catch { throw Error('Informe o endereço completo do PC.'); }
  if (url.username || url.password || url.pathname !== '/' || url.search || url.hash) {
    throw Error('Informe somente o endereço do PC, sem senha, caminho ou parâmetros.');
  }
  if (url.protocol !== 'https:' && (httpsOnly || url.protocol !== 'http:' || !isLocalAddress(url.hostname))) {
    throw Error('Use HTTPS para endereços públicos. HTTP só é permitido na rede local.');
  }
  return url.origin;
}
