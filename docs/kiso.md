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
| Screens | `web/src/` |
| Product components | None yet |
| Product CSS | `web/src/app.css` |

## Product components

None.

## Product rules

None.

## Pattern choices

| Screen | Pattern |
| --- | --- |
| Placeholder (`web/src/App.tsx`) | Empty states, informational |

## Exceptions

None.

## Verify

1. Install: `pnpm install` in `web/`.
2. Typecheck and build: `pnpm build` in `web/`.
3. Load the console: run `pg-probe analyze --data-directory web/dev-artifacts`, then
   `pnpm dev` in `web/` and open `http://localhost:5173/console/`. Vite
   proxies `/api` to port 3080.
