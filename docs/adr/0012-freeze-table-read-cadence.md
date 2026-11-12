---
status: accepted
date: 2026-11-11
supersedes: []
superseded_by: null
tags: [collection, postgresql, wraparound]
---

# The freeze table read runs at most once every 60 seconds

The capture records wraparound state over time: the multixact member space,
the freeze ranking of the databases, the running vacuums, and the freeze state
and autovacuum counters of the tables in the oldest database. The first three
are read on the kept sampler connection on every sample. The table read is
different. It must run inside the oldest connectable database, which is often
not the connected one, so the sampler closes its connection, connects to the
target, reads, and connects back
([ADR-0002](0002-sequential-collection-single-connection.md)). That is two or
three backend forks per read. On 9.3 the read also makes the stats collector
write a fresh stats file for the database.

We decided that the table read runs on the first sample and then at most once
every 60 seconds, whatever `--interval` is. The other wraparound reads stay on
every sample. Paying the swap and the stats refresh every 10 seconds on a
server that is already in trouble is the kind of load
[ADR-0001](0001-non-invasiveness-as-governing-constraint.md) forbids.

## Considered options

- **Read the tables on every sample.** Rejected: it gives the finest series of
  `autovacuum_count`, but at two or three connects and one or two stats
  refreshes per sample. A longer wait for the signal is acceptable, a heavier
  capture is not.
- **A second connection kept open on the oldest database.** Rejected: it breaks
  ADR-0002.
- **At most once every 60 seconds.** Chosen. The cadence is the same whether a
  swap is needed or not, so every capture has the same series and the stats
  refresh cost is bounded too.

## Consequences

- The counter series per table has one point per minute. A failed autovacuum
  shows as a try seen in the per-sample vacuum read whose `autovacuum_count`
  has not risen by the next table read, up to 60 seconds later.
- A capture shorter than 60 seconds has one table read.
- With `--interval` above 60 seconds, the table read runs on every sample.
- The 60 seconds are not configurable. A flag is a candidate if measurement on
  a real server shows the cost is lower than estimated.
