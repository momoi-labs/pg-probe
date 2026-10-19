# pg-probe

## 0.1.0

### Minor Changes

- 73c376a: First release. `pg-probe` captures a time-bounded PostgreSQL and Linux host
  diagnostic dataset into one portable SQLite file, from PostgreSQL 9.3 on.
  
  - `pg-probe capture` takes one snapshot, or samples over time with
    `--duration` and `--interval`. It reads settings, statistics views,
    activity, locks, replication, WAL, the largest relations, and host CPU,
    memory, pressure, disks, cgroups and network. Connection options follow
    psql.
  - `pg-probe analyze` serves a review console for captured artifacts in the
    browser, with charts for every sample.
  - `pg-probe top` is a live view of the queries running now: running,
    waiting, blocking and vacuum views, query details with the formatted
    query, function definitions and the estimated plan, and per-backend CPU
    and I/O when the server runs on the same host. Display options persist
    between runs.
  - Each release ships static musl tarballs and `.deb` and `.rpm` packages for
    x86_64 and arm64. The binary links no libc, so the packages have no
    dependencies and install on old and new distributions alike.
