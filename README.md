# pg-probe

`pg-probe` captures a time-bounded PostgreSQL and host diagnostic dataset into a
portable SQLite file.

> **Status:** early development. This first slice ships the snapshot spine: the
> CLI, the Linux host-identity collector, the typed snapshot contracts, and the
> single SQLite writer that publishes a compressed artifact. Compute, memory,
> storage, effective limits, PostgreSQL collection, and release packaging land
> in later slices and are intentionally not implemented yet.

## Requirements

- Linux (the collector reads kernel and libc interfaces directly).
- A recent stable Rust toolchain to build from source.

## Build

```bash
cargo build --release
```

The executable is produced at `target/release/pg-probe`.

## Usage

Capture a snapshot into a new compressed SQLite artifact:

```bash
pg-probe snapshot --output ./snapshot.sqlite.gz
```

The `--output` path must not already exist; `pg-probe` never overwrites an
existing artifact. On success the artifact is written atomically and the process
exits `0`. If any required source fails, the artifact is still published as a
partial snapshot and the process exits non-zero.

Inspect the artifact with standard tooling:

```bash
gunzip -k snapshot.sqlite.gz
sqlite3 snapshot.sqlite '.tables'
sqlite3 snapshot.sqlite 'SELECT status FROM snapshot;'
```

## Artifact format

The artifact is a gzip-compressed SQLite database. Its schema version is stored
in the SQLite `user_version` pragma. The current schema contains:

- `snapshot` — one row of run-level envelope metadata (identifier, collector
  name and version, start/finish timestamps in UTC, and overall status).
- `collector_outcome` — one row per collector component, recording its symbolic
  source, timing, status, and a stable machine-readable error code when it did
  not succeed.
- `linux_identity` — the collected host identity: kernel details, allowlisted
  `os-release` fields, clock parameters, and host uptime.

## Data boundary

The collector uses a strict positive allowlist. It reads only the enumerated
kernel, libc, and configuration sources, and it never persists environment
variables, command lines, credentials, network identifiers, or raw source input.
Errors carry only the symbolic source, a stable error code, and the OS error
number.

## Development

```bash
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets
```

These same checks run in CI on every pull request.

## License

Licensed under the Apache License, Version 2.0.
