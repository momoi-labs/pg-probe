# Artifact catalog

An artifact is a SQLite file that holds one snapshot: one run of pg-probe on the collector host. The `snapshot` table has one row. Every other table has a `snapshot_id` column that joins to `snapshot.snapshot_id`.

A snapshot holds one or more samples. A default run takes exactly one sample. A run with a duration takes one sample per interval. The `sample` table has one row per sample, numbered from 1, with the time it started in `sample.taken_at_utc`.

Tables hold two kinds of observations:

- Invariant tables have no `sample_id` column. pg-probe reads them once per snapshot, in the first sample. Examples are host identity, CPU topology, mounts, kernel settings, PostgreSQL settings and the database inventory.
- Variant tables have a `sample_id` column. pg-probe reads them again in every sample. Join them to `sample` on (`snapshot_id`, `sample_id`) to get the time of each row.

Many variant columns are cumulative counters. They count since boot, since the server started or since the last statistics reset, so one value alone says little. Subtract the value of the previous sample from the value of the current sample to get the change in that interval. Divide by the seconds between the two `sample.taken_at_utc` values to get a rate per second. The first sample has no previous sample, so it has no rate. A negative difference means the counter was reset (a restart or a statistics reset), so treat that interval as unknown. A snapshot with one sample has no rates. Gauges are values at the moment of the sample, and need no difference.

Example: commits per second over all databases.

```sql
WITH t AS (
  SELECT s.sample_id, s.taken_at_utc, SUM(d.xact_commit) AS commits
  FROM postgres_database_stats d
  JOIN sample s ON s.snapshot_id = d.snapshot_id AND s.sample_id = d.sample_id
  GROUP BY s.sample_id, s.taken_at_utc
)
SELECT taken_at_utc,
  (commits - LAG(commits) OVER w)
  / ((julianday(taken_at_utc) - julianday(LAG(taken_at_utc) OVER w)) * 86400.0) AS commits_per_second
FROM t WINDOW w AS (ORDER BY sample_id);
```

Every collector writes one row per source to `collector_outcome`. The `source` column names what was read, such as `proc.meminfo`, `proc.diskstats` or `postgres.stats.wal`. A status of `collected` means pg-probe read the source. `unsupported` means the source does not exist on this host or server version, and does not make the snapshot partial. `permission_denied`, `parse_error`, `io_error` and `raced` are failures, and any of them makes `snapshot.status` equal `partial`. When a table has no rows, look up its source in `collector_outcome` to learn why.

NULL means pg-probe did not observe the value: the source was missing, unreadable, absent in this kernel or PostgreSQL version, or the field was left out on purpose. NULL never means zero.

All timestamps written by pg-probe are RFC 3339 text in UTC, such as `2026-08-19T12:00:00.123456Z`. Columns copied as text from PostgreSQL, such as most `stats_reset` values, keep the PostgreSQL text format. Column names carry their unit where one applies: `_bytes`, `_ms`, `_microseconds`, `_seconds`, `_percent`, `_pages`.

## snapshot

One row per artifact. pg-probe writes it when the run starts and sets `finished_at_utc` and `status` when the run ends. Per snapshot.

- `snapshot_id`: UUID of the snapshot. Every other table joins on it.
- `schema_version`: Version of the artifact schema. The same number is in `PRAGMA user_version`.
- `collector_name`: Name of the program that wrote the artifact, always `pg-probe`.
- `collector_version`: Version of pg-probe that wrote the artifact.
- `started_at_utc`: Time the run started. UTC, RFC 3339.
- `finished_at_utc`: Time the run ended. UTC, RFC 3339. Equal to `started_at_utc` in a partial file that was never finalized.
- `status`: `complete`, or `partial` when any collector outcome failed or the run was interrupted.

## sample

One row per sample in the snapshot. The first sample starts with the snapshot. Later samples start one interval apart. Per sample.

- `snapshot_id`: Snapshot this sample belongs to.
- `sample_id`: Sample number, from 1. Variant tables join on (`snapshot_id`, `sample_id`).
- `taken_at_utc`: Time the sample started. UTC, RFC 3339. Use it to compute the seconds between samples.

## collector_outcome

One row per source each time a collector ran it. A source that runs in every sample has one row per sample, and `postgres.extensions.visit` has one row per database. The table has no `sample_id`: order rows by `sequence` or compare times with `sample.taken_at_utc`.

- `snapshot_id`: Snapshot this outcome belongs to.
- `source`: Symbolic name of what was read, such as `proc.stat.cpu`, `sys.block.devices`, `postgres.connection` or `postgres.stats.io`.
- `sequence`: Running number of the outcome in the snapshot, from 0, in the order pg-probe wrote them.
- `started_at_utc`: Time pg-probe started to read the source. UTC, RFC 3339.
- `finished_at_utc`: Time pg-probe finished with the source. UTC, RFC 3339.
- `status`: `collected`, `unsupported`, `permission_denied`, `parse_error`, `io_error` or `raced`. Only the last four make the snapshot partial.
- `error_code`: Stable code that names the cause, such as `psi_read_failed` or `not_in_server_version`. NULL when the status is `collected`.
- `os_error_code`: Linux errno of the failed system call, when there was one. NULL otherwise.
- `observed_at_utc`: Time the observation was taken. UTC, RFC 3339. Set only when the status is `collected`.
- `collector_name`: Name of the program that wrote the row, always `pg-probe`.
- `collector_version`: Version of pg-probe that wrote the row.

## linux_identity

One row per snapshot that identifies the collector host. pg-probe reads `uname(2)`, `/etc/os-release` (or `/usr/lib/os-release`), `sysconf(3)` and `/proc/uptime`. Per snapshot.

- `snapshot_id`: Snapshot this row belongs to.
- `hostname`: Host name, the `nodename` field of `uname(2)`.
- `kernel_sysname`: Kernel name, the `sysname` field of `uname(2)`, such as `Linux`.
- `kernel_release`: Kernel release, the `release` field of `uname(2)`, such as `6.12.0`.
- `kernel_version`: Kernel build string, the `version` field of `uname(2)`.
- `kernel_machine`: Hardware architecture, the `machine` field of `uname(2)`, such as `x86_64`.
- `distribution_id`: `ID` from os-release, such as `debian`.
- `distribution_version_id`: `VERSION_ID` from os-release, such as `13`.
- `distribution_pretty_name`: `PRETTY_NAME` from os-release, such as `Debian GNU/Linux 13`.
- `page_size_bytes`: Memory page size, `sysconf(_SC_PAGESIZE)`. Bytes.
- `ticks_per_second`: Clock ticks per second, `sysconf(_SC_CLK_TCK)`. It is the unit of `linux_cpu_time.ticks`.
- `uptime_microseconds`: Time since boot, the first field of `/proc/uptime`, when pg-probe read it. Microseconds.

## linux_cpu_set

One row per CPU in each CPU set. pg-probe reads `/sys/devices/system/cpu/online`, `present` and `possible`, and `Cpus_allowed_list` from `/proc/self/status`. Per snapshot.

- `snapshot_id`: Snapshot this row belongs to.
- `collected_at`: Time pg-probe wrote the batch that holds this row. UTC, RFC 3339.
- `set_kind`: Which set: `online`, `present` and `possible` from sysfs, or `affinity`, the CPUs the pg-probe process may run on. Count rows per kind to get the CPU count.
- `cpu_id`: Logical CPU number.

## linux_cpu_topology

One row per online CPU. pg-probe reads `/sys/devices/system/cpu/cpuN/topology/`. Per snapshot.

- `snapshot_id`: Snapshot this row belongs to.
- `collected_at`: Time pg-probe wrote the batch that holds this row. UTC, RFC 3339.
- `cpu_id`: Logical CPU number.
- `package_id`: Physical socket, from `topology/physical_package_id`.
- `die_id`: Die inside the package, from `topology/die_id`.
- `core_id`: Core inside the package, from `topology/core_id`.
- `sibling_set`: CPUs that share this core, from `topology/thread_siblings_list`, as a list such as `0,4` or `0-1`.

## linux_numa_node

One row per online NUMA node. pg-probe reads `/sys/devices/system/node/online` and `/sys/devices/system/node/nodeN/cpulist`. Per snapshot. No rows when the host has no NUMA sysfs.

- `snapshot_id`: Snapshot this row belongs to.
- `collected_at`: Time pg-probe wrote the batch that holds this row. UTC, RFC 3339.
- `node_id`: NUMA node number.
- `cpu_set`: CPUs on this node, as a list such as `0-7,16-23`.

## linux_cpu_time

One row per CPU and state in each sample. pg-probe reads the `cpu` lines of `/proc/stat`. Per sample. All values are cumulative counters since boot.

- `snapshot_id`: Snapshot this row belongs to.
- `sample_id`: Sample this row belongs to.
- `collected_at`: Time pg-probe wrote the batch that holds this row. UTC, RFC 3339.
- `cpu_id`: Logical CPU number. `-1` is the aggregate `cpu` line, the sum over all CPUs.
- `state`: CPU state: `user`, `nice`, `system`, `idle`, `iowait`, `irq`, `softirq`, `steal`, `guest` or `guest_nice`. `guest` and `guest_nice` are already counted in `user` and `nice`.
- `ticks`: Time spent in this state since boot. Unit is `linux_identity.ticks_per_second`. Cumulative counter. A share per state is its difference divided by the sum of differences of the first eight states.

## linux_scheduler

One row per sample with system-wide scheduler counters. pg-probe reads `/proc/stat` and `/proc/loadavg`. Per sample. Columns from a source that failed are NULL.

- `snapshot_id`: Snapshot this row belongs to.
- `sample_id`: Sample this row belongs to.
- `collected_at`: Time pg-probe wrote the batch that holds this row. UTC, RFC 3339.
- `procs_running`: Processes in a runnable state, `procs_running` in `/proc/stat`. Gauge.
- `procs_blocked`: Processes blocked waiting for I/O, `procs_blocked` in `/proc/stat`. Gauge.
- `ctxt`: Context switches since boot, `ctxt` in `/proc/stat`. Cumulative counter.
- `processes`: Processes and threads created since boot, `processes` in `/proc/stat`. Cumulative counter.
- `intr`: Interrupts serviced since boot, the first number of the `intr` line in `/proc/stat`. Cumulative counter.
- `softirq`: Soft interrupts serviced since boot, the first number of the `softirq` line in `/proc/stat`. Cumulative counter.
- `load_avg_1m`: Load average over 1 minute, field 1 of `/proc/loadavg`. Gauge.
- `load_avg_5m`: Load average over 5 minutes, field 2 of `/proc/loadavg`. Gauge.
- `load_avg_15m`: Load average over 15 minutes, field 3 of `/proc/loadavg`. Gauge.
- `runnable_tasks`: Runnable kernel scheduling entities, the number before the slash in field 4 of `/proc/loadavg`. Gauge.
- `total_tasks`: Kernel scheduling entities that exist, the number after the slash in field 4 of `/proc/loadavg`. Gauge.

## linux_memory

One row per allowlisted `/proc/meminfo` key in each sample. The keys are `MemTotal`, `MemFree`, `MemAvailable`, `Buffers`, `Cached`, `SReclaimable`, `SUnreclaim`, `Slab`, `Shmem`, `SwapTotal`, `SwapFree`, `Dirty`, `Writeback`, `AnonPages`, `Mapped`, `PageTables`, `KernelStack`, `AnonHugePages`, `Hugepagesize`, `HugePages_Total`, `HugePages_Free`, `HugePages_Rsvd` and `HugePages_Surp`. Per sample. All values are gauges.

- `snapshot_id`: Snapshot this row belongs to.
- `sample_id`: Sample this row belongs to.
- `collected_at`: Time pg-probe wrote the batch that holds this row. UTC, RFC 3339.
- `key`: The `/proc/meminfo` field name, such as `MemAvailable`.
- `value_bytes`: The value. Bytes when `unit` is `bytes` (pg-probe multiplies the kB values by 1024). A page count when `unit` is `pages`. Gauge.
- `unit`: `bytes`, or `pages` for the `HugePages_*` keys.

## linux_vm_event

One row per allowlisted `/proc/vmstat` key in each sample. The keys are `pgpgin`, `pgpgout`, `pswpin`, `pswpout`, `pgfault`, `pgmajfault`, `oom_kill`, and every key that starts with `allocstall_`, `pgscan_`, `pgsteal_` or `compact_`. Per sample. Values are cumulative counters since boot.

- `snapshot_id`: Snapshot this row belongs to.
- `sample_id`: Sample this row belongs to.
- `collected_at`: Time pg-probe wrote the batch that holds this row. UTC, RFC 3339.
- `key`: The `/proc/vmstat` field name, such as `pgmajfault` or `pgscan_direct`. Older kernels split some counters per zone, such as `pgscan_direct_normal`.
- `value`: The counter value. `pswpin` and `pswpout` count pages. Cumulative counter.
- `unit`: `KiB` for `pgpgin` and `pgpgout`, `events` for every other key.

## linux_psi

One row per resource and scope in each sample. pg-probe reads `/proc/pressure/cpu`, `/proc/pressure/memory` and `/proc/pressure/io` (pressure stall information). Per sample. No rows when the kernel has no PSI.

- `snapshot_id`: Snapshot this row belongs to.
- `sample_id`: Sample this row belongs to.
- `collected_at`: Time pg-probe wrote the batch that holds this row. UTC, RFC 3339.
- `resource`: `cpu`, `memory` or `io`.
- `scope`: `some` (at least one task stalled) or `full` (all non-idle tasks stalled).
- `avg10`: Share of time stalled, moving average over 10 seconds. Percent, 0 to 100. Gauge.
- `avg60`: Share of time stalled, moving average over 60 seconds. Percent, 0 to 100. Gauge.
- `avg300`: Share of time stalled, moving average over 300 seconds. Percent, 0 to 100. Gauge.
- `total_us`: Total stall time since boot, the `total` field. Microseconds. Cumulative counter.

## linux_mount

One row per mount visible to the pg-probe process. pg-probe reads `/proc/self/mountinfo` and calls `statvfs(3)` on recorded mount points. Per snapshot.

- `snapshot_id`: Snapshot this row belongs to.
- `collected_at`: Time pg-probe wrote the batch that holds this row. UTC, RFC 3339.
- `mount_id`: Mount ID, field 1 of mountinfo.
- `parent_mount_id`: Mount ID of the parent mount, field 2 of mountinfo.
- `device_major`: Major device number, from field 3 of mountinfo. Joins to `linux_block_device` and `linux_block_io`.
- `device_minor`: Minor device number, from field 3 of mountinfo.
- `mount_point`: Path of the mount. Set only for local disk filesystems (ext2, ext3, ext4, xfs, btrfs, zfs, f2fs, jfs, reiserfs) outside `/home`. NULL otherwise.
- `filesystem_type`: Filesystem type, the field after the `-` separator in mountinfo, such as `ext4` or `tmpfs`.
- `is_read_only`: 1 when the mount options include `ro`, else 0.
- `total_bytes`: Filesystem size, `f_blocks * f_frsize` from `statvfs`. Bytes. NULL when `mount_point` is NULL.
- `available_bytes`: Space available to unprivileged users, `f_bavail * f_frsize` from `statvfs`. Bytes. NULL when `mount_point` is NULL.
- `total_inodes`: Inode count, `f_files` from `statvfs`. NULL when `mount_point` is NULL.
- `available_inodes`: Inodes available to unprivileged users, `f_favail` from `statvfs`. NULL when `mount_point` is NULL.

## linux_block_device

One row per entry in `/sys/block`, which lists whole block devices, not partitions. pg-probe reads `/sys/block/<name>/dev` and the files in `/sys/block/<name>/queue/`. Per snapshot.

- `snapshot_id`: Snapshot this row belongs to.
- `collected_at`: Time pg-probe wrote the batch that holds this row. UTC, RFC 3339.
- `device_major`: Major device number, from `/sys/block/<name>/dev`.
- `device_minor`: Minor device number, from `/sys/block/<name>/dev`.
- `kernel_name`: Kernel device name, such as `nvme0n1` or `sda`.
- `logical_block_size_bytes`: `queue/logical_block_size`. Bytes.
- `physical_block_size_bytes`: `queue/physical_block_size`. Bytes.
- `minimum_io_size_bytes`: `queue/minimum_io_size`. Bytes.
- `optimal_io_size_bytes`: `queue/optimal_io_size`. Bytes. 0 when the device reports none.
- `read_ahead_kibibytes`: `queue/read_ahead_kb`. KiB.
- `request_queue_depth`: `queue/nr_requests`, the request queue size. Requests.
- `is_rotational`: 1 when `queue/rotational` is 1, else 0.
- `scheduler`: Active I/O scheduler, the bracketed entry of `queue/scheduler`, such as `mq-deadline` or `none`.

## linux_block_io

One row per line of `/proc/diskstats` in each sample. It has whole disks and partitions, so join to `linux_block_device` on major and minor to keep whole disks only. Per sample. Columns are cumulative counters except `io_in_progress`.

- `snapshot_id`: Snapshot this row belongs to.
- `sample_id`: Sample this row belongs to.
- `collected_at`: Time pg-probe wrote the batch that holds this row. UTC, RFC 3339.
- `device_major`: Major device number, diskstats field 1.
- `device_minor`: Minor device number, diskstats field 2.
- `reads_completed`: Reads completed, diskstats field 4. Cumulative counter.
- `reads_merged`: Adjacent reads merged, diskstats field 5. Cumulative counter.
- `sectors_read`: Sectors read, diskstats field 6. A sector is 512 bytes, whatever the device block size. Cumulative counter.
- `read_milliseconds`: Time spent on reads, diskstats field 7. Milliseconds. Cumulative counter. Its difference divided by the difference of `reads_completed` is the average read latency.
- `writes_completed`: Writes completed, diskstats field 8. Cumulative counter.
- `writes_merged`: Adjacent writes merged, diskstats field 9. Cumulative counter.
- `sectors_written`: Sectors written, diskstats field 10. 512 bytes each. Cumulative counter.
- `write_milliseconds`: Time spent on writes, diskstats field 11. Milliseconds. Cumulative counter.
- `io_in_progress`: I/O requests in flight at the moment of the sample, diskstats field 12. Gauge.
- `io_milliseconds`: Time the device had at least one request in flight, diskstats field 13. Milliseconds. Cumulative counter. Its difference divided by the elapsed milliseconds is the busy share.
- `weighted_io_milliseconds`: Time in flight weighted by the number of requests, diskstats field 14. Milliseconds. Cumulative counter. Its difference divided by the elapsed milliseconds is the average queue depth.
- `discards_completed`: Discards completed, diskstats field 15. Cumulative counter. NULL when the kernel does not report the field.
- `discard_sectors`: Sectors discarded, diskstats field 17. 512 bytes each. Cumulative counter. NULL when the kernel does not report the field.
- `discard_milliseconds`: Time spent on discards, diskstats field 18. Milliseconds. Cumulative counter. NULL when the kernel does not report the field.
- `flushes_completed`: Flush requests completed, diskstats field 19. Cumulative counter. NULL when the kernel does not report the field.
- `flush_milliseconds`: Time spent on flushes, diskstats field 20. Milliseconds. Cumulative counter. NULL when the kernel does not report the field.

## linux_network_io

One row per network interface in each sample. pg-probe reads `/proc/net/dev` and skips `lo` and interfaces whose name starts with `veth`, `docker`, `br-`, `virbr`, `cni`, `flannel`, `cali`, `vxlan` or `kube-`. Per sample. All values are cumulative counters.

- `snapshot_id`: Snapshot this row belongs to.
- `sample_id`: Sample this row belongs to.
- `collected_at`: Time pg-probe wrote the batch that holds this row. UTC, RFC 3339.
- `interface`: Interface name, such as `eth0`.
- `rx_bytes`: Bytes received. Cumulative counter.
- `rx_packets`: Packets received. Cumulative counter.
- `rx_errors`: Receive errors. Cumulative counter.
- `rx_dropped`: Received packets dropped. Cumulative counter.
- `tx_bytes`: Bytes sent. Cumulative counter.
- `tx_packets`: Packets sent. Cumulative counter.
- `tx_errors`: Send errors. Cumulative counter.
- `tx_dropped`: Sent packets dropped. Cumulative counter.

## linux_resource_limit

One row per resource limit of the pg-probe process, not of PostgreSQL. pg-probe calls `getrlimit(2)`. Per snapshot.

- `snapshot_id`: Snapshot this row belongs to.
- `collected_at`: Time pg-probe wrote the batch that holds this row. UTC, RFC 3339.
- `resource`: `nofile` (RLIMIT_NOFILE), `nproc` (RLIMIT_NPROC), `memlock` (RLIMIT_MEMLOCK), `address_space` (RLIMIT_AS), `core` (RLIMIT_CORE), `file_size` (RLIMIT_FSIZE) or `stack` (RLIMIT_STACK).
- `soft_value`: Soft limit, `rlim_cur`. In `unit`. NULL when unlimited.
- `hard_value`: Hard limit, `rlim_max`. In `unit`. NULL when unlimited.
- `soft_unlimited`: 1 when the soft limit is RLIM_INFINITY, else 0.
- `hard_unlimited`: 1 when the hard limit is RLIM_INFINITY, else 0.
- `unit`: `count` for `nofile` and `nproc`, `bytes` for the others.

## linux_cgroup

One row with the limits of the cgroup of the pg-probe process. pg-probe uses cgroup v2 when `/proc/self/mountinfo` has a `cgroup2` mount, and reads the cgroup path from `/proc/self/cgroup`. Per snapshot, from the first sample.

- `snapshot_id`: Snapshot this row belongs to.
- `collected_at`: Time pg-probe wrote the batch that holds this row. UTC, RFC 3339.
- `version`: `v1` or `v2`.
- `cpu_quota_microseconds`: CPU time allowed per period. v2: first field of `cpu.max`. v1: `cpu.cfs_quota_us`. Microseconds. NULL when unlimited.
- `cpu_period_microseconds`: Length of the quota period. v2: second field of `cpu.max`. v1: `cpu.cfs_period_us`. Microseconds.
- `cpu_quota_unlimited`: 1 when `cpu.max` starts with `max` (v2) or `cpu.cfs_quota_us` is -1 (v1), else 0.
- `cpu_weight`: `cpu.weight` (v2 only). NULL on v1.
- `memory_min_bytes`: `memory.min` (v2 only). Bytes. NULL on v1.
- `memory_low_bytes`: `memory.low` (v2 only). Bytes. NULL on v1.
- `memory_high_bytes`: `memory.high` (v2 only). Bytes. NULL when unlimited or on v1.
- `memory_high_unlimited`: 1 when `memory.high` is `max`, else 0. NULL on v1.
- `memory_max_bytes`: v2: `memory.max`. v1: `memory.limit_in_bytes`, which holds a very large number when unlimited. Bytes. NULL when unlimited on v2.
- `memory_max_unlimited`: 1 when `memory.max` is `max` (v2) or `memory.limit_in_bytes` is at least 0x7FFFFFFFFFFFF000 (v1), else 0.
- `swap_max_bytes`: `memory.swap.max` (v2 only). Bytes. NULL when unlimited or on v1.
- `swap_max_unlimited`: 1 when `memory.swap.max` is `max` (v2), else 0. On v1, 1 only when `memory.memsw.limit_in_bytes` is unlimited, else NULL.
- `pids_max`: `pids.max`. Processes. NULL when unlimited.
- `pids_max_unlimited`: 1 when `pids.max` is `max`, else 0.

## linux_cgroup_usage

One row per sample with the usage of the cgroup of the pg-probe process. Same cgroup as `linux_cgroup`. Per sample. All values are gauges.

- `snapshot_id`: Snapshot this row belongs to.
- `sample_id`: Sample this row belongs to.
- `collected_at`: Time pg-probe wrote the batch that holds this row. UTC, RFC 3339.
- `memory_current_bytes`: Memory charged to the cgroup, page cache included. v2: `memory.current`. v1: `memory.usage_in_bytes`. Bytes. Gauge.
- `swap_current_bytes`: Swap charged to the cgroup. v2: `memory.swap.current`. v1: `memory.memsw.usage_in_bytes` minus `memory.usage_in_bytes`. Bytes. Gauge.
- `pids_current`: Processes in the cgroup, `pids.current`. Gauge.

## linux_cgroup_event

One row per cgroup counter in each sample. On v2, pg-probe reads every line of `cpu.stat`, `memory.events` and `pids.events`. On v1, it records only CPU usage from `cpuacct.usage`. Per sample. All values are cumulative counters.

- `snapshot_id`: Snapshot this row belongs to.
- `sample_id`: Sample this row belongs to.
- `collected_at`: Time pg-probe wrote the batch that holds this row. UTC, RFC 3339.
- `controller`: Controller of the file: `cpu`, `memory` or `pids`.
- `event_name`: Key in the file, such as `usage_usec` and `nr_throttled` (cpu), `high`, `max` and `oom_kill` (memory), or `max` (pids). Older artifacts may hold `usage_us` in nanoseconds for v1 CPU usage.
- `cumulative_value`: Counter value. In `unit`. Cumulative counter.
- `unit`: `microseconds` for `cpu.stat` keys that end in `_usec` and for v1 CPU usage, `count` for the others.

## linux_cgroup_io

One row per block device in each sample, from the I/O counters of the cgroup of the pg-probe process. v2: `io.stat`. v1: `blkio.io_service_bytes_recursive`. Per sample. All values are cumulative counters.

- `snapshot_id`: Snapshot this row belongs to.
- `sample_id`: Sample this row belongs to.
- `collected_at`: Time pg-probe wrote the batch that holds this row. UTC, RFC 3339.
- `device_major`: Major device number.
- `device_minor`: Minor device number.
- `read_bytes`: Bytes read. v2: `rbytes`. v1: `Read`. Bytes. Cumulative counter.
- `write_bytes`: Bytes written. v2: `wbytes`. v1: `Write`. Bytes. Cumulative counter.
- `discard_bytes`: Bytes discarded. v2: `dbytes`. v1: `Discard`. Bytes. Cumulative counter.
- `read_operations`: Read operations, `rios` (v2 only). Cumulative counter. NULL on v1.
- `write_operations`: Write operations, `wios` (v2 only). Cumulative counter. NULL on v1.
- `discard_operations`: Discard operations, `dios` (v2 only). Cumulative counter. NULL on v1.

## linux_kernel_vm

One row with virtual memory settings of the kernel. pg-probe reads files in `/proc/sys/vm/`. Per snapshot. A column is NULL when its file is missing or unreadable.

- `snapshot_id`: Snapshot this row belongs to.
- `collected_at`: Time pg-probe wrote the batch that holds this row. UTC, RFC 3339.
- `overcommit_mode`: `vm.overcommit_memory`: 0, 1 or 2.
- `overcommit_ratio_percent`: `vm.overcommit_ratio`. Percent.
- `overcommit_kbytes_source`: `vm.overcommit_kbytes` as read. KiB.
- `overcommit_bytes`: `vm.overcommit_kbytes` multiplied by 1024. Bytes.
- `max_map_count`: `vm.max_map_count`. Memory map areas per process.
- `swappiness`: `vm.swappiness`.
- `zone_reclaim_mode`: `vm.zone_reclaim_mode`.
- `min_free_kbytes_source`: `vm.min_free_kbytes` as read. KiB.
- `min_free_bytes`: `vm.min_free_kbytes` multiplied by 1024. Bytes.
- `dirty_background_bytes`: `vm.dirty_background_bytes`. Bytes. 0 when the ratio applies instead.
- `dirty_background_ratio_percent`: `vm.dirty_background_ratio`. Percent.
- `dirty_bytes`: `vm.dirty_bytes`. Bytes. 0 when the ratio applies instead.
- `dirty_ratio_percent`: `vm.dirty_ratio`. Percent.
- `dirty_expire_microseconds`: `vm.dirty_expire_centisecs` multiplied by 10000. Microseconds.
- `dirty_writeback_microseconds`: `vm.dirty_writeback_centisecs` multiplied by 10000. Microseconds.

## linux_kernel_ipc

One row with System V IPC and file handle limits of the kernel. pg-probe reads `/proc/sys/kernel/shmmax`, `shmall`, `shmmni` and `sem`, and `/proc/sys/fs/file-max` and `nr_open`. Per snapshot.

- `snapshot_id`: Snapshot this row belongs to.
- `collected_at`: Time pg-probe wrote the batch that holds this row. UTC, RFC 3339.
- `shmmax_bytes`: `kernel.shmmax`, the largest shared memory segment. Bytes.
- `shmall_pages`: `kernel.shmall`, the total shared memory allowed. Pages.
- `shmall_bytes`: `kernel.shmall` multiplied by 4096. Bytes. pg-probe uses 4096 here, not `linux_identity.page_size_bytes`.
- `shmmni`: `kernel.shmmni`, the maximum number of shared memory segments.
- `semmsl`: Field 1 of `kernel.sem`, semaphores per set.
- `semmns`: Field 2 of `kernel.sem`, semaphores in the system.
- `semopm`: Field 3 of `kernel.sem`, operations per `semop` call.
- `semmni`: Field 4 of `kernel.sem`, semaphore sets in the system.
- `file_max`: `fs.file-max`, the system-wide limit of open file handles.
- `nr_open`: `fs.nr_open`, the ceiling for a per-process open file limit.

## linux_thp

One row with transparent huge page settings. pg-probe reads `/sys/kernel/mm/transparent_hugepage/`. Per snapshot.

- `snapshot_id`: Snapshot this row belongs to.
- `collected_at`: Time pg-probe wrote the batch that holds this row. UTC, RFC 3339.
- `enabled_policy`: Bracketed value of `enabled`: `always`, `madvise` or `never`.
- `defrag_policy`: Bracketed value of `defrag`, such as `madvise`.
- `khugepaged_defrag`: 1 when `khugepaged/defrag` is 1, else 0.

## postgres_target

One row that identifies the PostgreSQL target and the role pg-probe connected as. pg-probe reads `current_setting()`, `pg_is_in_recovery()`, `pg_roles` and `pg_has_role()` on the initial connection. Per snapshot.

- `snapshot_id`: Snapshot this row belongs to.
- `collected_at`: Time pg-probe wrote the batch that holds this row. UTC, RFC 3339.
- `endpoint_kind`: `tcp` or `unix_socket`.
- `endpoint`: Host name or IP address for `tcp`, or the socket directory for `unix_socket`, as given in the connection settings.
- `port`: TCP port. NULL for `unix_socket`.
- `connected_user`: Role of the session, `current_user`.
- `initial_database`: Database of the first connection, `current_database()`.
- `server_version_num`: `server_version_num`, such as `160004` for 16.4 or `90624` for 9.6.24.
- `server_version`: `server_version` text, such as `16.4`.
- `in_recovery`: 1 when `pg_is_in_recovery()` is true (a standby), else 0.
- `block_size_bytes`: `block_size` setting, the size of a data page. Bytes. Multiply page and buffer counts by it to get bytes.
- `is_superuser`: 1 when `pg_roles.rolsuper` is true for the role, else 0.
- `has_pg_monitor`: 1 when the role is a member of `pg_monitor`, else 0. Always 0 before PostgreSQL 10.
- `has_pg_read_all_stats`: 1 when the role is a member of `pg_read_all_stats`, else 0. Always 0 before PostgreSQL 10.
- `has_pg_read_all_settings`: 1 when the role is a member of `pg_read_all_settings`, else 0. Always 0 before PostgreSQL 10.

## postgres_setting

One row per allowlisted PostgreSQL setting. pg-probe reads `pg_settings` and keeps only a fixed allowlist of names. Per snapshot.

- `snapshot_id`: Snapshot this row belongs to.
- `collected_at`: Time pg-probe wrote the batch that holds this row. UTC, RFC 3339.
- `setting_name`: `pg_settings.name`, such as `shared_buffers`.
- `setting_value`: `pg_settings.setting`, as text, in the unit given by `unit`.
- `unit`: `pg_settings.unit`, such as `8kB`, `kB`, `ms` or `s`. NULL when the setting has no unit.
- `value_type`: `pg_settings.vartype`: `bool`, `enum`, `integer`, `real` or `string`.
- `source`: `pg_settings.source`, where the value came from, such as `default` or `configuration file`.
- `pending_restart`: 1 when `pg_settings.pending_restart` is true, else 0. Always 0 before PostgreSQL 9.5.

## postgres_database

One row per database in `pg_database`. pg-probe reads `pg_database` and `pg_database_size()`. Per snapshot.

- `snapshot_id`: Snapshot this row belongs to.
- `collected_at`: Time pg-probe wrote the batch that holds this row. UTC, RFC 3339.
- `database_name`: `pg_database.datname`. pg-probe writes `<invalid>` for a name with characters it does not allow.
- `is_template`: 1 when `pg_database.datistemplate` is true, else 0.
- `allows_connections`: 1 when `pg_database.datallowconn` is true, else 0.
- `can_connect`: 1 when the database allows connections and is not a template, else 0. pg-probe visits only these databases.
- `size_bytes`: `pg_database_size(oid)`. Bytes. NULL when pg-probe could not read the size.
- `extension_visit_status`: Result of the per-database visit that reads extensions and relations: `visited`, `not_connectable`, `permission_denied`, `connection_failed`, or `pending` when the visit never ran.

## postgres_extension

One row per installed extension in each visited database. pg-probe connects to each database where `can_connect` is 1 and reads `pg_extension`. Per snapshot.

- `snapshot_id`: Snapshot this row belongs to.
- `collected_at`: Time pg-probe wrote the batch that holds this row. UTC, RFC 3339.
- `database_name`: Database the extension is installed in. Joins to `postgres_database.database_name`.
- `extension_name`: `pg_extension.extname`.
- `extension_version`: `pg_extension.extversion`.

## postgres_relation

One row per selected table or materialized view in each visited database: the 20 largest by `relpages`, plus the 20 with the most dead tuples in `pg_stat_user_tables`. pg-probe reads `pg_class`, `pg_index` and `pg_stat_user_tables`, and leaves out `pg_catalog`, `information_schema` and TOAST schemas. Per snapshot.

- `snapshot_id`: Snapshot this row belongs to.
- `collected_at`: Time pg-probe wrote the batch that holds this row. UTC, RFC 3339.
- `database_name`: Database of the relation. Joins to `postgres_database.database_name`.
- `schema_name`: `pg_namespace.nspname` of the relation.
- `relation_name`: `pg_class.relname`.
- `relkind`: `pg_class.relkind`: `r` for a table, `m` for a materialized view.
- `heap_pages`: `pg_class.relpages` of the relation. Pages of `postgres_target.block_size_bytes`. PostgreSQL updates it at VACUUM, ANALYZE and some DDL.
- `index_pages`: Sum of `pg_class.relpages` over the indexes of the relation. Pages. 0 when it has no index.
- `toast_pages`: `pg_class.relpages` of the TOAST table. Pages. 0 when it has none.
- `estimated_rows`: `pg_class.reltuples`, the planner row estimate. -1 on PostgreSQL 14 and newer when the relation was never vacuumed or analyzed.
- `live_tuples`: `pg_stat_user_tables.n_live_tup`, estimated live rows. NULL when the relation has no row in `pg_stat_user_tables`.
- `dead_tuples`: `pg_stat_user_tables.n_dead_tup`, estimated dead rows. NULL when the relation has no row in `pg_stat_user_tables`.
- `last_autovacuum`: `pg_stat_user_tables.last_autovacuum`, as PostgreSQL text. NULL when autovacuum never ran on it.
- `autovacuum_count`: `pg_stat_user_tables.autovacuum_count`, autovacuum runs since the last statistics reset.
- `last_autoanalyze`: `pg_stat_user_tables.last_autoanalyze`, as PostgreSQL text. NULL when autoanalyze never ran on it.

## database_inventory

View over `postgres_database` that adds the number of extensions recorded per database. One row per database. Per snapshot.

- `snapshot_id`: Snapshot this row belongs to.
- `database_name`: Same as `postgres_database.database_name`.
- `is_template`: Same as `postgres_database.is_template`.
- `allows_connections`: Same as `postgres_database.allows_connections`.
- `can_connect`: Same as `postgres_database.can_connect`.
- `size_bytes`: Same as `postgres_database.size_bytes`. Bytes.
- `extension_visit_status`: Same as `postgres_database.extension_visit_status`.
- `extension_count`: Number of rows in `postgres_extension` for this database. 0 when the database was not visited.

## postgres_database_stats

One row per database in `pg_stat_database` in each sample, templates included. pg-probe reads `pg_stat_database`. Per sample. Columns are cumulative counters since the last statistics reset, except `num_backends` and `stats_reset`.

- `snapshot_id`: Snapshot this row belongs to.
- `sample_id`: Sample this row belongs to.
- `collected_at`: Time pg-probe wrote the batch that holds this row. UTC, RFC 3339.
- `database_oid`: `pg_stat_database.datid`.
- `database_name`: `pg_stat_database.datname`.
- `num_backends`: `numbackends`, backends connected to this database at the moment of the sample. Gauge.
- `xact_commit`: `xact_commit`, transactions committed. Cumulative counter.
- `xact_rollback`: `xact_rollback`, transactions rolled back. Cumulative counter.
- `blks_read`: `blks_read`, blocks read outside shared buffers. Blocks. Cumulative counter.
- `blks_hit`: `blks_hit`, blocks found in shared buffers. Blocks. Cumulative counter. Hit ratio is the difference of `blks_hit` over the sum of the differences of `blks_hit` and `blks_read`.
- `tup_returned`: `tup_returned`, rows returned by scans. Cumulative counter.
- `tup_fetched`: `tup_fetched`, rows fetched by index scans. Cumulative counter.
- `tup_inserted`: `tup_inserted`, rows inserted. Cumulative counter.
- `tup_updated`: `tup_updated`, rows updated. Cumulative counter.
- `tup_deleted`: `tup_deleted`, rows deleted. Cumulative counter.
- `conflicts`: `conflicts`, queries canceled by recovery conflicts. Cumulative counter.
- `temp_files`: `temp_files`, temporary files created. Cumulative counter.
- `temp_bytes`: `temp_bytes`, data written to temporary files. Bytes. Cumulative counter.
- `deadlocks`: `deadlocks`, deadlocks detected. Cumulative counter.
- `checksum_failures`: `checksum_failures`, data page checksum failures. Cumulative counter. NULL before PostgreSQL 14.
- `blk_read_time_ms`: `blk_read_time`, time backends spent reading blocks. Milliseconds. Cumulative counter. Stays 0 unless `track_io_timing` is on. NULL before PostgreSQL 14.
- `blk_write_time_ms`: `blk_write_time`, time backends spent writing blocks. Milliseconds. Cumulative counter. Stays 0 unless `track_io_timing` is on. NULL before PostgreSQL 14.
- `session_time_ms`: `session_time`, time spent in sessions. Milliseconds. Cumulative counter. NULL before PostgreSQL 14.
- `active_time_ms`: `active_time`, time spent running statements. Milliseconds. Cumulative counter. NULL before PostgreSQL 14.
- `idle_in_transaction_time_ms`: `idle_in_transaction_time`, time spent idle in a transaction. Milliseconds. Cumulative counter. NULL before PostgreSQL 14.
- `sessions`: `sessions`, sessions started. Cumulative counter. NULL before PostgreSQL 14.
- `sessions_abandoned`: `sessions_abandoned`, sessions ended because the client went away. Cumulative counter. NULL before PostgreSQL 14.
- `sessions_fatal`: `sessions_fatal`, sessions ended by a fatal error. Cumulative counter. NULL before PostgreSQL 14.
- `sessions_killed`: `sessions_killed`, sessions ended by an operator. Cumulative counter. NULL before PostgreSQL 14.
- `stats_reset`: `stats_reset`, time of the last statistics reset of this database. UTC, RFC 3339. NULL before PostgreSQL 14 or when never reset.

## postgres_standby_conflicts

One row per database in `pg_stat_database_conflicts` in each sample. pg-probe reads `pg_stat_database_conflicts`. Per sample. PostgreSQL counts these conflicts on standbys only. All counts are cumulative counters.

- `snapshot_id`: Snapshot this row belongs to.
- `sample_id`: Sample this row belongs to.
- `collected_at`: Time pg-probe wrote the batch that holds this row. UTC, RFC 3339.
- `database_oid`: `pg_stat_database_conflicts.datid`.
- `database_name`: `pg_stat_database_conflicts.datname`.
- `confl_tablespace`: `confl_tablespace`, queries canceled because of dropped tablespaces. Cumulative counter.
- `confl_lock`: `confl_lock`, queries canceled because of lock timeouts. Cumulative counter.
- `confl_snapshot`: `confl_snapshot`, queries canceled because of old snapshots. Cumulative counter.
- `confl_bufferpin`: `confl_bufferpin`, queries canceled because of pinned buffers. Cumulative counter.
- `confl_deadlock`: `confl_deadlock`, queries canceled because of deadlocks. Cumulative counter.

## postgres_wal_stats

One row per sample from `pg_stat_wal`. PostgreSQL 14 and newer. Per sample. Columns are cumulative counters since the last reset, except `stats_reset`.

- `snapshot_id`: Snapshot this row belongs to.
- `sample_id`: Sample this row belongs to.
- `collected_at`: Time pg-probe wrote the batch that holds this row. UTC, RFC 3339.
- `wal_records`: `wal_records`, WAL records generated. Cumulative counter.
- `wal_fpi`: `wal_fpi`, full page images generated. Cumulative counter.
- `wal_bytes`: `wal_bytes`, WAL generated. Bytes. Cumulative counter.
- `wal_buffers_full`: `wal_buffers_full`, times WAL was written because the WAL buffers were full. Cumulative counter.
- `wal_write`: `wal_write`, WAL write calls. Cumulative counter. NULL on PostgreSQL 18 and newer.
- `wal_sync`: `wal_sync`, WAL sync calls. Cumulative counter. NULL on PostgreSQL 18 and newer.
- `wal_write_time_ms`: `wal_write_time`. Milliseconds. Cumulative counter. NULL on PostgreSQL 18 and newer.
- `wal_sync_time_ms`: `wal_sync_time`. Milliseconds. Cumulative counter. NULL on PostgreSQL 18 and newer.
- `stats_reset`: `stats_reset`, time of the last reset, as PostgreSQL text.

## postgres_wal_position

One row per sample with the current WAL insert position. pg-probe reads `pg_wal_lsn_diff(pg_current_wal_lsn(), '0/0')` (`pg_xlog_location_diff` before PostgreSQL 10). PostgreSQL 9.2 and newer. Per sample. No row on a standby.

- `snapshot_id`: Snapshot this row belongs to.
- `sample_id`: Sample this row belongs to.
- `collected_at`: Time pg-probe wrote the batch that holds this row. UTC, RFC 3339.
- `wal_position_bytes`: WAL insert position as bytes since `0/0`. Bytes. Cumulative counter: its difference between samples is the WAL generated.

## postgres_bgwriter_stats

One row per sample from `pg_stat_bgwriter`. Per sample. Columns are cumulative counters since the last reset, except `stats_reset`. From PostgreSQL 17 the checkpoint columns move to `postgres_checkpointer_stats`.

- `snapshot_id`: Snapshot this row belongs to.
- `sample_id`: Sample this row belongs to.
- `collected_at`: Time pg-probe wrote the batch that holds this row. UTC, RFC 3339.
- `checkpoints_timed`: `checkpoints_timed`, scheduled checkpoints. Cumulative counter. NULL on PostgreSQL 17 and newer.
- `checkpoints_req`: `checkpoints_req`, requested checkpoints. Cumulative counter. NULL on PostgreSQL 17 and newer.
- `checkpoint_write_time_ms`: `checkpoint_write_time`. Milliseconds. Cumulative counter. NULL on PostgreSQL 17 and newer.
- `checkpoint_sync_time_ms`: `checkpoint_sync_time`. Milliseconds. Cumulative counter. NULL on PostgreSQL 17 and newer.
- `buffers_checkpoint`: `buffers_checkpoint`, buffers written by checkpoints. Buffers of `postgres_target.block_size_bytes`. Cumulative counter. NULL on PostgreSQL 17 and newer.
- `buffers_clean`: `buffers_clean`, buffers written by the background writer. Buffers. Cumulative counter.
- `maxwritten_clean`: `maxwritten_clean`, times the background writer stopped because it wrote too many buffers. Cumulative counter.
- `buffers_backend`: `buffers_backend`, buffers written by backends. Buffers. Cumulative counter. NULL on PostgreSQL 17 and newer.
- `buffers_backend_fsync`: `buffers_backend_fsync`, fsync calls made by backends. Cumulative counter. NULL on PostgreSQL 17 and newer.
- `buffers_alloc`: `buffers_alloc`, buffers allocated. Buffers. Cumulative counter.
- `stats_reset`: `stats_reset`, time of the last reset, as PostgreSQL text.

## postgres_checkpointer_stats

One row per sample from `pg_stat_checkpointer`. PostgreSQL 17 and newer. Per sample. Columns are cumulative counters since the last reset, except `stats_reset`.

- `snapshot_id`: Snapshot this row belongs to.
- `sample_id`: Sample this row belongs to.
- `collected_at`: Time pg-probe wrote the batch that holds this row. UTC, RFC 3339.
- `num_timed`: `num_timed`, scheduled checkpoints. Cumulative counter.
- `num_requested`: `num_requested`, requested checkpoints. Cumulative counter.
- `write_time_ms`: `write_time`, time spent writing checkpoint files. Milliseconds. Cumulative counter.
- `sync_time_ms`: `sync_time`, time spent syncing checkpoint files. Milliseconds. Cumulative counter.
- `buffers_written`: `buffers_written`, buffers written by checkpoints. Buffers of `postgres_target.block_size_bytes`. Cumulative counter.
- `stats_reset`: `stats_reset`, time of the last reset, as PostgreSQL text.

## postgres_archiver_stats

One row per sample from `pg_stat_archiver`. PostgreSQL 9.4 and newer. Per sample.

- `snapshot_id`: Snapshot this row belongs to.
- `sample_id`: Sample this row belongs to.
- `collected_at`: Time pg-probe wrote the batch that holds this row. UTC, RFC 3339.
- `archived_count`: `archived_count`, WAL files archived. Cumulative counter.
- `last_archived_wal`: `last_archived_wal`, name of the last WAL file archived. NULL when none.
- `last_archived_time`: `last_archived_time`, as PostgreSQL text. NULL when none.
- `failed_count`: `failed_count`, failed archive attempts. Cumulative counter.
- `last_failed_wal`: `last_failed_wal`, name of the WAL file of the last failure. NULL when none.
- `last_failed_time`: `last_failed_time`, as PostgreSQL text. NULL when none.
- `stats_reset`: `stats_reset`, time of the last reset, as PostgreSQL text.

## postgres_io_stats

One row per backend type, object and context in each sample, from `pg_stat_io`. PostgreSQL 16 and newer. Per sample. Counters are cumulative since the last reset. pg-probe writes 0 for a counter that PostgreSQL reports as NULL.

- `snapshot_id`: Snapshot this row belongs to.
- `sample_id`: Sample this row belongs to.
- `collected_at`: Time pg-probe wrote the batch that holds this row. UTC, RFC 3339.
- `backend_type`: `pg_stat_io.backend_type`, with spaces replaced by underscores, such as `client_backend`.
- `io_object`: `pg_stat_io.object`, such as `relation` or `temp relation`.
- `io_context`: `pg_stat_io.context`, such as `normal`, `vacuum`, `bulkread` or `bulkwrite`.
- `reads`: `reads`, read operations. Cumulative counter.
- `read_time_ms`: `read_time`. Milliseconds. Cumulative counter. Stays 0 unless `track_io_timing` is on.
- `writes`: `writes`, write operations. Cumulative counter.
- `write_time_ms`: `write_time`. Milliseconds. Cumulative counter. Stays 0 unless `track_io_timing` is on.
- `extends`: `extends`, relation extend operations. Cumulative counter.
- `extend_time_ms`: `extend_time`. Milliseconds. Cumulative counter. Stays 0 unless `track_io_timing` is on.
- `op_bytes`: On PostgreSQL 16 and 17, `pg_stat_io.op_bytes`, the bytes per read, write or extend operation. On PostgreSQL 18 and newer, the sum of `read_bytes`, `write_bytes` and `extend_bytes`, a cumulative byte counter. Bytes.
- `hits`: `hits`, times a block was found in a buffer. Cumulative counter.
- `evictions`: `evictions`, times a block was evicted from a buffer. Cumulative counter.
- `reuses`: `reuses`, times a ring buffer was reused. Cumulative counter.
- `fsyncs`: `fsyncs`, fsync calls. Cumulative counter.
- `stats_reset`: `stats_reset`, time of the last reset, as PostgreSQL text.

## postgres_slru_stats

One row per SLRU cache in each sample, from `pg_stat_slru`. PostgreSQL 13 and newer. Per sample. Counters are cumulative since the last reset.

- `snapshot_id`: Snapshot this row belongs to.
- `sample_id`: Sample this row belongs to.
- `collected_at`: Time pg-probe wrote the batch that holds this row. UTC, RFC 3339.
- `name`: `pg_stat_slru.name`, the SLRU cache, such as `Xact`, `Subtrans`, `MultiXactMember` or `Notify`.
- `blocks_zeroed`: `blks_zeroed`, blocks zeroed during initialization. Cumulative counter.
- `blocks_hit`: `blks_hit`, blocks found in the cache. Cumulative counter.
- `blocks_read`: `blks_read`, blocks read from disk. Cumulative counter.
- `blocks_written`: `blks_written`, blocks written to disk. Cumulative counter.
- `blocks_exists`: `blks_exists`, blocks checked for existence. Cumulative counter.
- `flush_count`: `flushes`, flushes of dirty data. Cumulative counter.
- `stats_reset`: `stats_reset`, time of the last reset, as PostgreSQL text.

## postgres_activity_aggregates

One row per group of backends in each sample. pg-probe reads `pg_stat_activity`, groups by database, backend type, state and wait event type, and keeps no PIDs, users, addresses or query text. Per sample. Counts and ages are gauges.

- `snapshot_id`: Snapshot this row belongs to.
- `sample_id`: Sample this row belongs to.
- `collected_at`: Time pg-probe wrote the batch that holds this row. UTC, RFC 3339.
- `database_name`: `pg_stat_activity.datname`. NULL for background processes without a database.
- `backend_type`: `pg_stat_activity.backend_type`, with spaces replaced by underscores, such as `client_backend` or `autovacuum_worker`. Always `client_backend` before PostgreSQL 10.
- `state`: `pg_stat_activity.state`, such as `active`, `idle` or `idle in transaction`. NULL for most background processes.
- `wait_event_type`: `pg_stat_activity.wait_event_type`, such as `Lock`, `LWLock` or `IO`. NULL when the backend reports no wait, and always NULL before PostgreSQL 9.6.
- `count`: Backends in this group. Gauge.
- `max_age_seconds`: Largest `now() - state_change` in the group, the longest time a backend stayed in its current state. Seconds. Gauge. 0 when no backend in the group has a `state_change`.

## postgres_lock_aggregates

One row per group of locks in each sample. pg-probe reads `pg_locks`, groups by mode, lock type, granted and fast path, and keeps no PIDs, object IDs or advisory keys. Per sample. Counts and ages are gauges.

- `snapshot_id`: Snapshot this row belongs to.
- `sample_id`: Sample this row belongs to.
- `collected_at`: Time pg-probe wrote the batch that holds this row. UTC, RFC 3339.
- `lock_mode`: `pg_locks.mode`, such as `AccessShareLock` or `RowExclusiveLock`.
- `lock_type`: `pg_locks.locktype`, such as `relation`, `transactionid` or `virtualxid`.
- `granted`: 1 for locks held, 0 for lock requests that wait.
- `fast_path`: 1 when `pg_locks.fastpath` is true, else 0.
- `count`: Locks in this group. Gauge.
- `max_wait_age_seconds`: Largest `now() - pg_locks.waitstart` in the group. Seconds. Gauge. 0 for groups where no lock has a `waitstart`, such as granted locks. NULL before PostgreSQL 14.

## postgres_replication_aggregates

One row per group of connected standbys in each sample. pg-probe reads `pg_stat_replication`, groups by state and sync state, and keeps no addresses, users or LSNs. Per sample. No rows when no standby is connected.

- `snapshot_id`: Snapshot this row belongs to.
- `sample_id`: Sample this row belongs to.
- `collected_at`: Time pg-probe wrote the batch that holds this row. UTC, RFC 3339.
- `state`: `pg_stat_replication.state`, such as `streaming` or `catchup`.
- `sync_state`: `pg_stat_replication.sync_state`, such as `async`, `sync`, `potential` or `quorum`.
- `count`: Standbys in this group. Gauge.
- `max_lag_bytes`: Largest `pg_wal_lsn_diff(pg_current_wal_lsn(), replay_lsn)` in the group, WAL not yet replayed on the standby. Bytes. Gauge. 0 when no standby in the group reports `replay_lsn`.

## postgres_slot_aggregates

One row per group of replication slots in each sample. pg-probe reads `pg_replication_slots`, groups by slot type and active flag, and keeps no slot names. PostgreSQL 9.4 and newer. Per sample.

- `snapshot_id`: Snapshot this row belongs to.
- `sample_id`: Sample this row belongs to.
- `collected_at`: Time pg-probe wrote the batch that holds this row. UTC, RFC 3339.
- `slot_type`: `pg_replication_slots.slot_type`: `physical` or `logical`.
- `active`: 1 when `pg_replication_slots.active` is true, else 0.
- `count`: Slots in this group. Gauge.
- `max_retained_bytes`: Largest `pg_wal_lsn_diff(pg_current_wal_lsn(), restart_lsn)` in the group, WAL kept for the slot. Bytes. Gauge. 0 when no slot in the group has a `restart_lsn`.
