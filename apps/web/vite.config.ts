import { defineConfig } from 'vite'
import react from '@vitejs/plugin-react'
import tailwindcss from '@tailwindcss/vite'
import { fileURLToPath, URL } from 'node:url'

export default defineConfig({
  plugins: [react(), tailwindcss()],
  resolve: {
    alias: {
      '@': fileURLToPath(new URL('./src', import.meta.url)),
    },
  },
  server: {
    host: '127.0.0.1',
    port: 4173,
    proxy: {
      '/v1': 'http://127.0.0.1:7331',
      '/healthz': 'http://127.0.0.1:7331',
    },
  },
  build: {
    outDir: 'dist',
    sourcemap: true,
    manifest: true,
    rollupOptions: {
      output: {
        onlyExplicitManualChunks: true,
        manualChunks(id) {
          if (id.endsWith('/M7NavigationOverlays.tsx')) return 'm7-navigation-overlays'
          if (
            id.endsWith('/M7ApplicationShell.tsx') ||
            id.endsWith('/M7ActivityInbox.tsx') ||
            id.includes('/components/shadcn/') ||
            id.endsWith('/hooks/use-mobile.ts') ||
            id.endsWith('/lib/utils.ts') ||
            id.includes('/node_modules/@base-ui/react/') ||
            id.includes('/node_modules/cmdk/') ||
            id.includes('/node_modules/class-variance-authority/') ||
            id.includes('/node_modules/clsx/') ||
            id.includes('/node_modules/tailwind-merge/')
          ) {
            return 'm7-production-shell'
          }
        },
      },
    },
  },
})
