# ADR 0025: PostgreSQL TLS verification modes, one TLS resolver, and staged connection diagnosis

- **Status:** Accepted (Plan 011, `PAR-006`)
- **Date:** 2026-08-24
- **Supersedes:** the TLS sentence of ADR-0021 ("Prefer accepts the
  existing permissive certificate policy"); amends the keepalive bullet
  of ADR-0013.

## Context

Before this decision every PostgreSQL connect site made its own two-way
TLS choice from the `ssl` boolean — the SQLx pool (`prefer | disable`),
the dedicated tokio-postgres driver (the same, plus an accept-all
certificate verifier), the SQLx-Any DSN, and the `pg_dump` /
`pg_restore` environment. No path could verify a server certificate; a
connection labelled `production` with SSL on was encrypted against a
passive observer and nothing else. `keepalive_seconds` had been stored
since ADR-0013 but was applied nowhere, and Test Connection reduced every
failure to one string produced by a driver the query editor does not use.

## Decision

1. **libpq's vocabulary is the model.** A PostgreSQL connection carries
   `tls_options { mode, root_cert_path, client_cert_path,
   client_key_path, server_name }` as one JSON blob (migration 18) with
   `mode ∈ disable | prefer | require | verify-ca | verify-full`. The
   legacy `ssl` flag remains as a mirror that storage normalizes to
   `mode != disable` on every save; rows without the blob resolve
   through `ssl` exactly as before (`true → prefer`, `false → disable`).
   `PgStoredConnection::resolved_tls_mode` is the only reader.
2. **Paths, not contents.** Certificate and key material is referenced
   by path, as libpq and the bastion private-key precedent do. The
   client key never enters SQLite or the credential blob. Validation
   (exists, readable, parses, not passphrase-protected) happens at
   connect time so a record can be saved before the file exists.
3. **One resolver, four renderers.** `postgres::tls::ResolvedTls` is
   computed once and rendered per backend: a rustls `ClientConfig` for
   the dedicated driver, `PgConnectOptions` for the pool, a DSN query
   string for the SQLx-Any fallback, and `--host` + `PGSSL*` /
   `PGHOSTADDR` for libpq tools. A new mode is a change to that module.
4. **Verification semantics.** `prefer` and `require` encrypt without
   authenticating the peer (libpq semantics; the policy every
   pre-existing `ssl: true` connection already had). `verify-ca`
   validates the chain and tolerates a name mismatch; `verify-full`
   validates both. Roots are the platform trust store **∪** the user's
   CA file — union rather than replacement because SQLx cannot replace,
   and both paths must trust the same set.
5. **Hostname verification survives SSH tunnels.** The tunnel rewrites
   `host` to the loopback endpoint; it now also records the original
   host as `tls_options.server_name` on the resolved copy (never
   persisted; a user-supplied name wins). The dedicated driver connects
   with `host = server_name` + `hostaddr = loopback`, and libpq tools
   with `--host server_name` + `PGHOSTADDR`. **Known limitation:** SQLx
   has no `hostaddr`, so `verify-full` over a tunnel verifies the chain
   only on the metadata pool; the renderer downgrades that one path to
   `VerifyCa`, logs it, and the diagnosis reports
   `poolHostnameVerification: caOnly`.
6. **Keepalive applies where it can.** `keepalive_seconds` sets
   `keepalives_idle` on the dedicated driver (query sessions, table
   browse, result mutation). SQLx 0.8 still exposes no socket setter, so
   the pool path is unchanged and documented as such.
7. **TLS failures are typed.** The dedicated driver no longer collapses
   a failed handshake into `ConnectionLost`; the three actor unions gain
   `tlsFailed { tlsKind, message }` with
   `TlsFailureKind ∈ serverRefusedTls | certificateUntrusted |
   hostnameMismatch | clientCertificateRejected | invalidLocalMaterial |
   handshakeFailed`.
8. **Diagnosis is stepwise.** `diagnose_connection` runs tunnel → DNS →
   TCP → TLS → authentication → database as separate operations
   (SSLRequest and the rustls handshake are performed by hand over the
   probe socket, then tokio-postgres speaks the plain protocol over the
   resulting stream), so every failure is attributed to the stage that
   produced it. Encryption state comes from the server's `pg_stat_ssl`
   row, which is the only honest source for `prefer`. Non-PostgreSQL
   engines get the tunnel stage plus one `database` stage wrapping the
   existing ping. The credential for a saved connection can be hydrated
   backend-side so edit-mode tests never move a secret over IPC.

## Threat model

Mistakes, not adversaries: a wrong CA, a certificate for a different
host, a server that silently negotiated plaintext under `prefer`, a
client key that is really the encrypted one. The loopback leg of an SSH
tunnel is authenticated by SSH; the CA-only limitation there is
disclosed rather than hidden, not defended as equivalent.

## Consequences

- Existing connections keep their behaviour; `prefer` stays the default
  for new PostgreSQL connections so nothing breaks on upgrade.
- The SQLx feature set moves from Mozilla (`webpki`) roots to the
  platform store so the pool and the dedicated driver agree on trust;
  no existing mode verified anything, so this changes no current
  behaviour.
- The native root store loads once per process; a CA installed while
  dbunk is running needs a restart.
- The diagnosis does not use SCRAM channel binding (it hands
  tokio-postgres a pre-wrapped stream); servers that *require*
  `scram-sha-256-plus` would fail the authentication stage while the
  real driver, which negotiates TLS itself, succeeds.
- MySQL keeps its single `ssl` toggle; TLS modes there are deferred
  (`PAR-006`, PostgreSQL-first per ADR-0001).
