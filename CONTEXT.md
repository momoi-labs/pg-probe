# Domain Glossary

## Snapshot

A single point-in-time collection of Linux host observations and, when
available, PostgreSQL observations.

## Collector host

The Linux host or container where pg-probe runs. All operating-system
observations describe this host, even when the PostgreSQL target is remote.

## PostgreSQL target

The PostgreSQL server selected through explicit connection information or the
standard libpq connection environment and defaults.

## Initial database

The database used to establish the first connection to a PostgreSQL target. It
does not limit the snapshot scope; collectors may connect to other databases on
the same target when an observation is database-local.

## Partial snapshot

A snapshot in which one collection area failed while another produced usable
observations. A partial snapshot remains available for diagnosis and the run is
reported as unsuccessful.

## Diagnostic artifact

The portable result of a snapshot. It contains allowlisted observations,
collection metadata, and structured failures, but never credentials, query
text, table data, or other prohibited sensitive values.
