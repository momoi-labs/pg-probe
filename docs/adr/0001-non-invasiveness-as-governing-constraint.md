---
status: accepted
date: 2026-08-16
supersedes: []
superseded_by: null
tags: [principles, collection, performance]
---

# Non-invasiveness is the governing constraint

pg-probe exists to bring monitoring data out of environments that have no
monitoring, so it runs against servers that are usually already degraded. We
decided that minimizing the load placed on the observed server outranks
collection speed, feature coverage, and implementation convenience: a longer
collection is always acceptable, a heavier one never is. The operational goal
is that an operator can run pg-probe on a production primary without
hesitation.

## Consequences

- Every implementation choice with more than one correct answer is settled by
  resource cost on the observed server. The applied form of this rule lives in
  [`docs/PRINCIPLES.md`](../PRINCIPLES.md).
- Expensive observations must be opt-in and visible rather than silent
  defaults, even when the data is valuable.
- This constraint, not throughput, is why collection is sequential
  ([ADR-0002](0002-sequential-collection-single-connection.md)) and why the
  schema separates invariant from variant observations
  ([ADR-0004](0004-temporal-sample-dimension.md)).
- Numeric budgets (memory ceilings, timeouts) are deliberately not fixed here.
  They are expected to emerge from measurement; the principle binds now, the
  numbers bind when they exist.
