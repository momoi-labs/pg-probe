---
"pg-probe": minor
---

The console and the printed report have a Wraparound section. "Vacuum tries
by table" counts the vacuums seen on each table during the capture, how many
ran to prevent wraparound and how many finished, with a line that shows the
samples in which a vacuum ran. A table with many tries and none finished has
an autovacuum that starts and stops before the end. "Multixact member space"
shows how full the space is and how fast it grew. "Oldest databases" lists
the databases by xid age, with their multixact age. The section needs a
capture made with this version.
