import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { publicPresets, mergeWithDefaults } from '../client/src/api.js';
import { watchProviderCatalog } from '../client/src/provider-catalog.js';

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const projectRoot = path.resolve(__dirname, '..');

test('Bug 1: watchProviderCatalog não descarta getCached quando snapshot muda e faz fallback se getPresets falhar', async () => {
  let snapshotVal = 'starting';
  const snapshot = () => snapshotVal;

  let delivered = null;
  const mockCached = { presets: [{ id: 'test-cached', label: 'Cached' }] };

  const win = new EventTarget();
  let failGetPresets = true;

  const cancel = watchProviderCatalog({
    window: win,
    snapshot,
    getCached: async () => {
      snapshotVal = 'local-active'; // snapshot mudou durante resolução do cache!
      return mockCached;
    },
    getPresets: async () => {
      if (failGetPresets) throw new Error('Host offline');
      return { presets: [{ id: 'test-remote', label: 'Remote' }] };
    },
    onPresets: (val) => {
      delivered = val;
    },
  });

  // Aguarda ticks do event loop
  await new Promise((r) => setTimeout(r, 60));
  assert.ok(delivered, 'onPresets deve ter sido chamado com presets em cache mesmo com snapshot transicionado');
  assert.equal(delivered.presets[0].id, 'test-cached');

  cancel();
});

test('Bug 1: ChatView e SettingsDrawer usam effectivePresets com fallback imediato aos presets padrão', () => {
  const chatViewSrc = fs.readFileSync(path.join(projectRoot, 'client', 'src', 'components', 'ChatView.jsx'), 'utf8');
  assert.ok(
    chatViewSrc.includes('useState(() => publicPresets(DEFAULT_PRESETS))'),
    'ChatView deve inicializar presets com os presets padrão em vez de null'
  );

  const drawerSrc = fs.readFileSync(path.join(projectRoot, 'client', 'src', 'components', 'SettingsDrawer.jsx'), 'utf8');
  assert.ok(
    drawerSrc.includes('effectivePresets'),
    'SettingsDrawer deve calcular effectivePresets para evitar presets vazios/nulos'
  );
  assert.ok(
    drawerSrc.includes('const effectivePresets = presets?.presets?.length ? presets : mergeWithDefaults(publicPresets(presets)) || publicPresets(DEFAULT_PRESETS);'),
    'SettingsDrawer deve conter fallback seguro para DEFAULT_PRESETS'
  );
});

test('Bug 2: SubPage renderiza direto em mc-drawer-main-scroller e inclui cabeçalho na StairList', () => {
  const drawerSrc = fs.readFileSync(path.join(projectRoot, 'client', 'src', 'components', 'SettingsDrawer.jsx'), 'utf8');

  // SubPage não deve ter o wrapper flex intermediário que quebrava o layout com mc-stair-free
  assert.ok(
    !drawerSrc.includes('<div className="flex h-full min-h-0 flex-col">'),
    'SubPage não deve conter o wrapper flex intermediário que causava blowout no layout'
  );

  // SubPage deve usar mc-drawer-main-scroller
  assert.ok(
    drawerSrc.includes('<div className="mc-drawer-main-scroller min-h-0 flex-1 overflow-y-auto">'),
    'SubPage deve usar mc-drawer-main-scroller diretamente'
  );

  // Cabeçalho e footer devem estar dentro da StairList para participar da escada (staircase)
  const subPageRegex = /export function SubPage\(\{[\s\S]*?return \([\s\S]*?<StairList className="min-h-full">([\s\S]*?)<\/StairList>/;
  const match = drawerSrc.match(subPageRegex);
  assert.ok(match, 'SubPage deve renderizar uma StairList');
  const stairListContent = match[1];

  assert.ok(stairListContent.includes('sticky top-0 z-20'), 'Cabeçalho deve estar dentro da StairList com sticky');
  assert.ok(stairListContent.includes('{children}'), 'Children deve estar dentro da StairList');
  assert.ok(stairListContent.includes('{footer && <div className="mc-subfooter">{footer}</div>}'), 'Footer deve estar dentro da StairList');

  // Ao mudar de página no drawer, motion stagger deve ser liberado (release)
  assert.ok(
    drawerSrc.includes('motionRef.current?.stagger?.release();'),
    'SettingsDrawer deve liberar o stagger ao navegar entre páginas para atualizar os elementos da escada'
  );
});
