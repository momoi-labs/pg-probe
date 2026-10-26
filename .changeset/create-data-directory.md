---
"pg-probe": minor
---

`pg-probe capture` creates a missing data directory, private to its owner, as
`pg_basebackup -D` does, instead of failing.
