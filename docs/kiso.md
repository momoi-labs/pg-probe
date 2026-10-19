# Kiso in pg-probe

The pg-probe review console (`pg-probe analyze`) follows Kiso. This file
records only what Kiso leaves to the product. The rules live in the installed
contracts. If this file and a contract disagree, the contract wins and this
file has a bug.

## Contracts

- Package: `@momoi-labs/kiso-react` 0.17.0 and `@momoi-labs/kiso` 0.21.0 in
  `web/`.
- Contracts: `web/node_modules/@momoi-labs/kiso/kiso/`. Start at `AGENTS.md`.

## Paths

| What | Where |
| --- | --- |
| Screens | `web/src/screens/` |
| Product components | `web/src/screens/panels.tsx` |
| Product CSS | `web/src/app.css` |

## Product components

| Component | Use |
| --- | --- |
| `Panel` | A dashboard panel: Card with title, optional subtitle and badge, in a DashboardPanel span |
| `GapPanel` | A panel the artifact cannot draw yet. The badge names the missing work: `needs sampling`, `no collector`, `needs schema change` or `blocked` |
| `ChartPanel` | A time series panel: a Kiso `Chart` in a `Panel`, on the shared `session` sync id |
| `Note` | Muted text that says why a value is missing. Always visible |

## Product rules

- Appearance is set once on `<html>` in `web/index.html`: accent `gold`,
  chart style `rounded`, corner style `rounded`, corner size `medium`, no
  corner marks, default visual style and app shell, frame scope `panels`,
  background `dots`. `data-theme` stays off so the console follows the system
  theme.
- Show facts and derived values only. The screens carry no formulas: every
  derived value is documented in `docs/console-metrics.md`, which changes with
  the screen that shows the value. No thresholds, danger markers or health
  colors for now.
- A metric that the artifact cannot answer shows the unknown treatment and
  says why: not recorded, or it needs more than one sample.
- Charts use one point per sample, on the timestamps from `/samples`. Rates
  come from the change between samples. The first sample and any negative
  change, such as a counter reset, are gaps, never zero.
- Overview tiles summarize; the panels below explain. A tile shows the last
  sample, or the last interval for rates, one line of context and the session
  peak in text. It has no chart of its own: the whole tile links to the panel
  with the full chart, as Kiso allows for a Card that is one destination.

## Pattern choices

| Screen | Pattern |
| --- | --- |
| Shell (`screens/shell.tsx`) | Application shell, `layout="topbar"`. Metrics, Logs and Queries are Header navigation |
| Snapshot header (`screens/snapshot.tsx`) | Dashboard page framing: PageHeader with the artifact Select, and an Alert for sources that were not collected |
| Metrics (`screens/metrics.tsx`, `system.tsx`, `postgres.tsx`) | Dashboard, one Disclosure per section |
| Queries (`screens/queries.tsx`) | Dashboard |
| Logs (`screens/not-collected.tsx`) | Empty states, informational |
| No snapshots, list error (`screens/start.tsx`) | Empty states, first run; Errors |

## Terminal view

`pg-probe top` is a terminal UI, so it cannot load Kiso's CSS. It takes the
dark theme values with the gold accent from `tokens/build/tokens.css` and
keeps them as RGB in `src/top/palette.rs`. Kiso keeps terminals dark in both
themes, so `top` paints its own `background` and uses the `*-on-dark` status
values. A terminal that does not set `COLORTERM` to `truecolor` or
`24bit` gets the nearest of the 256 xterm colors, since an older tmux misreads
RGB codes. The product rules above apply here too: no health colors, `—` for an
unknown value, IEC units, and numeric columns aligned right. Change the
palette with the package version, and record the version in the module doc.

## Exceptions

None.

## Verify

1. Install: `pnpm install` in `web/`.
2. Typecheck and build: `pnpm build` in `web/`.
3. Load the built console: run `pnpm build`, rebuild pg-probe, then run
   `pg-probe analyze --data-directory web/dev-artifacts` and open
   `http://localhost:3080/`. The binary embeds `web/dist` at compile time.
4. Develop with hot reload: run `pg-probe analyze --data-directory web/dev-artifacts`, then
   `pnpm dev` in `web/` and open `http://localhost:5173/console/`. Vite
   proxies `/api` to port 3080. Set `PG_PROBE_API` to use another address.
5. Check a complete artifact, a partial one (`partial-failures`) and a real
   capture, in both themes and at 390px.
