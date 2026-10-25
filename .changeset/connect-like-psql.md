---
"pg-probe": minor
---

pg-probe connects to a server whose TLS certificate is self-signed, as on
Debian and Ubuntu, instead of failing. TLS follows libpq's `sslmode`, from
`PGSSLMODE` or the conninfo string. The default, `prefer`, does not check the
certificate and retries without TLS when the handshake fails.

Without a host, pg-probe connects through the server's socket in
`/var/run/postgresql` or `/tmp`, as psql does. `-h` also takes the socket file
itself. A failed connection now says why, and `pg-probe top --verbose` prints
each step of the connection.
