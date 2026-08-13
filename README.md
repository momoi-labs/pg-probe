# pg-probe

`pg-probe` captures a time-bounded PostgreSQL and host diagnostic dataset into a
portable SQLite file.

> **Status:** early development. Collection domains continue to land in
> dependency-ordered slices. Linux release packaging for attested musl
> tarballs is available from SemVer tags.

## Requirements

- Linux (the collector reads kernel and libc interfaces directly).
- A recent stable Rust toolchain to build from source, or a published release
  binary for `x86_64-unknown-linux-musl` / `aarch64-unknown-linux-musl`.

## Install (release binaries)

Protected `vMAJOR.MINOR.PATCH` tags publish self-contained musl archives with
bundled libpq and vendored OpenSSL:

- `pg-probe-v<VERSION>-x86_64-unknown-linux-musl.tar.gz`
- `pg-probe-v<VERSION>-aarch64-unknown-linux-musl.tar.gz`
- `SHA256SUMS`

Each archive contains `pg-probe`, `README.md`, and `LICENSE`. After download:

```bash
tar -xzf pg-probe-v0.1.0-x86_64-unknown-linux-musl.tar.gz
sha256sum -c SHA256SUMS
gh attestation verify --repo momoi-labs/pg-probe \
  pg-probe-v0.1.0-x86_64-unknown-linux-musl.tar.gz
```

Builds are repeatable and provenance-verifiable through GitHub artifact
attestations. This project does not claim bit-for-bit reproducible Rust
binaries until independent rebuild-and-compare evidence exists.

## Build

```bash
cargo build --release
```

The executable is produced at `target/release/pg-probe`. Release automation
uses GoReleaser's Rust builder (`builder: rust` with `cross`) and pinned
toolchain/container inputs; see `.goreleaser.yaml`, `.github/workflows/release.yml`,
and `Cross.toml`.

## Usage

Capture a snapshot into a new compressed SQLite artifact in the current
directory (or an existing `--data-directory`):

```bash
pg-probe --domain all
```

Stdout prints only the final artifact path. The process never overwrites an
existing artifact; collisions get numeric suffixes. On success the artifact is
written atomically and the process exits `0`. If any required source fails, the
artifact is still published as a partial snapshot and the process exits
non-zero.

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
cargo fmt --all -- --check
cargo clippy --all-targets --all-features --locked -- -D warnings
cargo test --all-targets --locked
```

CI also cross-builds both musl release targets and exercises the packaging
scripts. Creating a protected `vMAJOR.MINOR.PATCH` tag whose version matches
`Cargo.toml` runs the attested release workflow.

## License

Licensed under the Apache License, Version 2.0.
