import type { SpeechModel } from './catalog'
import type { WorkerRequest, WorkerResponse } from './speech.worker'

const CACHE_NAME = 'transformers-cache'
const SAMPLE_RATE = 16_000

export interface DownloadProgress { loaded: number; total: number; progress: number }

let worker: Worker | null = null
let nextId = 1
const listeners = new Set<(message: WorkerResponse) => void>()

function getWorker() {
  if (!worker) {
    worker = new Worker(new URL('./speech.worker.ts', import.meta.url), { type: 'module' })
    worker.onmessage = (event: MessageEvent<WorkerResponse>) => listeners.forEach(listener => listener(event.data))
    worker.onerror = event => listeners.forEach(listener => listener({ type: 'error', message: event.message || 'The speech engine failed to start' }))
  }
  return worker
}

const send = (request: WorkerRequest) => getWorker().postMessage(request, request.type === 'transcribe' ? [request.audio.buffer] : [])

function listen(handler: (message: WorkerResponse) => boolean | void) {
  const listener = (message: WorkerResponse) => { if (handler(message) === true) listeners.delete(listener) }
  listeners.add(listener)
  return () => listeners.delete(listener)
}

/** Downloads (or loads from cache) a model, reporting aggregate byte progress. */
export function downloadModel(model: SpeechModel, onProgress: (progress: DownloadProgress) => void): Promise<string> {
  return new Promise((resolve, reject) => {
    listen(message => {
      if (message.type === 'progress' && message.modelId === model.id) onProgress(message)
      if (message.type === 'ready' && message.modelId === model.id) { resolve(message.device); return true }
      if (message.type === 'error' && (message.modelId === model.id || (message.modelId === undefined && message.id === undefined))) { reject(new Error(message.message)); return true }
    })
    send({ type: 'load', model })
  })
}

/** Decodes any recorded clip to 16 kHz mono, the format speech models expect. */
export async function decodeAudio(audio: Blob): Promise<Float32Array> {
  const context = new AudioContext({ sampleRate: SAMPLE_RATE })
  try {
    const buffer = await context.decodeAudioData(await audio.arrayBuffer())
    if (buffer.numberOfChannels === 1) return buffer.getChannelData(0).slice()
    const mono = new Float32Array(buffer.length)
    for (let channel = 0; channel < buffer.numberOfChannels; channel += 1) {
      const data = buffer.getChannelData(channel)
      for (let index = 0; index < data.length; index += 1) mono[index] += data[index] / buffer.numberOfChannels
    }
    return mono
  } finally { void context.close() }
}

export async function transcribeLocally(audio: Blob | Float32Array, model: SpeechModel, language: string): Promise<{ text: string; seconds: number; language?: string }> {
  const samples = audio instanceof Float32Array ? audio : await decodeAudio(audio)
  if (samples.length < SAMPLE_RATE * 0.3) throw new Error('The recording was too short to transcribe')
  const id = nextId++
  return new Promise((resolve, reject) => {
    listen(message => {
      if (message.type === 'result' && message.id === id) { resolve({ text: message.text, seconds: message.seconds, language: message.language }); return true }
      if (message.type === 'error' && message.id === id) { reject(new Error(message.message)); return true }
    })
    send({ type: 'transcribe', id, model, audio: samples, language })
  })
}

async function cachedUrls(model: SpeechModel) {
  if (typeof caches === 'undefined') return []
  const cache = await caches.open(CACHE_NAME)
  return (await cache.keys()).filter(request => request.url.includes(`/${model.repo}/`))
}

/** True when the model's weights are already in the local cache. */
export async function isModelCached(model: SpeechModel) {
  const urls = await cachedUrls(model)
  return urls.some(request => /encoder_model/.test(request.url)) && urls.some(request => /decoder_model_merged/.test(request.url))
}

export async function removeModel(model: SpeechModel) {
  const cache = await caches.open(CACHE_NAME)
  await Promise.all((await cachedUrls(model)).map(request => cache.delete(request)))
  if (worker) { worker.terminate(); worker = null }
}

export async function storageUsage() {
  const estimate = await navigator.storage?.estimate?.()
  return { usedMb: Math.round((estimate?.usage ?? 0) / 1e6), quotaMb: Math.round((estimate?.quota ?? 0) / 1e6) }
}
