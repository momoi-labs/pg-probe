// What each panel means for a reader of the printed report, by panel id.
// shows: what is measured. look: what a problem looks like.
export type Explain = { shows: string; look: string }

export const explain: Record<string, Explain> = {
  'cpu-utilization': {
    shows:
      'Share of CPU time in each state, averaged over all CPUs. User is time in programs such as PostgreSQL backends, and System is time in the kernel. I/O wait is idle time while a disk request was pending. Steal is time the hypervisor gave to other virtual machines.',
    look: 'Add User and System to get busy time. When busy time stays above 80% for minutes, check Load average. Load above the CPU count at the same time can mean queries waited for a core, but Linux load also counts tasks waiting on disk. A p95 near 100% with a low p50 in the table means short bursts, not a constant shortage. I/O wait above 10% means CPUs sat idle while disk requests were pending, so check the latency charts. Steal above a few percent means the instance shares its CPUs with busy neighbors.',
  },
  'load-average': {
    shows:
      'Average number of tasks running or waiting to run, over the last 1, 5 and 15 minutes. On Linux the count also includes tasks waiting on disk.',
    look: 'Compare the load with the number of CPUs on the Load tile of the Overview. Load above the CPU count means tasks waited for a core or a disk. A 1 minute line far above the 15 minute line is a spike that just started. High load with low CPU busy time means the tasks wait on disk, not on CPU.',
  },
  memory: {
    shows:
      'How the host used its RAM. Used is memory held by programs. Shared memory includes the PostgreSQL shared_buffers. Page cache is file data the kernel keeps for fast reads, and Buffers is filesystem metadata. Free is memory nobody uses. With huge pages, shared_buffers counts as Used instead.',
    look: 'Low Free alone is normal, because Linux fills spare memory with page cache. The warning sign is a page cache that shrinks while Used grows. PostgreSQL then finds less of its data in memory and reads more from disk. Read this chart with Cache hit ratio and Swap used.',
  },
  'swap-in-use': {
    shows: 'Swap space in use on the host.',
    look: 'A flat line is usually old data the kernel moved out long ago, and it does no harm. A line that rises during the session means the host ran short of memory while the capture ran. Swapped in and out shows whether the host reads that data back, which is what slows queries.',
  },
  'disk-throughput': {
    shows: 'Bytes read from and written to all disks per second.',
    look: 'A line that stays flat at the same top value for minutes is often a limit of the device or the cloud volume. A read spike that lines up with a drop in Cache hit ratio means queries read data that was not in memory. Write spikes every few minutes are often checkpoints. Compare with Disk busy time and the latency charts.',
  },
  'disk-utilization': {
    shows: 'Share of time each of the five busiest disks had at least one request in flight.',
    look: 'On a single spinning disk, values near 100% mean the disk is saturated. SSDs, RAID arrays and cloud volumes serve many requests at once, so they can show 100% and still have room. For those, trust read and write latency more than this chart.',
  },
  'pressure-stall': {
    shows:
      'Share of time at least one task stalled waiting for CPU, memory or I/O, as a 10 second exponential moving average. Linux 4.20 and newer report it in /proc/pressure. RHEL 8 needs the psi=1 boot option.',
    look: 'Values near zero are healthy. A value that stays above 10% shows which resource held work back. Memory pressure above zero means tasks waited while the kernel freed memory. I/O pressure usually lines up with high disk latency.',
  },
  'filesystem-usage': {
    shows: 'Space used on each local filesystem at capture time.',
    look: 'Above 85% on the data or WAL filesystem leaves little room for growth, temporary files and bursts of WAL. If the WAL filesystem fills up, PostgreSQL shuts down.',
  },
  'network-throughput': {
    shows: 'Bytes received and sent per second, summed over all interfaces.',
    look: 'A flat top near the link speed means the network is the limit. A 1 Gbit/s link tops out at about 119 MiB/s. Sudden spikes often match backups, a standby catching up or queries that return large results. Sent traffic on a primary grows with each standby.',
  },
  processes: {
    shows: 'Processes running on a CPU, and processes blocked, which on Linux mostly means waiting on disk.',
    look: 'Running above the CPU count means tasks waited for a core. Blocked above zero for several samples in a row means tasks waited on storage. Compare with I/O wait in CPU utilization and with the disk latency charts.',
  },
  'swap-activity': {
    shows:
      'Data the kernel moved from RAM to swap, and back from swap to RAM, per second.',
    look: 'Swapped in is the line that hurts, because a process stops until each page comes back from disk. Any steady swap in during the session slows PostgreSQL. Swapped out alone means the kernel is making room, often for page cache.',
  },
  'major-page-faults': {
    shows: 'Page faults per second that needed a disk read, because the page was not in memory.',
    look: 'A low, steady rate is normal when programs start or read memory mapped files. Spikes that line up with swap in or with a shrinking page cache mean the host was short of memory.',
  },
  'page-reclaim': {
    shows:
      'Pages the kernel scanned per second to free memory. kswapd scans in the background. Direct reclaim happens when a process needs memory and has to free some itself before it can go on.',
    look: 'Some kswapd activity is normal on a busy host. Direct reclaim above zero means processes, PostgreSQL backends included, stalled while they asked for memory. That causes latency spikes even when swap stays flat.',
  },
  'network-drops-and-errors': {
    shows: 'Packets dropped and interface errors per second, received and sent, over all interfaces.',
    look: 'A low, steady rate of received drops can be frames the host does not handle, such as unknown protocols. Drops that rise with traffic can mean the interface buffers overflow under load. Errors point to a cable, driver or network card problem. A dropped TCP packet must be sent again, which slows clients and standbys.',
  },
  'disk-read-latency': {
    shows: 'Average time a read request took on each of the five busiest disks, in milliseconds.',
    look: 'Local SSDs usually answer in under 1 ms, cloud volumes in 1 to 5 ms and spinning disks in 5 to 15 ms. Values in the tens of ms that last mean the disk was overloaded or throttled. The p95 in the table shows how slow the worst intervals were.',
  },
  'disk-write-latency': {
    shows: 'Average time a write request took on each of the five busiest disks, in milliseconds.',
    look: 'With synchronous_commit on, each commit waits until its WAL reaches the disk. High write latency on the WAL disk therefore slows every transaction. On SSD or cloud storage, writes above 10 ms make commits slow enough for users to notice. Spikes every few minutes often match checkpoints.',
  },
  'disk-queue-depth': {
    shows: 'Average number of requests in flight on each of the five busiest disks.',
    look: 'A queue that grows together with latency means the disk could not keep up. A deep queue with low latency is fine, because SSDs and cloud volumes work best with many requests at once.',
  },
  'cgroup-memory': {
    shows:
      'Memory and swap charged to the cgroup of the probe. When the probe runs in the PostgreSQL container or systemd unit, this is the memory of that container.',
    look: 'Near the limit, the kernel reclaims memory inside the cgroup. At the limit it reclaims harder, and it kills a process only if reclaim cannot free enough. Page cache counts against the limit too, so a cgroup can look full while most of it is cache.',
  },
  'cgroup-i-o': {
    shows: 'Bytes read and written per second by the cgroup of the probe, all devices.',
    look: 'A flat top can be an I/O limit set on the container. If Disk throughput for the whole host has room while this line is flat, the limit is on the container, not on the disk. On cgroup v1, the kernel does not charge buffered writes to the container, so this line misses most writes.',
  },
  'cgroup-events': {
    shows:
      'Change per interval of the limit counters of the cgroup of the probe, named controller.counter: cpu.nr_throttled, memory.high, memory.max, memory.oom_kill and pids.max. On cgroup v1, pg-probe records only CPU usage for now, so there the chart shows it, in microseconds of CPU time.',
    look: 'cpu.nr_throttled above zero means the CPU limit of the cgroup paused its tasks. memory.high counts times the kernel throttled the cgroup and reclaimed memory at its soft limit, which never kills a process. memory.max counts times the cgroup reached its hard limit. The kernel kills a process only when reclaim fails there, and memory.oom_kill counts those kills. When a PostgreSQL backend is killed, the server ends every session and runs crash recovery. pids.max means the cgroup could not start a process, so new connections fail.',
  },
  'connections-by-state': {
    shows:
      'Client connections at each sample, by state. Active connections run a query. Idle connections wait for the client. Idle in transaction connections have an open transaction but run nothing.',
    look: 'Active connections well above the CPU count mean queries compete for cores. Idle in transaction connections that last hold locks and can keep vacuum from removing dead rows. Compare the total with max_connections on the Connections tile. Many idle connections suggest a connection pooler.',
  },
  'cache-hit-ratio': {
    shows:
      'Share of block reads PostgreSQL found in shared_buffers, per interval, over all databases. The Linux page cache can still serve a miss, so a miss is not always a disk read.',
    look: 'OLTP workloads usually stay above 99%. Drops line up with scans of large tables, reports or a cold cache after a restart. A ratio that stays low while disk reads stay high means the working set does not fit in memory.',
  },
  transactions: {
    shows: 'Transactions committed and rolled back per second, over all databases.',
    look: 'Rollbacks above a few percent of commits often point to application errors, deadlocks or timeouts. A sudden drop in commits while connections stay active can be a stall, such as lock waits or a slow disk. Compare the shape with the CPU and disk charts to see which resource the load uses most.',
  },
  'temporary-files-written': {
    shows:
      'Data written to temporary files per second. PostgreSQL writes them when a sort, hash or similar step does not fit in work_mem.',
    look: 'Spikes from a few large reports are normal. Steady temporary writes mean some queries need more work_mem, an index or a better plan. Set log_temp_files to find the queries that write them.',
  },
  'wal-and-checkpoint-writes': {
    shows:
      'WAL generated per second, and data pages written by the checkpointer, the background writer and the backends themselves.',
    look: 'Most page writes should come from the checkpointer. Backend writes mean a query had to write a dirty page before it could reuse the buffer, and it waited for that write. Up to PostgreSQL 16, backend writes that persist outside bulk loads and VACUUM point to a small shared_buffers or a background writer that falls behind. Large checkpoint spikes can saturate the disk, so compare with disk latency.',
  },
  'replication-lag': {
    shows:
      'Standbys at the last sample, from pg_stat_replication, grouped by state and sync state, with the number of standbys and the largest lag in each group.',
    look: 'This is one moment, so compare with earlier captures to see whether lag grows. Lag that grows means a standby, its disk or the network cannot keep up. With large lag, reads on the standby return old data. With asynchronous replication, a failover can lose the transactions the standby had not received yet.',
  },
  'database-size': {
    shows: 'Size on disk of each database at capture time.',
    look: 'Compare the total with the free space on the data filesystem. Compare with earlier captures to get the growth rate. Fast growth without new data can be bloat, so check Dead tuples.',
  },
  'largest-tables': {
    shows:
      'The six largest tables. Each bar is the estimated size of the table with its indexes and TOAST data, from the page counts in pg_class.',
    look: 'The page counts update at VACUUM, ANALYZE and some DDL, so a table that grew since then shows smaller than it is. A table that also appears in Dead tuples needs vacuum tuning. A full scan of these tables costs the most I/O.',
  },
  'dead-tuples': {
    shows:
      'The six tables with the most dead rows. A dead row is an old row version that an UPDATE or DELETE left behind and that vacuum has not removed yet.',
    look: 'A high share of dead rows, or a last autovacuum long ago, means autovacuum falls behind. The table then grows, and scans read more pages. Long transactions and idle in transaction sessions keep vacuum from removing rows.',
  },
  'lock-waits': {
    shows: 'Lock requests waiting at each sample, by lock type.',
    look: 'Short waits come and go on a busy server. A count that stays above zero means sessions block each other. Relation waits often come from DDL or LOCK TABLE. Transactionid waits mean rows locked by a long transaction.',
  },
  'longest-lock-wait': {
    shows: 'The longest time any request had waited for a lock at each sample, in seconds.',
    look: 'Waits of more than a few seconds block users. One long wait can build a queue, because later requests wait behind it. pg_blocking_pids() finds the session that holds the lock.',
  },
  'locks-held': {
    shows: 'Locks held at each sample, by lock mode.',
    look: 'AccessShareLock comes from plain reads and is normal. AccessExclusiveLock blocks every read and write on its table. DDL, VACUUM FULL, TRUNCATE and some ALTER TABLE commands take it. A count of any mode that stays high can mean long transactions.',
  },
  'longest-time-in-state': {
    shows:
      'For active and idle in transaction sessions, the longest time any client session had stayed in that state since its last state change, in minutes.',
    look: 'A long active time is a slow query. A long idle in transaction time holds locks and can block vacuum. From PostgreSQL 9.6, idle_in_transaction_session_timeout can end those sessions.',
  },
  'wait-events': {
    shows:
      'Active backends at each sample, by the type of wait event they reported. No wait event means PostgreSQL reported none. The backend was running on a CPU, waiting for the operating system to schedule it, or in a wait PostgreSQL does not report, such as disk I/O on 9.6, which has no IO wait type.',
    look: 'Lock waits mean sessions blocked each other. IO waits are time spent waiting for reads, writes or flushes, so compare them with the disk latency charts. LWLock waits point to contention inside PostgreSQL, for example on WAL or on the buffer mapping. Many backends with no wait event on a busy host can mean they queue for CPU.',
  },
  'deadlocks-and-conflicts': {
    shows: 'Deadlocks, recovery conflicts and checksum failures per interval, over all databases.',
    look: 'Each deadlock aborts a transaction. The fix is in the application, usually by taking locks in the same order everywhere. Recovery conflicts happen on standbys. From PostgreSQL 12, with data checksums on, any checksum failure means data on disk is corrupt. Check the server log and the storage the same day.',
  },
  'sessions-ended-abnormally': {
    shows:
      'Sessions per interval that ended without a clean disconnect. Abandoned means the client went away. Fatal means an error ended the session, timeouts such as idle_in_transaction_session_timeout included. Killed means a SIGTERM ended it, from pg_terminate_backend or a server shutdown.',
    look: 'Abandoned sessions point to network problems, client crashes or a pooler that drops connections. Fatal sessions come from errors and timeouts, and the server log has the cause of each. Killed sessions match pg_terminate_backend calls by an operator or a tool, or a server shutdown.',
  },
  'block-i-o-time': {
    shows:
      'Time backends spent reading and writing data blocks, in milliseconds per second. It needs track_io_timing on.',
    look: 'A value of 1000 means one backend, on average, spent all its time on I/O. Read time that rises while Cache hit ratio falls means the working set does not fit in memory. Compare with disk latency to tell a slow disk from too many reads.',
  },
  'standby-conflicts': {
    shows:
      'Queries on a standby that recovery canceled, per interval, by cause. A snapshot conflict happens when vacuum on the primary removed rows a standby query still needed.',
    look: 'Any value above zero means standby users saw errors. Snapshot conflicts are the most common. hot_standby_feedback or a longer max_standby_streaming_delay reduces them, at the cost of more bloat on the primary or more lag on the standby.',
  },
  'wal-archiving': {
    shows: 'WAL files archived, and archive failures, per interval, from pg_stat_archiver.',
    look: 'Any failure means the archive falls behind, and point-in-time recovery cannot go past the last archived file. PostgreSQL keeps every WAL file it could not archive, so pg_wal (pg_xlog before PostgreSQL 10) grows until the archive command works again and can fill the disk. Archived files should keep pace with the WAL generated.',
  },
  'replication-slots': {
    shows:
      'WAL that PostgreSQL keeps on disk for replication slots, as the largest amount retained by any slot of each type, physical or logical.',
    look: 'A slot keeps WAL until its consumer confirms it. A line that only grows means a consumer is slow, down or gone. That WAL can fill the disk and stop the server. From PostgreSQL 13, max_slot_wal_keep_size can cap it. Drop slots nobody uses.',
  },
  'slru-reads': {
    shows:
      'Blocks read from disk per second by the SLRU caches. These small caches hold transaction status, subtransactions, multixacts, commit timestamps, LISTEN/NOTIFY and serializable transactions.',
    look: 'Values near zero are normal. Steady subtransaction reads come from heavy use of savepoints or of exception blocks in PL/pgSQL, and they can slow the whole server. Multixact reads come from many sessions locking the same rows, for example through SELECT FOR SHARE or foreign keys.',
  },
  'i-o-by-operation': {
    shows:
      'Data read, written and extended per second by all backends, from pg_stat_io. Extends are new blocks added to the end of a table or index file.',
    look: 'Large reads point to scans of tables that do not fit in memory. Writes should follow the checkpoints and the background writer, so compare with WAL and checkpoint writes. Steady extends are normal for tables that grow.',
  },
  rows: {
    shows:
      'Rows per second over all databases. Read is rows returned by scans. Written is rows inserted, updated and deleted.',
    look: 'Read counts every row a scan returned, before filters, so a high read rate with few transactions points to sequential scans of large tables. A written rate that rises with WAL generated is normal. Compare the shape with Transactions per second.',
  },
}
