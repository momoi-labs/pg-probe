# How the console computes its values

The review console (`pg-probe analyze`) shows facts from the artifact and
values derived from them. This page states where each value comes from and
how it is derived, so the screens can stay free of formulas.

## Conventions

- **Samples.** A capture with `--duration` takes one sample per `--interval`.
  Gauges, such as memory or load, use the value of each sample.
- **Rates.** Cumulative counters become a rate per interval: the change since
  the previous sample divided by the seconds between the two samples. The first
  sample has no rate.
- **Resets.** A counter that goes down between two samples was reset, by a
  restart or a statistics reset. That interval is a gap, never a negative value
  or a zero.
- **Read once.** Settings, database sizes, mounts, relations and host identity
  are read in the first sample only. Their panels show that one reading.
- **Time.** Every time is UTC.
- **Not supported.** A panel whose source the observed server or host does
  not have, such as a view added in a later PostgreSQL release, shows "Not
  supported on PostgreSQL 9.3" or "Not supported on this host" with the reason.
  "Not recorded in this artifact" means the source exists but the artifact
  holds no value for it.

## Overview tiles

Each tile shows the last sample, or the last interval for a rate, and the
highest value of the session with its time. Cache hit ratio shows the lowest
value instead, because a drop is the notable direction. A tile links to the
panel below that has the full chart.

| Tile | Value |
| --- | --- |
| CPU busy | Sum of the CPU utilization states in the last interval |
| Memory used | (MemTotal − MemAvailable) ÷ MemTotal |
| Disk busy | Busy time of the busiest whole disk in the last interval |
| Load, 1 minute | `load_avg_1m` |
| Connections | Client backends in `pg_stat_activity` |
| Cache hit ratio | As the Cache hit ratio panel, last interval. With one sample, `blks_hit ÷ (blks_hit + blks_read)` since the last statistics reset |
| WAL generated | As in WAL and buffer writes, last interval |
| Replica lag | Largest `max_lag_bytes` across standbys |

## System

| Panel | Source | How it is derived |
| --- | --- | --- |
| CPU utilization by state | `/proc/stat`, the all-CPU line | Change in each state's ticks ÷ change in all ticks. This is the mean across CPUs, not the sum. Guest time is already counted in user time and stays out of the total. "Nice, IRQ, softirq" groups those three states |
| Load average | `/proc/loadavg` | As reported by the kernel |
| Memory by use | `/proc/meminfo` | Used = MemTotal − MemFree − Buffers − Cached − SReclaimable. Page cache = Cached − Shmem + SReclaimable, because Cached already includes Shmem. Shared memory is Shmem, which includes `shared_buffers` |
| Swap used | `/proc/meminfo` | SwapTotal − SwapFree |
| Disk throughput | `/proc/diskstats` | Change in sectors read and written × 512 bytes ÷ seconds, summed over whole disks. 512 is the kernel's sector unit, not the device block size. Partitions are left out because they repeat their disk's I/O |
| Disk busy time | `/proc/diskstats` | Change in `io_milliseconds` ÷ milliseconds between samples, per whole disk, the five busiest. On SSDs and NVMe with deep queues this nears 100% before the device is saturated |
| Network throughput | `/proc/net/dev` | Change in bytes received and sent ÷ seconds, summed over the recorded interfaces. Loopback and the virtual links of containers and bridges are not recorded |
| Pressure stall | `/proc/pressure/{cpu,memory,io}` | `avg10` of the "some" line, as reported by the kernel. Absent on older kernels and in some containers |
| Filesystem usage | `statvfs` on each mount | Used = total − available. Available excludes space reserved for root, so used includes it. Only local disk filesystems are recorded, and none under `/home` |
| Processes running and blocked | `/proc/stat` | `procs_running` and `procs_blocked` |
| Swapped in and out | `/proc/vmstat` | Change in `pswpin` and `pswpout` × 4 KiB page ÷ seconds |
| Major page faults | `/proc/vmstat` | Change in `pgmajfault` ÷ seconds |
| Pages scanned for reclaim | `/proc/vmstat` | Change in the sum of `pgscan_kswapd_*` and of `pgscan_direct_*` over zones ÷ seconds. Direct reclaim stalls the process that asks for memory |
| Packets dropped and errors | `/proc/net/dev` | Change in `rx_dropped`, `tx_dropped`, `rx_errors` and `tx_errors` ÷ seconds, summed over the recorded interfaces |
| Average read and write latency | `/proc/diskstats` | Change in `read_milliseconds` ÷ change in `reads_completed`, and the same for writes, per whole disk. The five disks with the most completed I/O in the session. An interval without I/O is a gap |
| Average requests in flight | `/proc/diskstats` | Change in `weighted_io_milliseconds` ÷ milliseconds between samples, per whole disk, the same five disks |
| Cgroup memory and swap | the probe's cgroup | `memory.current` and `memory.swap.current` (v2) or `memory.usage_in_bytes` (v1) |
| Cgroup I/O | the probe's cgroup | Change in read and write bytes ÷ seconds, summed over devices. From `io.stat` (v2) or `blkio.io_service_bytes_recursive` (v1) |
| Cgroup limit events | the probe's cgroup | Change per interval in each counter of `cpu.stat` and `memory.events`, the first five |

## PostgreSQL

| Panel | Source | How it is derived |
| --- | --- | --- |
| Client connections by state | `pg_stat_activity` | Count of client backends per state, aggregated by the probe. Up to five states are shown. With more, the four most common are shown and the rest are grouped |
| Cache hit ratio per interval | `pg_stat_database` | Δ`blks_hit` ÷ (Δ`blks_hit` + Δ`blks_read`) between samples, all databases |
| Transactions per second | `pg_stat_database` | Change in `xact_commit` and `xact_rollback` ÷ seconds, all databases |
| Temporary files written | `pg_stat_database` | Change in `temp_bytes` ÷ seconds |
| WAL and buffer writes | see the next rows | All series in MiB per second |
| ↳ WAL generated | `pg_stat_wal.wal_bytes` from PostgreSQL 14, else the WAL insert position | Change ÷ seconds. The position is `pg_current_wal_lsn()` (`pg_current_xlog_location()` before 10) in bytes since 0/0. A standby has no insert position |
| ↳ Checkpoint writes | `pg_stat_bgwriter.buffers_checkpoint`, or `pg_stat_checkpointer.buffers_written` from 17 | Change in buffers ÷ seconds × block size |
| ↳ Background writer | `pg_stat_bgwriter.buffers_clean` | Change in buffers ÷ seconds × block size |
| ↳ Backend writes | `pg_stat_bgwriter.buffers_backend`, before 17 | Change in buffers ÷ seconds × block size |
| Replication lag | `pg_stat_replication` | Last sample: standbys per state and sync state, with the largest lag in bytes |
| Database size | `pg_database_size()` | Read once |
| Largest tables | `pg_class` in each database | (`relpages` of the table + its indexes + its TOAST table) × block size. `relpages` is an estimate updated by ANALYZE and VACUUM, chosen over `pg_total_relation_size()`, which reads every file. Read once |
| Dead tuples | `pg_stat_user_tables` in each database | `n_dead_tup` and `n_live_tup`, estimates kept by the statistics collector. Dead (%) = dead ÷ (dead + live). Read once |
| Lock requests waiting | `pg_locks` | Count of locks not granted, per lock type, aggregated by the probe |
| Longest lock wait | `pg_locks.waitstart`, from PostgreSQL 14 | Longest wait among locks not granted, in seconds |
| Locks held | `pg_locks` | Count of granted locks per mode. Up to five modes, as in Client connections by state |
| Longest time in state | `pg_stat_activity` | Longest `now() − state_change` among client backends, per state, for active and idle in transaction |
| Active backends by wait event type | `pg_stat_activity.wait_event_type`, from 9.6 | Count of active client backends per wait event type. No wait event is shown as "Not waiting" |
| Deadlocks, recovery conflicts and checksum failures | `pg_stat_database` | Change per interval in `deadlocks`, `conflicts` and, from 12, `checksum_failures`, all databases |
| Sessions ended abnormally | `pg_stat_database`, from 14 | Change per interval in `sessions_abandoned`, `sessions_fatal` and `sessions_killed` |
| Time reading and writing blocks | `pg_stat_database` | Change in `blk_read_time` and `blk_write_time` ÷ seconds, in ms per second. Zero unless `track_io_timing` is on |
| Queries canceled by recovery conflicts | `pg_stat_database_conflicts` | Change per interval in each `confl_*` counter. Zero on a primary |
| WAL files archived | `pg_stat_archiver`, from 9.4 | Change per interval in `archived_count` and `failed_count` |
| WAL retained by replication slots | `pg_replication_slots`, from 9.4 | Largest retained WAL per slot type, aggregated by the probe |
| SLRU blocks read | `pg_stat_slru`, from 13 | Change in `blks_read` ÷ seconds per SLRU. Up to five, as in Client connections by state |
| I/O by operation | `pg_stat_io`, from 16 | Change in reads, writes and extends × `op_bytes` ÷ seconds, all backends and objects |

The probe reads the 20 largest relations and the 20 with the most dead tuples
in each database.

## Queries

| Panel | Source | How it is derived |
| --- | --- | --- |
| Rows per second | `pg_stat_database` | Read = change in `tup_returned`. Written = change in `tup_inserted + tup_updated + tup_deleted`. Per database, not per statement |
