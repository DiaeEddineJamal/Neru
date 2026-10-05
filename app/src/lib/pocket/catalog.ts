// Exact ten-model catalog from Google AI Edge Gallery (1.0.19), bundled for offline browsing.
import gallery from './gallery-models.json';

export type LocalConfig = { systemPrompt: string; maxTokens: number; contextTokens: number; topK: number; topP: number; temperature: number; accelerator: 'cpu' | 'gpu'; thinking: boolean; speculative: boolean };
export type LocalModel = { id: string; name: string; repo: string; file: string; revision: string; bytes: number; memory: number; description: string; url?: string; sha256?: string; kind: 'chat' | 'tools' | 'segmenter'; accelerators: ('cpu' | 'gpu')[]; thinking: boolean; speculative: boolean; maxContext: number; defaults: LocalConfig };
type GalleryModel = { name: string; modelId: string; modelFile: string; description: string; commitHash: string; sizeInBytes: number; minDeviceMemoryInGb?: number; url?: string; capabilities?: string[]; defaultConfig?: { topK: number; topP: number; temperature: number; maxTokens: number; maxContextLength?: number; accelerators: string }; taskTypes: string[] };
const ids: Record<string, string> = { 'Gemma-4-E2B-it': 'gemma-4-e2b', 'Gemma-4-E4B-it': 'gemma-4-e4b', 'Gemma3-1B-IT': 'gemma-3-1b', 'Gemma-3n-E2B-it': 'gemma-3n-e2b', 'Gemma-3n-E4B-it': 'gemma-3n-e4b' };
// Gallery's published MTP updates, verified against Hugging Face file metadata.
const updates: Record<string, { revision: string; bytes: number; sha256: string }> = {
  'Gemma-4-E2B-it': { revision: '7fa1d78473894f7e736a21d920c3aa80f950c0db', bytes: 2583085056, sha256: 'ab7838cdfc8f77e54d8ca45eadceb20452d9f01e4bfade03e5dce27911b27e42' },
  'Gemma-4-E4B-it': { revision: '9695417f248178c63a9f318c6e0c56cb917cb837', bytes: 3654467584, sha256: 'f335f2bfd1b758dc6476db16c0f41854bd6237e2658d604cbe566bcefd00a7bc' },
};
export const localModels: LocalModel[] = (gallery.models as GalleryModel[]).map(m => {
  const config = m.defaultConfig;
  const accelerators = (config?.accelerators.split(',') ?? ['cpu', 'gpu']).filter(a => a === 'cpu' || a === 'gpu') as ('cpu' | 'gpu')[];
  return {
    id: ids[m.name] ?? m.name.toLowerCase().replace(/[^a-z0-9]+/g, '-'), name: m.name.replace(/-it$/i, '').replace(/-/g, ' '),
    repo: m.modelId, file: m.modelFile, revision: updates[m.name]?.revision ?? m.commitHash, bytes: updates[m.name]?.bytes ?? m.sizeInBytes, memory: m.minDeviceMemoryInGb ?? 0,
    description: m.description.replace(/\[([^\]]+)\]\([^)]+\)/g, '$1'), url: m.url,
    sha256: updates[m.name]?.sha256 ?? (m.modelId === 'magic_touch' ? '38431bc66b883404e8397f74c3579404315b9b52b04a46c6346fe906a7309b03' : undefined),
    kind: m.taskTypes.includes('mp_scrapbook') ? 'segmenter' : m.taskTypes.includes('llm_chat') ? 'chat' : 'tools',
    accelerators, thinking: m.capabilities?.includes('llm_thinking') ?? false, speculative: m.capabilities?.includes('speculative_decoding') ?? false,
    maxContext: config?.maxContextLength ?? config?.maxTokens ?? 4096,
    // Advertise the full model limit, but start with a context that fits ordinary devices.
    defaults: { systemPrompt: 'You are Neru, a helpful assistant. Answer clearly and concisely.', maxTokens: Math.min(config?.maxTokens ?? 1024, 4096), contextTokens: Math.min(config?.maxContextLength ?? config?.maxTokens ?? 4096, 4096), topK: config?.topK ?? 64, topP: config?.topP ?? 0.95, temperature: config?.temperature ?? 1, accelerator: accelerators[0], thinking: false, speculative: false },
  };
});
export const localModel = (id: string) => localModels.find(m => m.id === id);
export const modelUrl = (m: LocalModel) => m.url ?? `https://huggingface.co/${m.repo}/resolve/${m.revision}/${m.file}`;
export const sizeLabel = (bytes: number) => bytes >= 1e9 ? `${(bytes / 1e9).toFixed(1)} GB` : `${Math.round(bytes / 1e6)} MB`;
