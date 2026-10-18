import react from '@vitejs/plugin-react'
import { defineConfig } from 'vite'

// The binary serves the built console under /console/. In development, Vite
// proxies /api to a running `pg-probe analyze` (default port 3080).
export default defineConfig({
  base: '/console/',
  plugins: [react()],
  server: {
    proxy: {
      '/api': 'http://127.0.0.1:3080',
    },
  },
})
