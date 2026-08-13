# TODO

## Sensitive-data leak test harness

Build an automated canary-based harness for every collector. It must inject
prohibited values such as passwords, complete DSNs, environment values,
password-file contents, SQL text, and raw source data, then verify that the
canaries never appear in stdout, stderr, structured errors, `.sqlite.partial`,
SQLite records, or decompressed final artifacts. Choose fixtures, mocks, and
test libraries during collector implementation.

## Collector restart policy

Automatically restart a collector that panics or crashes. Before
implementation, define retry limits, backoff, idempotency, duplicate-observation
handling, and how repeated failures affect the artifact status. In the MVP, a
collector that panics remains stopped, while other collectors continue and the
artifact is finalized as partial.

## Privileged Linux metrics

The MVP must run without root privileges. Identify and add optional collectors
for Linux metrics that require root or additional capabilities. Privileged
collectors must never prevent an otherwise valid unprivileged collection run;
the output must record that those metrics were unavailable and why.

## Time-bounded collection

Add a `--duration <duration>` mode that samples Linux and PostgreSQL metrics for
a limited period before producing the final `.sqlite.gz` artifact. Define the
sampling interval, backpressure behavior, and partial-result semantics as part
of that work.

## Artifact encryption

Add password-based encryption for diagnostic artifacts. Define the file format,
key derivation, authenticated-encryption scheme, password input mechanism, and
recovery behavior before implementation. Passwords must never be accepted as a
plain command-line argument or persisted in logs or collection metadata.

## Partial artifact cleanup

Add a `--cleanup` command that safely removes abandoned `.sqlite.partial` files
from the selected data directory. Define age filtering, dry-run behavior,
concurrent-process protection, and confirmation rules before implementation.

Until cleanup is implemented, startup warnings for existing partial files must
suggest running `pg-probe --cleanup` once that command becomes available.
