---
status: accepted
date: 2026-10-26
supersedes: []
superseded_by: null
tags: [review, mcp, cli]
---

# A local MCP server lets AI agents query artifacts

Today a person reviews an artifact by clicking through the console, reading
the panels and asking for help elsewhere. Many people already work with an AI
coding agent such as Claude Code, Codex or Kimi. These agents speak the Model
Context Protocol (MCP), so pg-probe can give them direct, read-only access to
the artifacts. The person then asks questions in the agent, and the agent
writes the queries.

## Decision

`pg-probe mcp [--data-directory <dir>]` starts an MCP server over stdio. It is
part of the same binary, needs nothing else installed, and reads the same
`.sqlite.gz` artifacts as `pg-probe analyze`. It never connects to PostgreSQL
and never writes to an artifact.

It offers four tools:

- `list_snapshots` lists the artifacts in the data directory.
- `describe_schema` returns every table and column with its type, unit, what
  it measures and where pg-probe reads it. The descriptions live in the
  binary, in `schema/catalog.md`, so they also describe older artifacts. A
  test fails when a table or column in the migrations has no description.
- `query` runs one read-only SQL statement on one artifact and returns the
  rows. The agent filters and aggregates. pg-probe computes nothing for it.
- `export_query` runs the same kind of statement and writes every row to a new
  CSV file, for analysis the agent wants to do outside SQLite. It never
  overwrites a file.

`--data-directory` sets the default directory. `list_snapshots`, `query` and
`export_query` also take an absolute `data_directory`, so the person can point
the agent at another directory during a session. The server keeps no state
between calls, so each call shows where it read.

`query` cuts its result at a row limit and says so, so one `SELECT *` cannot
flood the agent's context. Both tools stop a statement that runs past a time
limit. The connection is read-only, allows no `ATTACH`, and loads no
extensions.

The bundled SQLite is built with `SQLITE_ENABLE_PERCENTILE` and
`SQLITE_ENABLE_MATH_FUNCTIONS`, so agents can compute p95 and p99 in SQL.

## Considered options

- **A hosted MCP server on pg-probe.dev.** It needs uploads, accounts and
  retention, and moves the artifact off the person's machine. Rejected.
- **Curated tools per console panel.** The series and percentiles live in the
  console's TypeScript. Porting them to Rust keeps two sources of truth.
  Rejected for now; free SQL plus good descriptions covers the same questions.
- **A catalog table inside the artifact** (#32). It would describe the
  artifact to any SQLite reader, but older artifacts would lack it. The
  binary's catalog can be copied into the artifact later.

## Consequences

- The binary gains `rmcp`, the official Rust MCP SDK, on the existing tokio
  runtime.
- Whatever the agent reads goes to the model the person chose. The artifact
  already holds no credentials or query text (`CONTEXT.md`), and this ADR does
  not change that. Query text stays out of the artifact; if it ever enters, it
  is opt-in.
- "Agent" already names pg-probe's OTLP export mode (#62). Code and docs call
  this feature the MCP server, and say "AI agent" for the client.
