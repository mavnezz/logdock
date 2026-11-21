import { fileURLToPath, URL } from 'node:url'

import tailwindcss from '@tailwindcss/vite'
import vue from '@vitejs/plugin-vue'
import { defineConfig } from 'vitest/config'

export default defineConfig({
  plugins: [vue(), tailwindcss()],
  // Fonts stay files: the Content Security Policy loads no data: fonts.
  build: { assetsInlineLimit: (file) => (file.endsWith('.woff2') ? false : undefined) },
  resolve: {
    alias: { '@': fileURLToPath(new URL('./src', import.meta.url)) },
  },
  server: {
    port: 5173,
    strictPort: true,
    // `just dev` serves the API here; the browser sees one origin, as behind the proxy.
    // LOGDOCK_API points it elsewhere, such as the dev stack (http://127.0.0.1:18480).
    proxy: {
      '/api': { target: process.env.LOGDOCK_API ?? 'http://127.0.0.1:8080' },
      '/health': { target: process.env.LOGDOCK_API ?? 'http://127.0.0.1:8080' },
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
