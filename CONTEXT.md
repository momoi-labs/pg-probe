# pg-probe

A standalone probe that captures Linux host and PostgreSQL monitoring data
from environments that have no monitoring in place, and records it in a
portable file for later analysis.

## Language

### The probe and its run

**Probe**:
A self-contained collector that an operator runs on demand, without installed
agents, servers, or persistent infrastructure.
_Avoid_: agent, exporter, monitor

**Snapshot**:
One complete capture performed by a single run of the probe, containing one or
more samples.
_Avoid_: run, execution, capture

**Sample**:
The set of observations taken at one instant within a snapshot. A default
snapshot contains exactly one sample.
_Avoid_: tick, datapoint, scrape

**Collector host**:
The Linux host or container where the probe runs. All operating-system
observations describe this host, even when the PostgreSQL target is remote.

**PostgreSQL target**:
The PostgreSQL server selected through explicit connection information or the
standard connection environment and defaults.
_Avoid_: instance, server, endpoint

**Initial database**:
The database used to establish the first connection to a PostgreSQL target. It
does not limit the snapshot scope; collectors may connect to other databases on
the same target when an observation is database-local.

### Observations

**Collector**:
A component responsible for one symbolic source of observations. Collectors are
independent: one failing never prevents the others from producing results.

**Observation**:
A single allowlisted measurement or fact about the collector host or the
PostgreSQL target, carrying an explicit unit and the time it was taken.
_Avoid_: metric, datum, reading

**Variant observation**:
An observation whose value is expected to change within the span of a snapshot,
and which is therefore recorded once per sample. Cumulative counters and
instantaneous gauges are both variant.

**Invariant observation**:
An observation whose value is not expected to change within the span of a
snapshot, and which is therefore recorded once per snapshot. Host identity, CPU
topology, and server configuration are invariant.

**Capability**:
A feature of the PostgreSQL target, determined by its version or installed
extensions, that decides whether a given source can be observed at all.

### Results and failure

**Diagnostic artifact**:
The portable result of a snapshot. It contains allowlisted observations,
collection metadata, and structured failures, but never credentials, query
text, table data, or other prohibited sensitive values.
_Avoid_: dump, bundle, report

**Collector outcome**:
The structured record of how one collector ended. Every planned collector has
one, so a missing observation is always explainable.

**Unsupported**:
The state of a collector whose source does not exist on this target, because
of its version or missing capability. Distinct from a failure: an unsupported
source does not by itself make a snapshot partial.

**Partial snapshot**:
A snapshot in which one collection area failed while another produced usable
observations. Its artifact is published and the run is reported as
unsuccessful.

**Partial file**:
The on-disk working file of a snapshot that was never finalized, left behind by
an interrupted or failed run so that committed evidence stays recoverable.
_Avoid_: partial snapshot, temp file
