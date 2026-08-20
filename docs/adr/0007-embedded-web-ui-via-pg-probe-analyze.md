# ADR 0007: Embedded web UI via pg-probe analyze

Date: 2025-09-29

## Status

Accepted

## Context

pg-probe is a single Rust CLI crate that captures portable PostgreSQL and Linux
diagnostic snapshots as .sqlite.gz artifacts. The team needs a way to review
the collected data — browse the ~37 tables, see collection outcomes (successes
and failures), inspect PostgreSQL settings (including dangerous ones like
`fsync = off`), and understand partial snapshots.

The Momoi Labs design system, Kiso, is a web design system (semantic CSS tokens,
Radix/shadcn-style component contracts, React). Using Kiso means the UI is web.

Two structural options emerged during grilling:

1. A separate Node.js web app that reads .sqlite.gz files and serves the UI.
2. An embedded web server inside the pg-probe binary itself, serving pre-built
   static UI assets plus a JSON API over the SQLite data.

The user's hard constraint: the pg-probe binary must remain self-contained. A
separate Node runtime does not add value for an operator who already has the
binary; it adds installation and maintenance surface.

## Decision

pg-probe will gain a subcommand `pg-probe analyze` that starts an embedded HTTP
server (axum) serving:

- `/console/*` — the pre-built static React/Kiso web application (assets
  embedded in the binary via `include_dir!` or equivalent).
- `/api/*` — a JSON API backed by `rusqlite` reading `.sqlite.gz` artifacts from
  a data directory.

The CLI structure will change from direct flags to a subcommand model:

- `pg-probe capture` — preserves the current collection behavior (the flags
  `--data-directory`, `--domain`, `--database`, `--quiet`, `--verbose` move
  under this subcommand).
- `pg-probe analyze [--data-directory <dir>] [--port <port>]` — starts the
  review server.

Invoking bare `pg-probe` is a usage error that prints the top-level help. A
subcommand is required so collection never starts implicitly.

The web frontend lives in `web/` within the repository as TypeScript/React
source. During development, a Vite dev server proxies `/api/*` to a running
`pg-probe analyze` instance. For release builds, `pnpm build` produces static
assets that are embedded into the Rust binary, so the single executable serves
the UI with no external dependencies.

## Consequences

- The binary gains `axum`, `tower`, and `include_dir` (or similar) as
  dependencies. The web assets add to binary size but keep the single-executable
  deployment model.
- The CLI migrates to required clap subcommands. Existing scripts must add the
  explicit `capture` subcommand.
- The `web/` directory introduces a Node.js toolchain (Vite, TypeScript,
  React) for development only — never for runtime. The binary is the sole
  runtime artifact.
- The seed for test data is a .sql file of INSERT statements plus a shell
  script that applies the shared `schema.sql` and produces `.sqlite.gz` files.
  This avoids compiling Rust to generate test data and keeps the seed
  human-editable.
- The schema SQL (`SCHEMA_SQL` in `artifact.rs`) is extracted to versioned
  Refinery migrations embedded in the Rust binary. The seed shell script
  applies the same initial migration directly, keeping the migration SQL as
  the single source of truth.
