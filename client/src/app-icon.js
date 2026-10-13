import { Capacitor, registerPlugin } from '@capacitor/core';

// App icons. On Android each one is an activity-alias (AndroidManifest.xml); AppIconPlugin enables the chosen one
// and disables the others when the app leaves the screen, so the launcher does not kill the app mid-use.
export const APP_ICONS = [
  {
    id: 'pierogi_dark',
    name: 'Pierogi Dark',
    subtitle: 'Vidro líquido escuro iridescente',
    preview: '/icons/pierogi-dark-192.png',
    tile: '/icons/pierogi-dark-tile-192.png', // cropped without the transparent margin (in-app preview)
  },
  {
    id: 'pierogi_light',
    name: 'Pierogi Light',
    subtitle: 'Vidro líquido claro translúcido',
    preview: '/icons/pierogi-light-192.png',
    tile: '/icons/pierogi-light-tile-192.png', // cropped without the transparent margin (in-app preview)
  },
];

const KEY = 'modularchat_app_icon';
const native = Capacitor.isNativePlatform();
const NativeIcon = registerPlugin('AppIcon');
const known = (id) => APP_ICONS.find((i) => i.id === id) || APP_ICONS[0];

export function getSelectedAppIcon() {
  try {
    return known(localStorage.getItem(KEY)).id;
  } catch {
    return APP_ICONS[0].id;
  }
}

function setFavicon(icon) {
  if (typeof document === 'undefined') return;
  for (const rel of ['icon', 'apple-touch-icon']) {
    let link = document.querySelector(`link[rel='${rel}']`);
    if (!link) {
      link = document.createElement('link');
      link.rel = rel;
      document.head.appendChild(link);
    }
    link.href = icon.preview;
  }
}

// Returns the applied id. On the device the home-screen icon change is scheduled for when the app goes to the
// background; `pending` tells the screen to say so.
export async function applyAppIcon(iconId) {
  const icon = known(iconId);
  try {
    localStorage.setItem(KEY, icon.id);
  } catch {}
  setFavicon(icon);
  if (!native) return { id: icon.id, pending: false };
  const r = await NativeIcon.set({ id: icon.id });
  return { id: icon.id, pending: !!r?.pending };
}

export async function initAppIcon() {
  let id = getSelectedAppIcon();
  if (native) {
    try {
      // the device is the source of truth (the icon may have been changed by an earlier install)
      const r = await NativeIcon.get();
      id = known(r?.id).id;
      localStorage.setItem(KEY, id);
    } catch {}
  }
  setFavicon(known(id));
  return id;
}
