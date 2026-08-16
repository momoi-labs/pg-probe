# Engineering Principles

These principles govern *how* pg-probe is built. They are not a specification
of *what* it does — that lives in the GitHub issues — and not a glossary of
domain terms — that lives in `CONTEXT.md`.

Use them when an implementation choice has more than one correct answer. When
a principle and a convenience conflict, the principle wins. When two principles
conflict, the order below is the tie-breaker.

## 1. Non-invasiveness above speed

pg-probe runs against systems that are usually already in trouble. A
diagnostic tool that competes for the resources it is measuring corrupts its
own measurement and, worse, becomes a tool nobody is allowed to run in
production. **A slower collection is always preferable to a heavier one.**

When choosing between two correct implementations, prefer the one that:

- holds fewer resources on the observed server, and holds them for less time;
- opens fewer connections, and never more than one at a time;
- reads from the cheapest source that answers the question — catalog lookups
  over filesystem walks, pre-aggregated kernel counters over raw enumeration;
- allocates less, or frees earlier, when memory is the scarce resource;
- spends memory to save CPU when CPU is the scarce resource, and the reverse
  when it is not;
- streams results to the sink instead of accumulating them in memory;
- fails fast and bounded rather than waiting indefinitely.

Cost that cannot be avoided must be made **opt-in and visible**, never a
silent default. An expensive observation the operator did not ask for is a
defect, even when the data is useful.

Taking longer is acceptable. Making the observed server slower is not.

## 2. Evidence, not diagnosis

pg-probe collects and records. It does not interpret, score, rank, warn, or
recommend. The consultant reading the artifact performs the diagnosis; turning
observations into analysis is the job of a separate consumer.

Practically: no thresholds, no "healthy/unhealthy" columns, no derived
judgments in the artifact. Derived *values* with an explicit unit and a stated
formula are fine; derived *opinions* are not.

## 3. The artifact schema is a compatibility contract

The artifact outlives the process that wrote it. A consultant may open a file
captured months earlier, so schema changes are governed rather than free.

- **Breaking changes require a new major version.** Removing a table or
  column, renaming one, changing a type, or changing the meaning or unit of an
  existing value all count as breaking.
- **Additive changes are minor versions.** New tables, new columns, and new
  views may land in a minor version. Readers must tolerate columns they do not
  recognize.
- **Every new major version must ship one of two things**, in order of
  preference:
  1. a conversion mechanism that upgrades artifacts from the previous major, or
  2. the ability to read the previous major directly.
- **Compatibility only ever flows in one direction:** a newer reader supports
  older artifacts. An older reader is never expected to understand a newer
  artifact.
- Before 1.0 the schema is unstable and these rules do not bind, but the
  version recorded in the SQLite `user_version` pragma must always be accurate.

Views are the intended stable surface for consumers. Prefer changing tables and
keeping view output stable over the reverse.
