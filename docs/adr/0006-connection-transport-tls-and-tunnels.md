# Connection transport: driver options, TLS and SSH tunnels

**Status:** Accepted (2026-05-14 to 2026-08-24, rewritten 2026-10-11)

**PostgreSQL driver options** live on the connection record as one optional
`PgDriverOptions` value (`backend/src/types.rs`): statement and idle-in-
transaction timeouts, connect timeout, keepalive, `search_path` and default
role, applied whenever a connection opens (`backend/src/postgres/connect_spec.rs`).
Connect timeout is deliberately not mapped to SQLx `acquire_timeout`.

**TLS** uses libpq's vocabulary: disable, prefer, require, verify-ca,
verify-full. Certificates are referenced by file path, not copied. One
resolver, `ResolvedTls` (`backend/src/postgres/tls.rs`), feeds every driver so
the modes mean the same thing everywhere. A supplied CA is added to the
platform trust store rather than replacing it. Connection testing is a staged
diagnosis (tunnel, DNS, TCP, TLS, authentication, database;
`backend/src/diagnosis.rs`) that names the failing stage instead of returning
one opaque error.

**SSH tunnels** go through reusable Bastion Server records with
trust-on-first-use host keys (`backend/src/tunnel.rs`,
`backend/src/tunnel/session.rs`). A tunnelled connection never falls back to
a direct connection.

## Considered options

- **Per-field columns or per-query flags for driver options.** Rejected.
- **Per-operation tunnels.** Rejected in favour of shared routes.

## Consequences

`verify-full` over an SSH tunnel checks the CA only on the SQLx paths, because
SQLx has no `hostaddr` to separate the dial address from the verified name.
