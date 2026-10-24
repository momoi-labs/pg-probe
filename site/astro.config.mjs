import { defineConfig } from 'astro/config';
import sitemap from '@astrojs/sitemap';

export default defineConfig({
  // Cloudflare Workers (assets only, see wrangler.jsonc) serves ./dist as a
  // plain static site, so no adapter is needed.
  site: 'https://pg-probe.dev',
  integrations: [sitemap()],
});
