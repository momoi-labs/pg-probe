---
status: accepted
date: 2026-10-26
supersedes: []
superseded_by: null
tags: [release, cli, linux]
---

# macOS builds, where Linux collection is unsupported

pg-probe shipped for Linux only. The PostgreSQL engineer who reviews a
snapshot often works on a Mac, so they could not run the review console or
`pg-probe top` on their own machine.

## Decision

Each release also ships `aarch64-apple-darwin` and `x86_64-apple-darwin`
archives, and a Homebrew cask in `momoi-labs/homebrew-packages`, as self-host
does.

- The review console and the MCP server (ADR 0009) only read artifacts, so
  they run on macOS without limits.
- `capture` on macOS collects PostgreSQL. Each Linux collector reads nothing
  and records its source as `unsupported` with the code `not_linux`, so the
  artifact and the console say what was not collected. The capture warns once
  and points to `--domain postgresql`, which skips Linux and the warning.
- `pg-probe top` on macOS leaves out the host row and the per-backend CPU
  and I/O columns. On Linux, the row still says when the server is on another
  host.
- The archives build on Blacksmith's macOS runners, which are Apple Silicon
  only. The x86_64 archive is cross-built there and smoke-tested through
  Rosetta. They are not notarized; the cask removes the quarantine attribute.

## Considered options

- **Refuse `capture` on macOS.** Rejected: the PostgreSQL part works the same
  from any client, and the artifact already has a status for a source the
  host cannot provide.
- **Cross-compile from Linux with cargo-zigbuild.** Rejected: rustls'
  `aws-lc-sys` and the binary link Apple frameworks, which need the macOS SDK.
  A macOS runner also lets the release smoke-test the exact archive.
- **GitHub-hosted macOS runners.** Rejected: the organization runs CI on
  Blacksmith, and GitHub bills macOS minutes apart from it.
- **Notarize the binary.** Deferred: it needs a paid Apple Developer account,
  and the cask covers the common install.

## Consequences

- A macOS capture with the default domains is complete, not partial, since
  unsupported sources do not make a snapshot partial.
- A Blacksmith macOS minute costs as much as 20 Linux 2-vCPU minutes. CI runs
  one macOS job only when Rust code changes.
- Most integration tests read a Linux host, so CI on macOS runs the console
  tests and the release smoke test, not the whole suite.
- A capture against a remote server still records the local host's Linux
  data. That is out of scope here.
