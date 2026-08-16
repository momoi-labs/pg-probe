---
status: accepted
date: 2026-08-16
supersedes: []
superseded_by: null
tags: [collection, orchestration, postgresql]
---

# Collection is sequential and holds one connection at a time

Issue #6 specified bounded concurrent collectors feeding a bounded channel and
a single transactional writer. We decided against concurrency: collectors run
sequentially, and at most one PostgreSQL connection is open at any moment.
Concurrency would win wall-clock time, which we do not value, at the cost of
competing with the observed server for CPU, I/O, and connection slots, which
[ADR-0001](0001-non-invasiveness-as-governing-constraint.md) forbids.

The current code is already sequential; this ADR records that the shape is
deliberate rather than an unfinished implementation of #6, so nobody "fixes"
it later.

## Considered options

- **Bounded concurrency, as issue #6 specified.** Rejected: faster collection
  is not a goal, and parallel collectors are exactly the resource contention
  the tool must avoid.
- **One connection for the entire run.** Rejected: extension inventory is
  database-local, and per-database data is genuinely useful for consultancy
  (`pg_stat_statements` present in one database and absent in another changes
  the diagnosis).
- **One connection at a time, reconnecting per database.** Chosen. Each
  connection costs a backend fork on the server, but the cost is serialized
  and bounded rather than concurrent.

## Consequences

- Issue #6's concurrency design is superseded. The single transactional writer
  from that issue is kept.
- Collection time grows roughly linearly with the number of connectable
  databases. This is accepted.
- The per-database connection cost remains a candidate for an opt-in flag if
  measurement shows it matters on instances with many databases.
