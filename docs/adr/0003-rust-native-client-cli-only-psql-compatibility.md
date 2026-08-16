---
status: accepted
date: 2026-08-16
supersedes: []
superseded_by: null
tags: [postgresql, cli, packaging]
---

# psql compatibility is a CLI contract, not libpq re-implementation

Issue #11 (user story 5) promised that pg-probe would resolve connections the
way psql does, including service files, password files, and multi-host
strings; issue #8 assumed a bundled libpq. Commit `c174105` switched to the
Rust-native PostgreSQL client with rustls, which does not implement that
resolution. We decided to keep the Rust-native client and narrow the promise:
**pg-probe accepts the same command-line options as psql, and nothing more.**

The original wording overshot the intent. What was wanted was a familiar
command line, not a second implementation of libpq's resolution rules.

## Considered options

- **Return to libpq for full parity.** Rejected: it reintroduces a dynamic
  dependency on libpq and OpenSSL, which is what made the static musl build —
  the artifact that lets operators run pg-probe in restricted environments —
  awkward to produce.
- **Re-implement service files, password files, and multi-host resolution.**
  Rejected: significant surface area to match psql's behavior exactly, for a
  parity nobody asked for.
- **Keep the Rust-native client, match psql only at the CLI.** Chosen.

## Consequences

- Supported: psql's connection options (`-h`, `-p`, `-U`, `-d`), a
  `postgresql://` URL, a keyword/value string, and the `PG*` environment
  variables.
- Not supported: `pg_service.conf`, `.pgpass`, and multi-host connection
  strings. This must be stated in the README rather than left implicit.
- `-d` belongs to `--dbname`, following psql. The data directory takes `-D`.
- Static musl binaries with no libpq or OpenSSL dependency remain possible,
  which was the point.
