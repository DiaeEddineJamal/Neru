import { fileURLToPath, URL } from 'node:url'
import { defineConfig } from 'vitest/config'

// Unit and component tests for the window (src/**/*.test.ts[x]). Rust has its own `cargo test`.
export default defineConfig({
  // Vitest bundles its own Vite, so JSX is compiled here rather than through the app's React plugin.
  esbuild: { jsx: 'automatic' },
  resolve: { alias: { '@': fileURLToPath(new URL('./src', import.meta.url)) } },
  test: { environment: 'jsdom', include: ['src/**/*.test.{ts,tsx}'], setupFiles: ['src/test/setup.ts'], css: false },
})
