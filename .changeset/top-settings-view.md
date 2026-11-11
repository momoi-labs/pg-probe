---
"pg-probe": minor
---

`pg-probe top` has a settings view on `F5` or `5`. It lists every row of `pg_settings` under its category, shows each value beside its default, marks the values that differ from the built-in default and the ones that wait for a restart, and shows beside the list the description of the selected setting, how many times a changed amount is more or less than the default, the source with its file and line, and the context. `/` searches names and categories, `d` keeps only what differs from the default, and `p` only what waits for a restart. It reads `pg_settings` once when the view opens and again on `r`. It works from PostgreSQL 9.3; the restart mark needs 9.5.
