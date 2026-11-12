# pg-probe

## 0.6.0

### Minor Changes

- be36fee: A capture with `--duration` now records the wraparound state over time. Each
  sample has the size of the multixact member space, the databases ranked by
  oldest xid and oldest multixact, and the vacuums in progress. The tables with
  the oldest xid and the oldest multixact, in the oldest databases you can
  connect to, are read on the first sample and then at most once every 60
  seconds, with their `autovacuum_count`. A table that shows a wraparound
  autovacuum again and again while its count stays the same has autovacuums that
  do not finish. This works from PostgreSQL 9.3, where the multixact order comes
  from a rank because there is no multixact age. Reading the member space needs
  a superuser, or from 11 EXECUTE on `pg_ls_dir`. The artifact schema is now
  version 4, and it also records the four freeze age settings.
- cee6197: The console and the printed report have a Wraparound section. "Vacuum tries
  by table" counts the vacuums seen on each table during the capture, how many
  ran to prevent wraparound and how many finished, with a line that shows the
  samples in which a vacuum ran. A table with many tries and none finished has
  an autovacuum that starts and stops before the end. "Multixact member space"
  shows how full the space is and how fast it grew. "Oldest databases" lists
  the databases by xid age, with their multixact age. The section needs a
  capture made with this version.
- 08d50e3: `pg-probe top` has a log view on `F6` or `6`. On the database host, it shows the lines of the server's csvlog as they arrive, like `tail -f`, and `e`, `t` and `c` keep only the errors, the temp files or the checkpoints. Each filter shows beside the list a summary since `top` started, which `←` and `→` resize: the errors with the most lines per user and database, with the count of the last 5 minutes; the statements that wrote the most temp file bytes; and what started the checkpoints, with p95 and p99 of buffers, write and sync time. `Enter` shows every field of the selected line with its statement, and `/` searches the lines. When `logging_collector`, `log_temp_files` or `log_checkpoints` is off, the view says what is missing and `S` opens the settings view at that setting. It reads the file from the local disk, at most 8 MiB per refresh, and writes none of it. It works from PostgreSQL 9.3.
- 0f1596c: `pg-probe top` has a settings view on `F5` or `5`. It lists every row of `pg_settings` under its category, shows each value beside its default, marks the values that differ from the built-in default and the ones that wait for a restart, and shows beside the list the description of the selected setting, how many times a changed amount is more or less than the default, the source with its file and line, and the context. `/` searches names and categories, `d` keeps only what differs from the default, and `p` only what waits for a restart. It reads `pg_settings` once when the view opens and again on `r`. It works from PostgreSQL 9.3; the restart mark needs 9.5.
- 1471bb2: `pg-probe top` shows the table that owns the TOAST table when you open the details of an autovacuum worker that is vacuuming one. It runs one catalog query in the backend's database, when the panel opens.
- 5cb2053: The vacuum view of `pg-probe top` now shows what should be vacuumed against
  wraparound and is not. It lists the tables past `autovacuum_freeze_max_age` or
  `autovacuum_multixact_freeze_max_age`, worst first, with the TOAST tables shown
  under their owner. Each row has the time since the last autovacuum, the workers
  seen on the table and the autovacuums that finished. A failed autovacuum never
  counts as finished. A head line shows how full the multixact member space is,
  and the database that holds the oldest multixact. The view reads these every
  30 seconds and only while it is open. Reading the member space needs a
  superuser, or from 11 EXECUTE on `pg_ls_dir`.
  Before 9.5 there is no multixact age, so only the xid age counts, and the view
  says so.

### Patch Changes

- 0f1596c: `pg-probe top` fills the key hint of a switch that is on, so the footer says what is in effect: `tab preview` while the preview is open, `g by user` while the list is grouped, and `space paused` while the refresh is paused. The `space` hint no longer changes to `resume`.

## 0.5.0

### Minor Changes

- b8e9f23: The review console shows the pg-probe version next to the brand.
- 81a5eed: `pg-probe capture` creates a missing data directory, private to its owner, as
  `pg_basebackup -D` does, instead of failing.
- 0e80ca4: pg-probe runs on macOS, on Apple Silicon and Intel. Install it with
  `brew install momoi-labs/packages/pg-probe`. The review console and
  `pg-probe top` work as on Linux, without the host row in `top`. A capture
  collects PostgreSQL and records Linux collection as unsupported; use
  `--domain postgresql` to skip it.
- b5811fa: `pg-probe mcp` lets an AI agent such as Claude Code or Codex query the
  artifacts in a directory, instead of reading the console panel by panel. It is
  a local MCP server over stdio with four tools: list the snapshots, describe
  each table and column, run a read-only SQL query, and export a query to CSV.
  SQL on an artifact can now compute percentiles and use math functions.
- 1771dc8: `pg-probe top` now uses the full width of the terminal, with no empty columns
  at its sides, and a page that fits uses the full width as well. An empty list
  says so in the middle of the screen. The header shows the time of the last
  refresh in place of the spinner, and `‖ paused` with the time of the data
  while paused. In `top` and in the capture, the version shows apart from the
  name, on a gray fill.

### Patch Changes

- 7026a6a: The capture output starts on the first column, aligned with the shell prompt,
  and its section rules and status banner reach the edge of the terminal, as in
  `pg-probe top`.

## 0.4.0

### Minor Changes

- a72a607: pg-probe connects to a server whose TLS certificate is self-signed, as on
  Debian and Ubuntu, instead of failing. TLS follows libpq's `sslmode`, from
  `PGSSLMODE` or the conninfo string. The default, `prefer`, does not check the
  certificate and retries without TLS when the handshake fails.
  
  Without a host, pg-probe connects through the server's socket in
  `/var/run/postgresql` or `/tmp`, as psql does. `-h` also takes the socket file
  itself. A failed connection now says why, and `pg-probe top --verbose` prints
  each step of the connection.

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
