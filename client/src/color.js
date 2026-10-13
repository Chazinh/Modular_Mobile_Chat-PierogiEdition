export const validHex = value => /^#[0-9a-f]{6}$/i.test(value || '');

export function hexToHsl(hex) {
  const [r, g, b] = [1, 3, 5].map(i => parseInt((validHex(hex) ? hex : '#e7e7ea').slice(i, i + 2), 16) / 255);
  const max = Math.max(r, g, b), min = Math.min(r, g, b), delta = max - min;
  const l = (max + min) / 2;
  const s = delta ? delta / (1 - Math.abs(2 * l - 1)) : 0;
  const h = !delta ? 0 : max === r ? 60 * (((g - b) / delta + 6) % 6) : max === g ? 60 * ((b - r) / delta + 2) : 60 * ((r - g) / delta + 4);
  return { h, s: s * 100, l: l * 100 };
}

export function hslToHex({ h, s, l }) {
  h = ((h % 360) + 360) % 360; s = Math.max(0, Math.min(100, s)) / 100; l = Math.max(0, Math.min(100, l)) / 100;
  const a = s * Math.min(l, 1 - l);
  const channel = n => { const k = (n + h / 30) % 12; return Math.round(255 * (l - a * Math.max(-1, Math.min(k - 3, 9 - k, 1)))).toString(16).padStart(2, '0'); };
  return '#' + channel(0) + channel(8) + channel(4);
}
