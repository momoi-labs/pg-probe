---
"pg-probe": minor
---

A capture with `--duration` now records the wraparound state over time. Each
sample has the size of the multixact member space, the databases ranked by
oldest xid and oldest multixact, and the vacuums in progress. The tables with
the oldest xid and the oldest multixact, in the oldest databases you can
connect to, are read on the first sample and then at most once every 60
seconds, with their `autovacuum_count`. A table that shows a wraparound
autovacuum again and again while its count stays the same has autovacuums that
do not finish. This works from PostgreSQL 9.3, where the multixact order comes
from a rank because there is no multixact age. Reading the member space needs
a superuser, or from 11 EXECUTE on `pg_ls_dir`. The artifact schema is now
version 4, and it also records the four freeze age settings.
