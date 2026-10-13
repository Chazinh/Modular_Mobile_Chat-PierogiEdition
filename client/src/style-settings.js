import { api } from './api.js';

let pending = {};
let saving = null;

export function queueStyleSettings(patch) {
  pending = { ...pending, ...patch };
}

// A shared queue prevents out-of-order PUTs and lets the save finish
// when the form unmounts. Sending a message waits on the same queue.
export async function flushStyleSettings() {
  if (saving) {
    await saving;
    return flushStyleSettings();
  }
  if (!Object.keys(pending).length) return null;
  const body = pending;
  pending = {};
  saving = api.saveGlobalSettings(body);
  let result;
  try {
    result = await saving;
  } catch (error) {
    pending = { ...body, ...pending };
    throw error;
  } finally {
    saving = null;
  }
  return (await flushStyleSettings()) || result;
}
