// Hugging Face integration for local mobile AI models in ModularChat
import { tr } from './i18n.js';

export const CURATED_MODELS = [
  {
    id: 'nemotron-3.5-bolt-e4b-4gb',
    repo: 'nvidia/NVIDIA-Nemotron-3.5-Bolt-E4B-4GB',
    filename: 'Nemotron-3.5-Bolt-E4B-Q4_K_M.gguf',
    name: 'Nemotron 3.5 Bolt E4B 4GB',
    badge: 'Recomendado',
    type: 'chat',
    isOfficialChat: true,
    description: 'Modelo oficial recomendado para conversa e extração de memória. Rápido, inteligente e multilíngue.',
    languages: '30+ idiomas · pt, en, es...',
    format: 'GGUF · Q4_K_M',
    sizeBytes: 6180446208,
    sizeFormatted: '6,2 GB',
    precision: 5,
    speed: 4,
    downloads: 'NVIDIA',
  },
  {
    id: 'embeddinggemma-300m',
    repo: 'ggml-org/embeddinggemma-300M-GGUF',
    filename: 'embeddinggemma-300M-Q8_0.gguf',
    name: 'EmbeddingGemma 300M',
    badge: 'Recomendado',
    type: 'embed',
    isOfficialEmbed: true,
    description: 'Modelo oficial de embeddings para memória de longo prazo e busca vetorial no aparelho.',
    languages: 'Multilíngue · Vetorial',
    format: 'GGUF · Q8_0',
    sizeBytes: 333590944,
    sizeFormatted: '334 MB',
    precision: 5,
    speed: 5,
    downloads: '850k',
  },
  {
    id: 'qwen-2.5-1.5b-instruct',
    repo: 'Qwen/Qwen2.5-1.5B-Instruct-GGUF',
    filename: 'qwen2.5-1.5b-instruct-q4_k_m.gguf',
    name: 'Qwen 2.5 1.5B Instruct',
    badge: 'Compacto',
    type: 'chat',
    description: 'Alternativa ultra compacta para conversa e extração no celular.',
    languages: '28 idiomas · pt-BR, en, es...',
    format: 'GGUF · Q4_K_M',
    sizeBytes: 1117320736,
    sizeFormatted: '1,1 GB',
    precision: 4,
    speed: 5,
    downloads: '1.2M',
  },
  {
    id: 'qwen-2.5-0.5b-instruct',
    repo: 'Qwen/Qwen2.5-0.5B-Instruct-GGUF',
    filename: 'qwen2.5-0.5b-instruct-q4_k_m.gguf',
    name: 'Qwen 2.5 0.5B Instruct',
    badge: 'Ultra Leve',
    type: 'chat',
    description: 'Mínimo e instantâneo. Roda com consumo insignificante de RAM e resposta imediata.',
    languages: 'Multilíngue (12 idiomas)',
    format: 'GGUF · Q4_K_M',
    sizeBytes: 397000000,
    sizeFormatted: '397 MB',
    precision: 3,
    speed: 5,
    downloads: '450k',
  },
  {
    id: 'smollm2-1.7b-instruct',
    repo: 'HuggingFaceTB/SmolLM2-1.7B-Instruct-GGUF',
    filename: 'smollm2-1.7b-instruct-q4_k_m.gguf',
    name: 'SmolLM2 1.7B Instruct',
    badge: 'Hugging Face',
    type: 'chat',
    description: 'Modelo moderno da Hugging Face focado em raciocínio conciso e assistente para celular.',
    languages: 'English, Português básico',
    format: 'GGUF · Q4_K_M',
    sizeBytes: 1060000000,
    sizeFormatted: '1,06 GB',
    precision: 4,
    speed: 4,
    downloads: '620k',
  },
  {
    id: 'llama-3.2-1b-instruct',
    repo: 'bartowski/Llama-3.2-1B-Instruct-GGUF',
    filename: 'Llama-3.2-1B-Instruct-Q4_K_M.gguf',
    name: 'Llama 3.2 1B Instruct',
    badge: 'Meta AI',
    type: 'chat',
    description: 'Excelente arquitetura compacta da Meta para diálogos expressivos e personas no aparelho.',
    languages: 'Multilíngue (8 idiomas)',
    format: 'GGUF · Q4_K_M',
    sizeBytes: 770000000,
    sizeFormatted: '770 MB',
    precision: 4,
    speed: 4,
    downloads: '910k',
  },
  {
    id: 'bge-micro-v2',
    repo: 'TaylorAI/bge-micro-v2',
    filename: 'bge-micro-v2.gguf',
    name: 'BGE Micro V2',
    badge: 'Micro',
    type: 'embed',
    description: 'Embeddings ultraleves para indexação rápida de memória com consumo mínimo de armazenamento.',
    languages: 'Multilíngue',
    format: 'GGUF',
    sizeBytes: 45000000,
    sizeFormatted: '45 MB',
    precision: 3,
    speed: 5,
    downloads: '180k',
  },
];

export function formatBytes(bytes) {
  const n = Number(bytes) || 0;
  if (n <= 0) return '—';
  if (n >= 1024 * 1024 * 1024) {
    return `${(n / (1024 * 1024 * 1024)).toLocaleString(undefined, { minimumFractionDigits: 1, maximumFractionDigits: 1 })} GB`;
  }
  return `${(n / (1024 * 1024)).toLocaleString(undefined, { minimumFractionDigits: 0, maximumFractionDigits: 1 })} MB`;
}

export function estimateRatings(name = '', tags = []) {
  const str = `${name} ${tags.join(' ')}`.toLowerCase();
  let precision = 4;
  let speed = 4;

  if (str.includes('0.5b') || str.includes('300m') || str.includes('micro') || str.includes('tiny')) {
    precision = 3;
    speed = 5;
  } else if (str.includes('1b') || str.includes('1.5b') || str.includes('1.7b') || str.includes('2b')) {
    precision = 4;
    speed = 4;
  } else if (str.includes('3b') || str.includes('4b')) {
    precision = 5;
    speed = 3;
  } else if (str.includes('7b') || str.includes('8b') || str.includes('9b') || str.includes('14b')) {
    precision = 5;
    speed = 2;
  }

  return { precision, speed };
}

export function cleanModelName(repoId = '') {
  const parts = repoId.split('/');
  const name = parts[parts.length - 1] || repoId;
  return name
    .replace(/-GGUF$/i, '')
    .replace(/\.gguf$/i, '')
    .replace(/[-_]/g, ' ')
    .trim();
}

export async function fetchHuggingFaceModels(query = '', { signal } = {}) {
  const trimmed = query.trim().toLowerCase();
  const curatedMatches = CURATED_MODELS.filter((m) => {
    if (!trimmed) return true;
    return m.name.toLowerCase().includes(trimmed)
      || m.repo.toLowerCase().includes(trimmed)
      || m.description.toLowerCase().includes(trimmed)
      || m.languages.toLowerCase().includes(trimmed);
  });

  try {
    const searchTerm = trimmed ? `${trimmed} gguf` : 'gguf';
    const url = `https://huggingface.co/api/models?search=${encodeURIComponent(searchTerm)}&filter=gguf&sort=downloads&direction=-1&limit=25`;
    const res = await fetch(url, { signal, headers: { Accept: 'application/json' } });
    if (!res.ok) throw new Error(`HTTP ${res.status}`);
    const items = await res.json();

    const existingRepos = new Set(CURATED_MODELS.map((m) => m.repo.toLowerCase()));
    const fetched = (Array.isArray(items) ? items : [])
      .filter((item) => item.id && !existingRepos.has(item.id.toLowerCase()))
      .map((item) => {
        const tags = Array.isArray(item.tags) ? item.tags : [];
        const isEmbed = tags.some((t) => t.includes('embedding') || t.includes('feature-extraction'))
          || (item.pipeline_tag === 'feature-extraction');
        const isChat = !isEmbed;
        const name = cleanModelName(item.id);
        const { precision, speed } = estimateRatings(name, tags);

        // Language tag detection
        let languages = 'Multilíngue';
        if (tags.includes('pt') && tags.includes('en')) languages = 'Português & English';
        else if (tags.includes('pt')) languages = 'Português';
        else if (tags.includes('en')) languages = 'English';

        const downloads = item.downloads ? (item.downloads > 1000000 ? `${(item.downloads / 1000000).toFixed(1)}M` : item.downloads > 1000 ? `${Math.round(item.downloads / 1000)}k` : item.downloads) : null;

        return {
          id: item.id,
          repo: item.id,
          name,
          badge: 'Hugging Face',
          type: isChat ? 'chat' : 'embed',
          description: tr('Disponibilizado na Hugging Face por {0}', [item.id.split('/')[0] || 'comunidade']),
          languages,
          format: 'GGUF',
          sizeBytes: null,
          sizeFormatted: tags.find((t) => t.match(/^[0-9]+[bB]$/)) || 'GGUF',
          precision,
          speed,
          downloads: downloads ? `${downloads}` : null,
          likes: item.likes || null,
          url: `https://huggingface.co/${item.id}`,
        };
      });

    return [...curatedMatches, ...fetched];
  } catch {
    // If offline or network fails, gracefully return the curated models matching the query
    return curatedMatches;
  }
}
