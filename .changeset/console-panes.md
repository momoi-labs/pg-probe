---
"pg-probe": minor
---

The review console shows more host and PostgreSQL metrics: swap, page
faults, reclaim, network drops, disk latency, cgroup usage, locks, wait
events, conflicts, archiving, replication slots, SLRU and `pg_stat_io`.
A metric the server or host cannot provide says "Not supported on
PostgreSQL 9.3" or "on this host" and why. Panels can be moved and resized,
and the layout is kept per section. Chart legends show p50, p75, p95, p99
and the last sample. Hover marks the same time on every chart, and a click
pins it.
