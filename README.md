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

Protected `vMAJOR.MINOR.PATCH` tags publish self-contained static musl archives
that use the Rust-native PostgreSQL client and rustls:

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

To build a binary for a target server, use the static musl build. It runs on
any x86_64 Linux, including old glibc hosts such as CentOS 7:

```bash
mise install
mise run build
```

The executable is produced at
`target/x86_64-unknown-linux-musl/release/pg-probe`. A plain
`cargo build --release` links against the local glibc and is meant for
development; it writes to `target/release/pg-probe`. Release automation
uses GoReleaser's Rust builder with `cargo-zigbuild` and pinned toolchain
inputs; see `.goreleaser.yaml` and `.github/workflows/release.yml`.

## Usage

Capture a snapshot into a new compressed SQLite artifact in the current
directory (or an existing `-D`/`--data-directory`):

```bash
pg-probe --domain all
```

Connection options follow psql:

| Option | Meaning |
|---|---|
| `-h`, `--host` | PostgreSQL host or socket directory |
| `-p`, `--port` | PostgreSQL port |
| `-U`, `--username` | PostgreSQL user |
| `-d`, `--dbname` | database name, a `postgresql://` URL, or a keyword/value string |
| `-D`, `--data-directory` | directory where the artifact is written |

`-d` is the database name, matching psql; it no longer means the data
directory (a breaking change accepted pre-1.0). Precedence, highest first,
matches psql: a conninfo string or URI in `--dbname`, then the individual
connection options, then the `PG*` environment variables, then the client
defaults.

Without any connection options, pg-probe reads the ordinary connection
environment variables: `PGHOST`, `PGPORT`, `PGDATABASE`, `PGUSER`,
`PGPASSWORD`, `PGOPTIONS`, `PGAPPNAME`, and `PGCONNECT_TIMEOUT`. Service files
(`pg_service.conf`), passfiles (`.pgpass`), and multi-host connection strings
are intentionally unsupported.

All diagnostics go to stderr as structured log lines: the default keeps errors
and warnings, `--quiet` keeps only errors, and `--verbose` adds per-collector
progress. The published artifact path is reported as the `artifact` field of
the final "complete"/"partial snapshot published" line. The process never
overwrites an existing artifact; collisions get numeric suffixes. On success the
artifact is written atomically and the process exits `0`. If any required
source fails, the artifact is still published as a partial snapshot and the
process exits non-zero.

Inspect the artifact with standard tooling:

```bash
gunzip -k snapshot.sqlite.gz
sqlite3 snapshot.sqlite '.tables'
sqlite3 snapshot.sqlite 'SELECT status FROM snapshot;'
```

### Live view

`pg-probe top` shows the queries running now, longest first, in the style of
pg_activity. It takes the same connection options as `capture`:

```bash
pg-probe top -U postgres -h /var/run/postgresql
```

`F1`, `F2`, and `F3` switch between running, waiting, and blocking queries.
`v` opens the details of the selected backend, and `f` there shows the
definition and call statistics of the functions its query calls. `Space`
pauses, and `q` quits.
`-i` sets the refresh interval in seconds (default 2).

The session is read-only: it starts with `default_transaction_read_only=on`
and a 5-second `statement_timeout`. Each refresh sends one `SELECT` over
`pg_stat_activity`. The blocking view and the details panel add one query each,
and only while they are open. The function panel runs one catalog query when
opened, through a short session in the query's database when that differs. Query text appears on the terminal only and is
never written to disk.

When the server runs on the same host, the view adds load, memory, and
per-backend CPU and I/O from `/proc`. Reading another process's I/O counters
needs root or the `postgres` user.

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

There is no unsafe debug or raw-output mode: no flag, environment variable, or
build configuration bypasses the allowlist. Collector panics emit only a
generic notice; payloads, source locations, and backtraces are suppressed.

## Development

```bash
cargo fmt --all -- --check
cargo clippy --all-targets --all-features --locked -- -D warnings
cargo test --all-targets --locked
```

CI also builds both static musl release targets and exercises the packaging
scripts. Creating a protected `vMAJOR.MINOR.PATCH` tag whose version matches
`Cargo.toml` runs the attested release workflow.

## License

Licensed under the [PostgreSQL License](https://opensource.org/license/postgresql).
