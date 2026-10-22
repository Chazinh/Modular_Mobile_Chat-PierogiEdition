import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import { DEFAULT_PRESETS } from '../client/src/default-presets.js';

// Dynamically extract and load isReasoningModel directly from SettingsDrawer.jsx
const settingsDrawerSource = fs.readFileSync(new URL('../client/src/components/SettingsDrawer.jsx', import.meta.url), 'utf8');
const fnMatch = settingsDrawerSource.match(/export function isReasoningModel\(preset\) \{[\s\S]*?\n\}/);
if (!fnMatch) {
  throw new Error('isReasoningModel function declaration not found in SettingsDrawer.jsx');
}
const { isReasoningModel } = await import('data:text/javascript,' + encodeURIComponent(fnMatch[0]));

test('R3: isReasoningModel returns false for falsy or empty presets', () => {
  assert.equal(isReasoningModel(null), false, 'null preset must return false');
  assert.equal(isReasoningModel(undefined), false, 'undefined preset must return false');
  assert.equal(isReasoningModel({}), false, 'empty preset object must return false');
});

test('R3: isReasoningModel returns false for mobile-local and generic', () => {
  assert.equal(isReasoningModel({ id: 'mobile-local', efforts: [] }), false, 'mobile-local by id must return false');
  assert.equal(isReasoningModel({ id: 'generic', efforts: [] }), false, 'generic by id must return false');
  assert.equal(isReasoningModel({ id: 'custom-local', family: 'local', efforts: ['low'] }), false, 'family local must return false');

  const mobileLocal = DEFAULT_PRESETS.presets.find((p) => p.id === 'mobile-local');
  if (mobileLocal) {
    assert.equal(isReasoningModel(mobileLocal), false, 'mobile-local from DEFAULT_PRESETS must return false');
  }

  const generic = DEFAULT_PRESETS.presets.find((p) => p.id === 'generic');
  if (generic) {
    assert.equal(isReasoningModel(generic), false, 'generic from DEFAULT_PRESETS must return false');
  }
});

test('R3: isReasoningModel returns true for reasoning models', () => {
  const expectedReasoningIds = [
    'deepseek-v41-flash',
    'glm-52',
    'muse-spark-13',
    'gemini-38-flash',
    'claude-sonnet-5-5',
    'claude-opus-5-5',
    'nemotron-35',
    'minimax-m31',
    'gemma-4',
  ];

  for (const id of expectedReasoningIds) {
    const preset = DEFAULT_PRESETS.presets.find((p) => p.id === id);
    assert.ok(preset, `Preset ${id} must exist in DEFAULT_PRESETS`);
    assert.equal(
      isReasoningModel(preset),
      true,
      `isReasoningModel must return true for reasoning preset: ${id}`
    );
  }
});

test('R3: Thinking switch display condition evaluates to false for non-reasoning models and models that cannot disable thinking', () => {
  const canShowThinkingSwitch = (p) => isReasoningModel(p) && p?.thinkingCanDisable === true;

  // Non-reasoning models must evaluate to false
  assert.equal(canShowThinkingSwitch({ id: 'mobile-local', thinkingCanDisable: true }), false, 'mobile-local even with thinkingCanDisable true must be false');
  assert.equal(canShowThinkingSwitch({ id: 'generic', thinkingCanDisable: true }), false, 'generic even with thinkingCanDisable true must be false');
  assert.equal(canShowThinkingSwitch({ id: 'other', family: 'local', thinkingCanDisable: true }), false, 'local family must be false');
  assert.equal(canShowThinkingSwitch({ id: 'no-efforts', efforts: [], thinking: 'none', thinkingCanDisable: true }), false, 'none thinking must be false');

  // Reasoning models that CANNOT disable thinking (e.g. Gemini, Opus) must evaluate to false
  const gemini = DEFAULT_PRESETS.presets.find((p) => p.id === 'gemini-38-flash');
  assert.ok(gemini);
  assert.equal(isReasoningModel(gemini), true, 'gemini-38-flash is reasoning model');
  assert.equal(canShowThinkingSwitch(gemini), false, 'gemini-38-flash switch display condition must be false (thinking cannot disable)');

  const opus = DEFAULT_PRESETS.presets.find((p) => p.id === 'claude-opus-5-5');
  assert.ok(opus);
  assert.equal(isReasoningModel(opus), true, 'claude-opus-5-5 is reasoning model');
  assert.equal(canShowThinkingSwitch(opus), false, 'claude-opus-5-5 switch display condition must be false (thinking cannot disable)');

  // Reasoning models that CAN disable thinking must evaluate to true
  const controllableModels = [
    'deepseek-v41-flash',
    'glm-52',
    'muse-spark-13',
    'claude-sonnet-5-5',
    'nemotron-35',
    'minimax-m31',
    'gemma-4',
  ];

  for (const id of controllableModels) {
    const p = DEFAULT_PRESETS.presets.find((x) => x.id === id);
    assert.ok(p, `Preset ${id} must exist`);
    assert.equal(canShowThinkingSwitch(p), true, `canShowThinkingSwitch must be true for ${id}`);
  }
});

test('R3: SettingsDrawer.jsx source code conforms strictly to R3 reasoning gate requirements', () => {
  // 1. Exportable helper function exists
  assert.ok(
    settingsDrawerSource.includes('export function isReasoningModel(preset)'),
    'SettingsDrawer.jsx must declare and export isReasoningModel(preset)'
  );

  // 2. Main page thinking switch condition
  assert.ok(
    settingsDrawerSource.includes('isReasoningModel(preset) && preset?.thinkingCanDisable === true'),
    'Main page switch must be strictly guarded by isReasoningModel(preset) && preset?.thinkingCanDisable === true'
  );

  // 3. ModelPage switch condition
  assert.ok(
    settingsDrawerSource.includes('isReasoningModel(p) && p.thinkingCanDisable === true'),
    'ModelPage switch must be strictly guarded by isReasoningModel(p) && p.thinkingCanDisable === true'
  );

  // 4. ModelPage fallback text only rendered for reasoning models that cannot disable thinking
  assert.ok(
    settingsDrawerSource.includes(') : isReasoningModel(p) ? ('),
    'ModelPage fallback disclaimer text must only be shown for reasoning models'
  );

  // 5. ModelPage state handling curCanDisable
  assert.ok(
    settingsDrawerSource.includes('const curCanDisable = isReasoningModel(cur) && cur?.thinkingCanDisable === true;'),
    'ModelPage curCanDisable must be guarded by isReasoningModel(cur) && cur?.thinkingCanDisable === true'
  );
});
