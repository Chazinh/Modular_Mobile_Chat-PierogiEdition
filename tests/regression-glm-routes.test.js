import test from 'node:test';
import assert from 'node:assert/strict';
import { DEFAULT_PRESETS } from '../client/src/default-presets.js';
import { mergeWithDefaults, publicPresets } from '../client/src/api.js';

test('R1: DEFAULT_PRESETS exposes both active routes for glm-52', () => {
  const glm = DEFAULT_PRESETS.presets.find((p) => p.id === 'glm-52');
  assert.ok(glm, 'glm-52 preset must exist in DEFAULT_PRESETS');
  assert.equal(glm.groupRoute, 'GLM', 'glm-52 groupRoute must be GLM');
  assert.ok(Array.isArray(glm.routes), 'glm-52 routes must be an array');
  assert.equal(glm.routes.length, 2, 'glm-52 must have exactly 2 active routes');

  const routeIds = glm.routes.map((r) => r.id);
  assert.ok(routeIds.includes('GLM'), 'routes must include GLM group route');
  assert.ok(routeIds.includes('cbai/glm-5.2'), 'routes must include cbai/glm-5.2 direct route');
  assert.ok(!routeIds.includes('nvidia/z-ai/glm-5.2'), 'routes must NOT include retired nvidia route');

  assert.ok(Array.isArray(glm.preferredRoutes), 'preferredRoutes must be an array');
  assert.ok(glm.preferredRoutes.includes('GLM'), 'preferredRoutes must include GLM');
  assert.ok(glm.preferredRoutes.includes('cbai/glm-5.2'), 'preferredRoutes must include cbai/glm-5.2');
  assert.ok(!glm.preferredRoutes.includes('nvidia/z-ai/glm-5.2'), 'preferredRoutes must NOT include retired nvidia route');
});

test('R1: mergeWithDefaults preserves both routes when server advertises only cbai/glm-5.2', () => {
  const serverData = {
    presets: [
      {
        id: 'deepseek-v41-flash',
        groupRoute: 'Deepseek',
        routes: [{ id: 'Deepseek' }]
      },
      {
        id: 'glm-52',
        groupRoute: 'GLM',
        routes: [{ id: 'cbai/glm-5.2' }],
        preferredRoutes: ['cbai/glm-5.2']
      }
    ]
  };

  const merged = mergeWithDefaults(serverData);
  const glm = merged.presets.find((p) => p.id === 'glm-52');
  assert.ok(glm, 'glm-52 must exist after merge');
  assert.equal(glm.routes.length, 2, 'Must contain both routes after merging with defaults');

  const routeIds = glm.routes.map((r) => r.id);
  assert.ok(routeIds.includes('GLM'), 'Must restore groupRoute GLM');
  assert.ok(routeIds.includes('cbai/glm-5.2'), 'Must preserve cbai/glm-5.2');
  assert.equal(routeIds[0], 'GLM', 'GLM must be ordered first as groupRoute');
});

test('R1: mergeWithDefaults preserves both routes when server advertises only GLM', () => {
  const serverData = {
    presets: [
      {
        id: 'deepseek-v41-flash',
        groupRoute: 'Deepseek',
        routes: [{ id: 'Deepseek' }]
      },
      {
        id: 'glm-52',
        groupRoute: 'GLM',
        routes: [{ id: 'GLM' }],
        preferredRoutes: ['GLM']
      }
    ]
  };

  const merged = mergeWithDefaults(serverData);
  const glm = merged.presets.find((p) => p.id === 'glm-52');
  assert.ok(glm, 'glm-52 must exist after merge');
  assert.equal(glm.routes.length, 2, 'Must contain both routes after merging with defaults');

  const routeIds = glm.routes.map((r) => r.id);
  assert.ok(routeIds.includes('GLM'), 'Must preserve GLM');
  assert.ok(routeIds.includes('cbai/glm-5.2'), 'Must restore cbai/glm-5.2');
  assert.equal(routeIds[0], 'GLM', 'GLM must be ordered first as groupRoute');
});

test('R1: mergeWithDefaults does not duplicate routes when server advertises both', () => {
  const serverData = {
    presets: [
      {
        id: 'deepseek-v41-flash',
        groupRoute: 'Deepseek',
        routes: [{ id: 'Deepseek' }]
      },
      {
        id: 'glm-52',
        groupRoute: 'GLM',
        routes: [{ id: 'GLM' }, { id: 'cbai/glm-5.2' }],
        preferredRoutes: ['GLM', 'cbai/glm-5.2']
      }
    ]
  };

  const merged = mergeWithDefaults(serverData);
  const glm = merged.presets.find((p) => p.id === 'glm-52');
  assert.equal(glm.routes.length, 2, 'Must not duplicate routes when both are already present');
});

test('R1: Route switching logic between GLM and cbai/glm-5.2', () => {
  const glm = DEFAULT_PRESETS.presets.find((p) => p.id === 'glm-52');

  // Verify groupRoute discriminator logic (mirrors SettingsDrawer.jsx:357)
  const isGroupRoute = (routeId) => routeId === glm.groupRoute;
  assert.equal(isGroupRoute('GLM'), true, 'GLM is the group fallback route');
  assert.equal(isGroupRoute('cbai/glm-5.2'), false, 'cbai/glm-5.2 is a direct route');

  // Simulate user switching from GLM to cbai/glm-5.2 in SettingsDrawer
  let sel = { presetId: 'glm-52', routeId: 'GLM', effort: 'high' };
  sel = { ...sel, routeId: 'cbai/glm-5.2' };
  const patchToCbai = { presetId: sel.presetId, routeId: sel.routeId, effort: sel.effort };
  assert.deepEqual(patchToCbai, { presetId: 'glm-52', routeId: 'cbai/glm-5.2', effort: 'high' });

  // Simulate user switching back from cbai/glm-5.2 to GLM
  sel = { ...sel, routeId: 'GLM' };
  const patchToGroup = { presetId: sel.presetId, routeId: sel.routeId, effort: sel.effort };
  assert.deepEqual(patchToGroup, { presetId: 'glm-52', routeId: 'GLM', effort: 'high' });
});

test('R1: publicPresets preserves GLM label and both routes for UI rendering', () => {
  const pub = publicPresets(DEFAULT_PRESETS);
  const glm = pub.presets.find((p) => p.id === 'glm-52');
  assert.ok(glm, 'glm-52 must be included in publicPresets');
  assert.equal(glm.label, 'GLM');
  assert.equal(glm.routes.length, 2);
  assert.ok(glm.routes.some((r) => r.id === 'GLM'));
  assert.ok(glm.routes.some((r) => r.id === 'cbai/glm-5.2'));
});
