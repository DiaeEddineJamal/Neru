/** On-device speech-to-text models, run with Transformers.js and cached on this machine. */
export type SpeechDevice = 'wasm' | 'webgpu'
export type ModelDtype = 'q8' | 'q4' | 'fp16' | 'fp32'

export interface SpeechModel {
  id: string
  repo: string
  name: string
  family: 'whisper' | 'moonshine'
  /** Approximate download in MB for the chosen precision. */
  sizeMb: number
  languages: 'multilingual' | 'english'
  /** 1–5 relative scores, used for the small meters in the catalog. */
  speed: number
  accuracy: number
  device: SpeechDevice
  dtype: { encoder_model: ModelDtype; decoder_model_merged: ModelDtype }
  summary: string
  recommended?: boolean
  requiresGpu?: boolean
}

export const speechModels: SpeechModel[] = [
  {
    id: 'moonshine-base', repo: 'onnx-community/moonshine-base-ONNX', name: 'Moonshine Base', family: 'moonshine',
    sizeMb: 63, languages: 'english', speed: 5, accuracy: 4, device: 'wasm',
    dtype: { encoder_model: 'q8', decoder_model_merged: 'q8' },
    summary: 'Fastest English dictation. Built for live speech on laptops.',
  },
  {
    id: 'whisper-tiny', repo: 'onnx-community/whisper-tiny', name: 'Whisper Tiny', family: 'whisper',
    sizeMb: 41, languages: 'multilingual', speed: 5, accuracy: 2, device: 'wasm',
    dtype: { encoder_model: 'q8', decoder_model_merged: 'q8' },
    summary: 'Lightest multilingual option. Good for English, rougher for Arabic.',
  },
  {
    id: 'whisper-base', repo: 'onnx-community/whisper-base', name: 'Whisper Base', family: 'whisper',
    sizeMb: 77, languages: 'multilingual', speed: 4, accuracy: 3, device: 'wasm',
    dtype: { encoder_model: 'q8', decoder_model_merged: 'q8' },
    summary: 'Balanced default for English, French, and Spanish on any machine.', recommended: true,
  },
  {
    id: 'whisper-small', repo: 'onnx-community/whisper-small', name: 'Whisper Small', family: 'whisper',
    sizeMb: 249, languages: 'multilingual', speed: 3, accuracy: 4, device: 'wasm',
    dtype: { encoder_model: 'q8', decoder_model_merged: 'q8' },
    summary: 'Clearly better French, Arabic, and Spanish. A few seconds per sentence.',
  },
  {
    id: 'whisper-large-v3-turbo', repo: 'onnx-community/whisper-large-v3-turbo', name: 'Whisper Large v3 Turbo', family: 'whisper',
    sizeMb: 760, languages: 'multilingual', speed: 3, accuracy: 5, device: 'webgpu',
    dtype: { encoder_model: 'q4', decoder_model_merged: 'q4' },
    summary: 'Best accuracy across languages, including Arabic. Uses your GPU.', requiresGpu: true,
  },
]

export const dictationLanguages: { value: string; label: string }[] = [
  { value: 'auto', label: 'Detect automatically' },
  { value: 'english', label: 'English' },
  { value: 'french', label: 'Français' },
  { value: 'arabic', label: 'العربية' },
  { value: 'spanish', label: 'Español' },
  { value: 'german', label: 'Deutsch' },
  { value: 'portuguese', label: 'Português' },
  { value: 'italian', label: 'Italiano' },
  { value: 'turkish', label: 'Türkçe' },
  { value: 'chinese', label: '中文' },
  { value: 'japanese', label: '日本語' },
  { value: 'hindi', label: 'हिन्दी' },
]

export const findSpeechModel = (id: string | null | undefined) => speechModels.find(model => model.id === id)
