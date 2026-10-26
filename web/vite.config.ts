import { readFileSync } from 'node:fs'
import react from '@vitejs/plugin-react'
import { defineConfig } from 'vite'

// The binary embeds the console from the same commit, so the version in
// Cargo.toml is the version of the binary that serves it.
const version = /^version = "(.+)"$/m.exec(readFileSync('../Cargo.toml', 'utf8'))![1]

// The binary serves the built console under /console/. In development, Vite
// proxies /api to a running `pg-probe analyze` (default port 3080, or the
// address in PG_PROBE_API).
export default defineConfig({
  base: '/console/',
  define: { __PG_PROBE_VERSION__: JSON.stringify(version) },
  plugins: [react()],
  server: {
    proxy: {
      '/api': process.env.PG_PROBE_API ?? 'http://127.0.0.1:3080',
    },
  },
})
