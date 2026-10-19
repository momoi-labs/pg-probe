---
"pg-probe": minor
---

`pg-probe capture` shows its progress by default. On a terminal, a summary
redraws in place: samples taken, collector counts per domain, sample time p95
and p99, pg-probe's own CPU, peak memory and disk writes, and the errors and
unsupported sources so far. `--refresh` sets how often it redraws, half the
`--interval` by default and at least 1s. Off a terminal, it appends a line only
when something changes, plus one progress line per minute. `--verbose` still
prints one line per collector outcome.
