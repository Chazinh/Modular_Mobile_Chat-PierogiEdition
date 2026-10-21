import test from 'node:test';
import assert from 'node:assert/strict';
import {
  CURATED_MODELS,
  formatBytes,
  estimateRatings,
  cleanModelName,
  fetchHuggingFaceModels
} from '../client/src/huggingface.js';

test('huggingface: CURATED_MODELS possui modelos oficiais de chat e embed para celular', () => {
  const officialChat = CURATED_MODELS.find((m) => m.isOfficialChat);
  const officialEmbed = CURATED_MODELS.find((m) => m.isOfficialEmbed);

  assert.ok(officialChat, 'Modelo oficial de chat deve existir');
  assert.equal(officialChat.type, 'chat');
  assert.ok(officialChat.repo.includes('Qwen2.5-1.5B'));
  assert.equal(officialChat.precision, 4);
  assert.equal(officialChat.speed, 4);

  assert.ok(officialEmbed, 'Modelo oficial de embeddings deve existir');
  assert.equal(officialEmbed.type, 'embed');
  assert.ok(officialEmbed.repo.includes('embeddinggemma-300M'));
  assert.equal(officialEmbed.precision, 5);
  assert.equal(officialEmbed.speed, 5);

  // All curated models must have name, format, badge and valid ratings
  for (const model of CURATED_MODELS) {
    assert.ok(model.name, 'Nome do modelo deve ser definido');
    assert.ok(model.repo, 'Repo do Hugging Face deve ser definido');
    assert.ok(model.precision >= 1 && model.precision <= 5);
    assert.ok(model.speed >= 1 && model.speed <= 5);
    assert.ok(['chat', 'embed'].includes(model.type));
  }
});

test('huggingface: formatBytes formata tamanhos de bytes em MB e GB', () => {
  assert.equal(formatBytes(0), '—');
  assert.equal(formatBytes(-100), '—');
  assert.ok(formatBytes(45000000).includes('MB'));
  assert.ok(formatBytes(333590944).includes('318'));
  assert.ok(formatBytes(1117320736).includes('GB'));
});

test('huggingface: estimateRatings estima precisão e velocidade conforme o porte do modelo', () => {
  // Ultra-light models: speed 5, precision 3
  const tiny = estimateRatings('qwen2.5-0.5b-instruct', ['gguf', '0.5b']);
  assert.equal(tiny.speed, 5);
  assert.equal(tiny.precision, 3);

  // Micro embeddings: speed 5, precision 3
  const micro = estimateRatings('bge-micro-v2', ['embeddings']);
  assert.equal(micro.speed, 5);
  assert.equal(micro.precision, 3);

  // Balanced 1.5B / 2B models: speed 4, precision 4
  const balanced = estimateRatings('qwen2.5-1.5b-instruct', ['1.5b', 'chat']);
  assert.equal(balanced.speed, 4);
  assert.equal(balanced.precision, 4);

  // 7B+ heavy models: speed 2, precision 5
  const heavy = estimateRatings('qwen2.5-7b-instruct', ['7b']);
  assert.equal(heavy.speed, 2);
  assert.equal(heavy.precision, 5);
});

test('huggingface: cleanModelName remove sufixos técnicos de repositório', () => {
  assert.equal(cleanModelName('Qwen/Qwen2.5-1.5B-Instruct-GGUF'), 'Qwen2.5 1.5B Instruct');
  assert.equal(cleanModelName('bartowski/Llama-3.2-1B-Instruct-GGUF'), 'Llama 3.2 1B Instruct');
  assert.equal(cleanModelName('custom_model.gguf'), 'custom model');
});

test('huggingface: fetchHuggingFaceModels retorna modelos da API e une com os curados', async () => {
  const originalFetch = globalThis.fetch;

  try {
    globalThis.fetch = async () => ({
      ok: true,
      json: async () => [
        {
          id: 'test-org/Mock-Mobile-LLM-GGUF',
          downloads: 125000,
          likes: 320,
          tags: ['gguf', 'chat', 'conversational', 'pt', 'en', '1b'],
          pipeline_tag: 'text-generation',
        },
      ],
    });

    const results = await fetchHuggingFaceModels('Mock');
    assert.ok(Array.isArray(results));

    const mockItem = results.find((m) => m.repo === 'test-org/Mock-Mobile-LLM-GGUF');
    assert.ok(mockItem, 'Item retornado do Hugging Face deve ser encontrado');
    assert.equal(mockItem.badge, 'Hugging Face');
    assert.equal(mockItem.languages, 'Português & English');
    assert.equal(mockItem.precision, 4);
    assert.equal(mockItem.speed, 4);
  } finally {
    globalThis.fetch = originalFetch;
  }
});

test('huggingface: fetchHuggingFaceModels cai em fallback gracioso nos curados em caso de falha de rede', async () => {
  const originalFetch = globalThis.fetch;

  try {
    globalThis.fetch = async () => {
      throw new Error('Network error / offline');
    };

    const results = await fetchHuggingFaceModels('qwen');
    assert.ok(Array.isArray(results));
    assert.ok(results.length > 0, 'Deve retornar modelos curados mesmo offline');
    assert.ok(results.some((m) => m.name.includes('Qwen')));
  } finally {
    globalThis.fetch = originalFetch;
  }
});
