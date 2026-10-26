# pg-probe site

The marketing site for pg-probe, at https://pg-probe.dev. It is a static Astro
site, served by Cloudflare Workers as static assets.

```sh
pnpm install
pnpm dev          # http://localhost:4321
pnpm build        # -> dist/
pnpm check        # astro check, 0 errors expected
pnpm preview      # serve dist/
```

## Languages

The site is built once per language: English at `/`, Portuguese at `/pt-br/`,
and Spanish at `/es/`. The text lives in `src/i18n/`; `en.ts` is the source,
and its type makes the other files carry every key. `src/components/Home.astro`
is the page, and each route in `src/pages/` renders it in one language.

A visitor's choice in the language menu is saved in `localStorage` and wins
on later visits. Without one, `/` follows the browser's language, and a link
to `/pt-br/` or `/es/` opens in that language. The menu works as pg-yard's.
The report images in the page are printed in the page's language.

## Screenshots

Every image in `src/assets/shots/` is a real pg-probe screen. `mise run demo`
in the repository root produces them in `target/demo/screenshots/`. The report
pages come from the PDFs there:

```sh
pdftoppm -png -r 110 -f 1 -l 1 -singlefile report-en.pdf report-page-cover-en
```

`public/demo/report/` holds the same PDFs as the sample report linked from
the report section, one per language.

The `pg-probe top` screens in `src/assets/top/` are real captures in a
terminal of 120 columns and 35 rows. `scripts/top-screens.sh` in the
repository root stages a workload in Docker, runs `top` in tmux, and writes
them as HTML:

```sh
cargo build --release --locked
scripts/top-screens.sh
```

The hero is drawn in HTML from one real 3 minute capture of `db-prod-01`:
`src/components/TermCapture.astro` holds the last frame of `pg-probe capture`,
and `public/demo/console/` is a static copy of the page that `pg-probe
analyze` serves for the same file, with the console's own CSS. Queries and the
sections below the overview are removed from the copy.

## Install script

`public/install.sh` is the script behind `curl -fsSL
https://pg-probe.dev/install.sh | sh`. It downloads releases from the R2
bucket, not from GitHub. See `docs/releases.md` in the repository root.

## Deploy

A Cloudflare Workers project named `pg-probe-site`, connected to this
repository, builds and deploys the site. `wrangler.jsonc` declares the worker
name and the assets directory, so do not delete it: without it, `wrangler
deploy` runs a setup wizard that installs the Cloudflare adapter and switches
the build to server output.

| Setting        | Value                  |
|----------------|------------------------|
| Root directory | `site`                 |
| Build command  | `pnpm build`           |
| Deploy command | `npx wrangler deploy`  |

Add `pg-probe.dev` and `www.pg-probe.dev` under Settings, Domains & Routes,
and redirect `www` to the apex with a zone Redirect Rule.
