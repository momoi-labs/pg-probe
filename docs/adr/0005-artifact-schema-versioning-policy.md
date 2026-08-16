---
status: accepted
date: 2026-08-16
supersedes: []
superseded_by: null
tags: [schema, artifact, compatibility]
---

# Artifact schema versioning and compatibility policy

The artifact outlives the process that produced it: a consultant may open a
`.sqlite.gz` captured months earlier, and a separate reporting tool will read
these tables as its input. We decided that breaking schema changes require a
new major version, additive changes are minor versions, and every new major
must ship either a conversion mechanism for artifacts of the previous major
(preferred) or the ability to read it directly. Compatibility flows in one
direction only — a newer reader supports older artifacts, never the reverse.

Views are the intended stable surface for consumers, so tables can be
reorganized while view output stays put.

## Consequences

- Before 1.0 the schema is unstable and these rules do not bind, but the
  version in the SQLite `user_version` pragma must always be accurate.
- The reporting epic should consume views rather than tables wherever a view
  can answer the question. Only one of the four planned views
  (`database_inventory`) currently exists.
- Readers must tolerate unrecognized columns, since additive changes land in
  minor versions.

## Open question

Whether the conversion mechanism lives in pg-probe (for example
`pg-probe --upgrade-artifact`) or in the reporting consumer is not decided
here. pg-probe never reads artifacts it did not just write, which argues for
the consumer, but the producer owns the schema knowledge. Decide when the
reporting epic starts; nothing before 1.0 depends on it.
