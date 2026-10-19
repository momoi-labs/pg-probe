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

`mise run build` first builds the review console in `web/` with pnpm, and
the binary embeds it, so `pg-probe analyze` needs nothing else at run time.
A binary built without `web/dist` still works and serves a page that says how
to build the console.

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

### Sampling over time

By default a capture takes one sample. Add `--duration` to keep sampling and
record how values change:

```bash
pg-probe capture --duration 5m --interval 10s
```

The first sample reads everything. Each later sample, one per `--interval`
(default `10s`, at least `1s`), reads only the values that change:

- Linux: CPU, memory, pressure, disk I/O, cgroup and network counters.
- PostgreSQL: the statistics views, the activity, lock and replication
  aggregates, and the WAL position.

Settings, database sizes, extensions, the largest relations and host identity
are read once. PostgreSQL sampling holds one connection for the whole session.
If a sample takes longer than the interval, the next tick is skipped instead
of queued. All samples go into one artifact, written when the duration ends.
Ctrl+C stops the session early and keeps the samples taken so far in the
`.sqlite.partial` file.

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
pg_activity. It works with PostgreSQL 9.3 and later and takes the same
connection options as `capture`:

```bash
pg-probe top -U postgres -h /var/run/postgresql
pg-probe top -h db.example.com -U monitor -d app -i 5
```

`-i`/`--interval` sets the refresh interval in seconds (default 2).

| Key | Action |
|---|---|
| `F1` or `1` | Running queries |
| `F2` or `2` | Queries waiting on a lock |
| `F3` or `3` | Queries that block others |
| `↑` `↓`, `PgUp` `PgDn` | Select a query |
| `v` or `Enter` | Open the query page |
| `Space` | Pause or resume the refresh |
| `q` | Quit |

The query page shows everything known about the selected backend: the full
query, client, application, connection, transaction, and query ages, and
`xid`/`xmin` on 9.4 and later. On the query page:

| Key | Action |
|---|---|
| `f` | Definition and call statistics of the functions the query calls, as `\df+` and `\sf` show them |
| `e` | Estimated plan of the query |
| `←` `→` | Other functions, when the query calls more than one |
| `v` or `Esc` | Back to the list |

Function statistics need `track_functions` set to `pl` or `all`. The plan
comes from `EXPLAIN` without `ANALYZE`, so it may differ from the plan the
backend runs. Queries cut at `track_activity_query_size` or with `$1`
parameters cannot be planned, and the popup says why.

When the server runs on the same host and you connect through a socket or
`localhost`, the view adds host load, memory, and swap, and per-backend CPU,
I/O, and I/O wait from `/proc`. Run it as root or as the `postgres` user to
read the I/O of every backend.

To see other users' query text, connect as a superuser or, on PostgreSQL 10
and later, as a member of `pg_read_all_stats`.

#### Cost and safety

- One connection, opened with `default_transaction_read_only=on` and a
  5-second `statement_timeout`. The session cannot write.
- One `SELECT` over `pg_stat_activity` per refresh. On a test server with 206
  connections under pgbench, it took 0.7 ms per call and 0.15% of one core.
- The blocking view, the query page, and the function and plan popups each add
  one query, only while open. The blocking view on 9.3 to 9.5 reads `pg_locks`
  once per refresh, because `pg_blocking_pids` arrived in 9.6.
- The function and plan popups query the database the backend runs in. When it
  differs from `-d`, they open a short read-only session there and close it.
- `EXPLAIN` goes over the extended query protocol, which accepts a single
  command, so query text such as `select 1; delete ...` cannot run its second
  part. `EXPLAIN ANALYZE` is never sent.
- Query text and function source appear on the terminal only. They are never
  written to disk.

## Artifact format

The artifact is a gzip-compressed SQLite database. Its schema version is stored
in the SQLite `user_version` pragma. The current schema contains:

- `snapshot` — one row of run-level envelope metadata (identifier, collector
  name and version, start/finish timestamps in UTC, and overall status).
- `sample` — one row per sample, with the UTC time it was taken. Values that
  change during a run, such as counters and gauges, are keyed by sample. A
  default run writes one sample. Values that do not change, such as host
  identity and settings, are recorded once per snapshot.
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
