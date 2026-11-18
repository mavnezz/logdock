import { fileURLToPath, URL } from 'node:url'

import tailwindcss from '@tailwindcss/vite'
import vue from '@vitejs/plugin-vue'
import { defineConfig } from 'vitest/config'

export default defineConfig({
  plugins: [vue(), tailwindcss()],
  resolve: {
    alias: { '@': fileURLToPath(new URL('./src', import.meta.url)) },
  },
  server: {
    port: 5173,
    strictPort: true,
    // `just dev` serves the API here; the browser sees one origin, as behind the proxy.
    proxy: {
      '/api': { target: 'http://127.0.0.1:8080' },
      '/health': { target: 'http://127.0.0.1:8080' },
    },
  },
  test: {
    environment: 'jsdom',
    include: ['src/**/*.test.ts'],
    // `npm run test:coverage` (just coverage-web); CI holds the thresholds.
    coverage: {
      provider: 'v8',
      include: ['src/**/*.{ts,vue}'],
      exclude: ['src/**/*.test.ts', 'src/**/*.d.ts', 'src/main.ts', 'src/shared/testing/**'],
      reporter: ['text-summary', 'html', 'json-summary'],
      reportsDirectory: '../dev/coverage/frontend',
      thresholds: {
        lines: 85,
        statements: 85,
        functions: 78,
        branches: 82,
      },
    },
  },
})
