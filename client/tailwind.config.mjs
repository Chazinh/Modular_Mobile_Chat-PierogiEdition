import colors from 'tailwindcss/colors.js';

// Eye care without a full-screen blend layer: every palette color is multiplied by three per-channel factors
// (--mc-tr/--mc-tg/--mc-tb, set by src/eyecare.js). Without the filter the factors are 1 and the color is unchanged;
// with the blue-light filter the blue channel (and a little of the green) drops in backgrounds, surfaces, borders and accents.
// Same result as the old multiplied overlay, but resolved in the color itself (nothing to recomposite per frame).
const tintOf = (hex) => {
  const [r, g, b] = [1, 3, 5].map((i) => parseInt(hex.slice(i, i + 2), 16));
  return ({ opacityValue } = {}) => {
    const alpha = opacityValue === undefined ? '' : ` / ${opacityValue}`;
    return `rgb(calc(${r} * var(--mc-tr, 1)) calc(${g} * var(--mc-tg, 1)) calc(${b} * var(--mc-tb, 1))${alpha})`;
  };
};
const tintShades = (shades) => Object.fromEntries(Object.entries(shades).map(([k, v]) => [k, tintOf(v)]));
const tintedFamilies = ['slate', 'gray', 'zinc', 'neutral', 'stone', 'red', 'orange', 'amber', 'yellow', 'lime', 'green', 'emerald', 'teal', 'cyan', 'sky', 'blue', 'indigo', 'violet', 'purple', 'fuchsia', 'pink', 'rose'];

/** @type {import('tailwindcss').Config} */
export default {
  content: ['./index.html', './src/**/*.{js,ts,jsx,tsx}'],
  theme: {
    colors: {
      transparent: 'transparent',
      current: 'currentColor',
      inherit: 'inherit',
      white: tintOf('#ffffff'),
      black: tintOf('#000000'),
      ...Object.fromEntries(tintedFamilies.map((f) => [f, tintShades(colors[f])])),
      bgDark: tintOf('#1b1b1d'), // fundo
      cardDark: tintOf('#26252a'), // reply card
      cardDarkHover: tintOf('#302f36'),
      composer: tintOf('#28282a'), // compositor
      headerDark: tintOf('#25242f'), // glass base with gradient
      sheet: tintOf('#252430'),
      accentPurple: tintOf('#7c3aed'),
      accentPurpleLight: tintOf('#a78bfa'),
      textMuted: tintOf('#9a9aa3'),
      borderDark: tintOf('#34343a'),
    },
    extend: {
      // Text: follows the color and brightness from Settings → Eye care (--mc-* variables).
      // Also applies to placeholder:text-*, hover:text-* etc. Semantic colors (red/yellow/emerald/accentPurpleLight) stay as they are.
      textColor: {
        white: 'var(--mc-white, #ffffff)',
        textMuted: 'var(--mc-muted, #9a9aa3)',
        gray: { 100: 'var(--mc-text, #f3f4f6)', 200: 'var(--mc-text, #e5e7eb)', 300: 'var(--mc-text, #d1d5db)', 400: 'var(--mc-muted, #9ca3af)' },
        // text on a button filled with the accent color: not dimmed by the brightness setting (only tinted by the blue-light filter)
        onAccent: 'var(--mc-onaccent, #ffffff)',
      },
    },
  },
  plugins: [],
};
