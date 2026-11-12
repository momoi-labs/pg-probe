---
"pg-probe": minor
---

The vacuum view of `pg-probe top` now shows what should be vacuumed against
wraparound and is not. It lists the tables past `autovacuum_freeze_max_age` or
`autovacuum_multixact_freeze_max_age`, worst first, with the TOAST tables shown
under their owner. Each row has the time since the last autovacuum, the workers
seen on the table and the autovacuums that finished. A failed autovacuum never
counts as finished. A head line shows how full the multixact member space is,
and the database that holds the oldest multixact. The view reads these every
30 seconds and only while it is open. Reading the member space needs a
superuser, or from 11 EXECUTE on `pg_ls_dir`.
Before 9.5 there is no multixact age, so only the xid age counts, and the view
says so.
