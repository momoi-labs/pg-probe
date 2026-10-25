# pg-probe

## 0.3.0

### Minor Changes

- da4821c: The printed report opens on a cover: the server, the capture period, how
  long it ran, how many sources came in, and the facts of the host,
  PostgreSQL and the capture. When a source did not collect in full, a Data
  coverage page follows, with failed sources first and the ones the host or
  server does not have grouped by reason. Every page after the cover has a
  header with the server and period, and a footer with the page number.

### Patch Changes

- da4821c: On cgroup v1 hosts, such as CentOS 7, the capture now records the swap the
  cgroup uses, and records CPU usage in microseconds instead of nanoseconds.
  The Page reclaim chart shows direct reclaim on kernel 4.8 and newer, and
  the Cgroup events chart shows the limit counters its explanation describes.
- 65f1422: The review console shows the pg-probe emblem in its header and as the browser
  tab's icon.
- f726a7d: The pg-probe emblem in the review console's header no longer shows a database.
  It is now the elephant and the probe waves, as in the browser tab's icon.
- da4821c: The printed report no longer leaves a section title alone on a page in
  Portuguese or Spanish, and no longer prints a blank page after a full
  section. Charts and gauges print without the dot texture that some PDF
  viewers drew as a grid. The Portuguese and Spanish translations were
  reviewed, and the "What to look for" texts now say which PostgreSQL
  version or kernel a setting or view needs.

## 0.2.2

### Patch Changes

- c0b490f: The release checks artifacts made in Rocky Linux 9 with the host's sqlite3,
  since Rocky's is too old for their STRICT tables. v0.2.1 was tagged but not
  published because that check failed; this release ships the same changes.

## 0.2.1

### Patch Changes

- b973367: Release builds are smoke-tested with `pg-probe capture`. v0.2.0 was tagged
  but not published, because the release smoke test still called the old
  command line; this release ships the same changes.

## 0.2.0

### Minor Changes

- e34f667: `pg-probe capture --cleanup` deletes the `.sqlite.partial` files left by
  earlier captures. It lists them and asks to confirm: `y` deletes them and the
  capture continues; any other answer aborts with exit code 2 and deletes
  nothing.
- 5d98798: `pg-probe capture` shows its progress by default. On a terminal, a summary
  redraws in place: samples taken, collector counts per domain, sample time p95
  and p99, pg-probe's own CPU, peak memory and disk writes, and the errors and
  unsupported sources so far. `--refresh` sets how often it redraws, half the
  `--interval` by default and at least 1s. Off a terminal, it appends a line only
  when something changes, plus one progress line per minute. `--verbose` still
  prints one line per collector outcome.
- 0c5157b: The review console shows more host and PostgreSQL metrics: swap, page
  faults, reclaim, network drops, disk latency, cgroup usage, locks, wait
  events, conflicts, archiving, replication slots, SLRU and `pg_stat_io`.
  A metric the server or host cannot provide says "Not supported on
  PostgreSQL 9.3" or "on this host" and why. Panels can be moved and resized,
  and the layout is kept per section. Chart legends show p50, p75, p95, p99
  and the last sample. Hover marks the same time on every chart, and a click
  pins it.
- d4e68f6: The review console prints a report of the artifact. "Print report" in the
  header opens it: a cover, a linked contents page, then one chart or table
  per page with a note on what it shows and what to look for.
- 3204db2: The review console prints the report in English, Brazilian Portuguese or Spanish. "Print report" asks for the language.
- 813a39e: A time range in the review console header limits every chart and its
  legend to part of the capture. A drag across any chart sets it, and
  "Entire run" restores the whole capture.
- 7ccbeef: `pg-probe top` and the capture progress screen follow Kiso's terminal look.
  Both set the terminal's own background and text color while they run and
  put them back when they end. Sections are a title and a rule, the selected
  row has a gold bar, the views are tabs with their session counts, query
  details open as a page, and help, functions and explain open as dialogs. On
  a narrow screen the query moves under the figures of its row. Keys and saved
  settings stay the same.

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
