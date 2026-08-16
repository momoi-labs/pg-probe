---
status: accepted
date: 2026-08-16
supersedes: []
superseded_by: null
tags: [architecture, collection, testing]
---

# Adopt Collector and Sink seams; reject a closed Observation enum

Issue #1 called for three seams — `Collector`, a closed typed `Observation`,
and `Sink` — and none were built: `src/lib.rs` inlines the same eight-line
block nine times, once per collector. We decided to introduce `Collector` and
`Sink` and to leave `Observation` out.

The seams are adopted for maintenance reasons alone, not performance. Rust
monomorphizes static dispatch, so the binary is unchanged. What they buy is
per-collector panic isolation (issue #11, user story 31, currently unmet), the
ability to run the sensitive-data leak tests of issue #19 collector by
collector, and a single place to invoke a collector once per sample under
[ADR-0004](0004-temporal-sample-dimension.md).

`Observation` as a closed enum is rejected: with thirty-seven already-typed
observation shapes, the enum becomes a wide match arm duplicated at every
consumer, and each new collector edits a shared type. The typed writer methods
already provide the type safety the enum was meant to add.

## Consequences

- The rejection of `Observation` is a deliberate deviation from issue #1 and
  should not be re-proposed without new evidence.
- Panic isolation moves from ad-hoc `catch_unwind` calls in
  `src/postgres/mod.rs` into the seam, so every collector gets it uniformly.
