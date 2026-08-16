---
status: accepted
date: 2026-08-16
supersedes: []
superseded_by: null
tags: [schema, artifact, collection]
---

# Variant observations carry a sample dimension

Issue #5 designed one artifact as one point-in-time snapshot: every table is
keyed by `snapshot_id`, and `collected_at` appears in no primary key. But
roughly thirteen of the thirty-seven tables hold cumulative counters —
`postgres_database_stats`, `postgres_wal_stats`, `postgres_io_stats`,
`linux_cpu_time`, `linux_vm_event` and others — whose absolute values are
totals since the last reset and carry almost no meaning in a single sample. A
probe whose purpose is to supply monitoring data where none exists must be
able to express a rate.

We decided to introduce the sample dimension into the schema now, while the
project is pre-1.0 and no artifact exists in the field, and to have the
default single-shot run write exactly one sample. Only **variant**
observations are keyed by sample; **invariant** observations — host identity,
CPU topology, `postgres_setting`, database inventory, extensions, mounts,
limits — stay keyed to the artifact and are collected once.

## Considered options

- **Keep one sample per artifact; let the reporting consumer correlate
  files.** Rejected: it pushes the hardest part onto every consumer, and
  joining separate `.sqlite.gz` files with unrelated snapshot identifiers is
  worse than joining rows in one table.
- **Give every table a sample dimension uniformly.** Rejected: re-reading
  `pg_settings` and a forty-database inventory on every sample is precisely
  the waste [ADR-0001](0001-non-invasiveness-as-governing-constraint.md)
  forbids.
- **Sample every table, but write a row only when the value changes.**
  Rejected: it makes the consumer perform an as-of join to learn the value in
  effect at a given sample, and simple SQL for the consultant is a goal.

## Consequences

- Timed collection (`--duration`) is not implemented by this decision, but it
  stops being a schema change and becomes a loop.
- Doing this later would mean rewriting the primary keys of every table in
  `src/artifact.rs` plus every artifact already captured. Doing it now costs
  one column.
- The distinction between variant and invariant observation becomes domain
  vocabulary and belongs in `CONTEXT.md`.
