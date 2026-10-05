import { cpSync, mkdirSync } from 'node:fs'
import { fileURLToPath } from 'node:url'
const target = fileURLToPath(new URL('../public/mediapipe/', import.meta.url))
mkdirSync(target, { recursive: true })
cpSync(fileURLToPath(new URL('../node_modules/@mediapipe/tasks-vision/wasm/', import.meta.url)), target, { recursive: true })
