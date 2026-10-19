import react from '@vitejs/plugin-react'
import { defineConfig } from 'vite'

// The binary serves the built console under /console/. In development, Vite
// proxies /api to a running `pg-probe analyze` (default port 3080, or the
// address in PG_PROBE_API).
export default defineConfig({
  base: '/console/',
  plugins: [react()],
  server: {
    proxy: {
      '/api': process.env.PG_PROBE_API ?? 'http://127.0.0.1:3080',
    },
  },
})
