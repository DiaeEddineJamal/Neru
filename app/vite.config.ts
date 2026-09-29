import { fileURLToPath, URL } from 'node:url'
import react from '@vitejs/plugin-react'
import { defineConfig } from 'vite'
import tailwindcss from '@tailwindcss/vite'

// https://vite.dev/config/
export default defineConfig({
  plugins: [react(), tailwindcss()],
  // Pre-bundle the speech runtime so the dev server never reloads mid-download on first use.
  optimizeDeps: { include: ['@huggingface/transformers'] },
  preview: { headers: { 'Cross-Origin-Opener-Policy': 'same-origin', 'Cross-Origin-Embedder-Policy': 'credentialless' } },
  resolve: { alias: { '@': fileURLToPath(new URL('./src', import.meta.url)) } },
  // Cross-origin isolation lets the speech runtime use every CPU core (WASM threads).
  server: {
    headers: { 'Cross-Origin-Opener-Policy': 'same-origin', 'Cross-Origin-Embedder-Policy': 'credentialless' },
    port: 1420,
    strictPort: true,
    host: '127.0.0.1',
  },
})
