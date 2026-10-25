---
status: accepted
date: 2026-10-25
supersedes: []
superseded_by: null
tags: [postgresql, cli, security]
---

# TLS follows libpq's sslmode, and prefer does not check the certificate

Until now pg-probe offered TLS on every TCP connection and accepted only a
certificate signed by a public certificate authority, for the host's name.
PostgreSQL on Debian and Ubuntu ships with `ssl = on` and a self-signed
certificate, so pg-probe could not connect to it over TCP, while psql could.
The client also defaulted to `localhost` without a host, where psql uses the
local socket, so `pg-probe top` failed on a stock Ubuntu install with no
reason given.

## Decision

TLS follows libpq's `sslmode`, read from the conninfo string or `PGSSLMODE`.
ADR 0003 limits pg-probe to psql's options; `sslmode` is one of them, so no
new flag is needed. The client parses the conninfo string and knows only
`disable`, `prefer`, and `require`, so `verify-ca` and `verify-full` come from
`PGSSLMODE` alone. Parsing the conninfo string a second time to find them
would mean keeping a second parser in step with the client's.

- `prefer` is the default, as in libpq. It encrypts without checking the
  certificate. When the handshake fails, pg-probe warns and connects once more
  without TLS.
- `require` encrypts without checking the certificate.
- `verify-ca` and `verify-full` check against the public certificate
  authorities built into the binary. `verify-full` also checks the name.
- The client cannot tell an explicit `prefer` in the conninfo string from its
  default, so `PGSSLMODE` applies unless the string says `disable` or
  `require`.
- `allow` and `sslrootcert` are not supported.

Without a host, pg-probe uses the socket in `/var/run/postgresql` or `/tmp`
when the server's socket is there, and `localhost` otherwise.

## Considered options

- **Keep checking the certificate by default.** Rejected: it fails on the most
  common self-hosted setup, and psql does not check it either.
- **Add a `--no-ssl` flag.** Rejected: ADR 0003 keeps the command line to
  psql's options, and `sslmode` already covers it.
- **Read the system's certificate store for the verify modes.** Deferred: it
  adds a dependency, and no one has asked for it yet.

## Consequences

- Under `prefer` and `require`, a man in the middle can read the session, as
  with psql's defaults. Operators who need more set `verify-full`.
- The retry under `prefer` opens a second connection only after the first one
  failed, so pg-probe still holds at most one connection at a time. The retry
  gets only what is left of `connect_timeout`.
- The client does TLS only when it has a host name. A `hostaddr` alone also
  serves as the name. A socket directory as the host with a `hostaddr` leaves
  no name, so `require` fails there and `prefer` falls back to no TLS.
- A failed connection shows the server's or the operating system's message.
  Other driver text stays hidden, since it may echo connection input.
