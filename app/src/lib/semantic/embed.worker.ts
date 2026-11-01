/// <reference lib="webworker" />
import { env, pipeline, type FeatureExtractionPipeline } from '@huggingface/transformers'
// The ONNX runtime ships with the app instead of loading from a CDN (the desktop CSP only allows 'self').
import ortWasm from '../../../node_modules/onnxruntime-web/dist/ort-wasm-simd-threaded.asyncify.wasm?url'
import ortModule from '../../../node_modules/onnxruntime-web/dist/ort-wasm-simd-threaded.asyncify.mjs?url'
import { EMBEDDING_MODEL } from './model'

export type EmbedRequest = { type: 'embed'; id: number; texts: string[] }
export type EmbedResponse =
  | { type: 'result'; id: number; vectors: number[][] }
  | { type: 'error'; id: number; message: string }

env.allowLocalModels = false
env.useBrowserCache = true
const onnx = env.backends.onnx
if (onnx.wasm) onnx.wasm.wasmPaths = { wasm: new URL(ortWasm, self.location.href).href, mjs: new URL(ortModule, self.location.href).href }
// Leave cores for the interface and the speech engine.
if (onnx.wasm && self.crossOriginIsolated) onnx.wasm.numThreads = Math.max(1, Math.min(4, (navigator.hardwareConcurrency || 4) - 2))

const post = (message: EmbedResponse) => (self as unknown as DedicatedWorkerGlobalScope).postMessage(message)

let extractor: Promise<FeatureExtractionPipeline> | null = null

function load(): Promise<FeatureExtractionPipeline> {
  extractor ??= (async () => {
    const gpu = 'gpu' in navigator && Boolean(await (navigator as Navigator & { gpu?: { requestAdapter: () => Promise<unknown> } }).gpu?.requestAdapter())
    return await pipeline('feature-extraction', EMBEDDING_MODEL, { device: gpu ? 'webgpu' : 'wasm', dtype: gpu ? 'fp16' : 'q8' }) as FeatureExtractionPipeline
  })()
  extractor.catch(() => { extractor = null })
  return extractor
}

self.onmessage = async (event: MessageEvent<EmbedRequest>) => {
  const { id, texts } = event.data
  try {
    const model = await load()
    const output = await model(texts, { pooling: 'mean', normalize: true })
    post({ type: 'result', id, vectors: output.tolist() as number[][] })
  } catch (cause) {
    post({ type: 'error', id, message: cause instanceof Error ? cause.message : String(cause) })
  }
}
