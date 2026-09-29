/// <reference lib="webworker" />
import { env, pipeline, Tensor, type AutomaticSpeechRecognitionPipeline, type ProgressInfo } from '@huggingface/transformers'
// The ONNX runtime ships with the app instead of loading from a CDN (the desktop CSP only allows 'self').
import ortWasm from '../../../node_modules/onnxruntime-web/dist/ort-wasm-simd-threaded.asyncify.wasm?url'
import ortModule from '../../../node_modules/onnxruntime-web/dist/ort-wasm-simd-threaded.asyncify.mjs?url'
import type { SpeechModel } from './catalog'

export type WorkerRequest =
  | { type: 'load'; model: SpeechModel }
  | { type: 'transcribe'; id: number; model: SpeechModel; audio: Float32Array; language: string }

export type WorkerResponse =
  | { type: 'progress'; modelId: string; loaded: number; total: number; progress: number }
  | { type: 'ready'; modelId: string; device: string }
  | { type: 'result'; id: number; text: string; seconds: number; language?: string }
  | { type: 'error'; id?: number; modelId?: string; message: string }

env.allowLocalModels = false
env.useBrowserCache = true
const onnx = env.backends.onnx
if (onnx.wasm) onnx.wasm.wasmPaths = { wasm: new URL(ortWasm, self.location.href).href, mjs: new URL(ortModule, self.location.href).href }
// With cross-origin isolation the runtime can use threads; leave one core for the interface.
if (onnx.wasm && self.crossOriginIsolated) onnx.wasm.numThreads = Math.max(1, Math.min(8, (navigator.hardwareConcurrency || 4) - 1))

const post = (message: WorkerResponse) => (self as unknown as DedicatedWorkerGlobalScope).postMessage(message)

interface Loaded { id: string; transcriber: AutomaticSpeechRecognitionPipeline; device: string }
let loaded: Loaded | null = null
let loading: Promise<Loaded> | null = null
let loadingId = ''

async function load(model: SpeechModel): Promise<Loaded> {
  if (loaded?.id === model.id) return loaded
  if (loading && loadingId === model.id) return loading
  loadingId = model.id
  loading = (async (): Promise<Loaded> => {
    await loaded?.transcriber.dispose()
    loaded = null
    const gpu = model.device === 'webgpu' && 'gpu' in navigator && Boolean(await (navigator as Navigator & { gpu?: { requestAdapter: () => Promise<unknown> } }).gpu?.requestAdapter())
    if (model.requiresGpu && !gpu) throw new Error(`${model.name} needs a GPU with WebGPU support. Choose Whisper Small or Base instead.`)
    const device = gpu ? 'webgpu' : 'wasm'
    const onProgress = (info: ProgressInfo) => {
      if (info.status === 'progress_total') post({ type: 'progress', modelId: model.id, loaded: info.loaded, total: info.total, progress: info.progress })
    }
    const transcriber = await pipeline('automatic-speech-recognition', model.repo, { device, dtype: model.dtype, progress_callback: onProgress }) as AutomaticSpeechRecognitionPipeline
    const ready: Loaded = { id: model.id, transcriber, device }
    loaded = ready
    post({ type: 'ready', modelId: model.id, device })
    return ready
  })()
  try { return await loading } finally { loading = null }
}

interface WhisperInternals {
  model: ((inputs: Record<string, unknown>) => Promise<{ logits: { data: Float32Array } }>) & { generation_config?: { lang_to_id?: Record<string, number>; decoder_start_token_id?: number } }
  processor: (audio: Float32Array) => Promise<{ input_features: unknown }>
}

/**
 * Transformers.js has no Whisper language detection and silently falls back to English
 * (which turns French speech into an English translation). Whisper predicts the spoken
 * language right after its start token, so one decoder step over the language tokens gives it.
 */
async function detectLanguage(transcriber: AutomaticSpeechRecognitionPipeline, audio: Float32Array): Promise<string> {
  const { model, processor } = transcriber as unknown as WhisperInternals
  const config = model.generation_config
  if (!config?.lang_to_id || config.decoder_start_token_id === undefined) return 'en'
  const { input_features } = await processor(audio.subarray(0, 30 * 16_000))
  const decoder_input_ids = new Tensor('int64', BigInt64Array.from([BigInt(config.decoder_start_token_id)]), [1, 1])
  const { logits } = await model({ input_features, decoder_input_ids })
  let best = 'en'
  let score = Number.NEGATIVE_INFINITY
  for (const [token, id] of Object.entries(config.lang_to_id)) {
    if (logits.data[id] > score) { score = logits.data[id]; best = token.slice(2, -2) }
  }
  return best
}

self.onmessage = async (event: MessageEvent<WorkerRequest>) => {
  const request = event.data
  try {
    if (request.type === 'load') { await load(request.model); return }
    const started = performance.now()
    const { transcriber } = await load(request.model)
    const options: Record<string, unknown> = {}
    if (request.model.family === 'whisper') {
      options.task = 'transcribe'
      options.chunk_length_s = 30
      options.stride_length_s = 5
      options.language = request.language !== 'auto' ? request.language : await detectLanguage(transcriber, request.audio)
    }
    const output = await transcriber(request.audio, options)
    const text = (Array.isArray(output) ? output.map(item => item.text).join(' ') : output.text).trim()
    post({ type: 'result', id: request.id, text, seconds: (performance.now() - started) / 1000, language: options.language as string | undefined })
  } catch (cause) {
    const message = cause instanceof Error ? cause.message : String(cause)
    post(request.type === 'transcribe' ? { type: 'error', id: request.id, message } : { type: 'error', modelId: request.model.id, message })
  }
}
