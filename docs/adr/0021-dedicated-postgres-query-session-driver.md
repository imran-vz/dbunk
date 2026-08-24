# ADR-0021: Dedicated PostgreSQL query-session driver

Status: Accepted

## Decision

Query tabs use a dedicated tokio-postgres 0.7.18 actor connection. SQLx remains
canonical for metadata, mutations, administration, and the legacy `run_query`
path. SQLx cannot expose pre-row zero-row RowDescription, structured notices,
or a protocol CancelToken. tokio-postgres exposes those facilities; neither
driver exposes the ReadyForQuery transaction-status byte, and CommandComplete
contains a count but no command tag.

Transaction truth therefore comes from one dedicated short-query observer per
stored Connection. It coalesces work, compares PID plus backend start, and adds
one roundtrip after execution, cancellation recovery, Commit, and Rollback. It
never consumes a slot from the five-connection SQLx pool.

Every session belongs to an opaque renderer owner and the injected Tauri
window label. Owner replacement, window destruction, app exit, and connection
invalidation close it. Focused time drives the 120-second owner and ACK leases:
unfocused windows do not expire, and refocus grants a fresh deadline.

Streaming uses cumulative ACKs with a four-batch and 4 MiB serialized credit
window. Retained rows, result sets, notices, and metadata are bounded; the
driver necessarily decodes one transient DataRow before those limits apply.
Admission permits seven sessions plus one observer per Connection, eight
Connections with sessions, and 24 sessions app-wide. Live sessions are never
evicted.

The connector shares resolved routing, connect deadline, ordered driver
options, and TLS Disable/Prefer semantics with SQLx. Prefer accepts the existing
permissive certificate policy and falls back to plaintext only on TLS refusal.
*(Superseded by ADR-0025: TLS now comes from one shared resolver with
`require` / `verify-ca` / `verify-full`, client certificates, tunnel-aware
hostname verification, and applied keepalive.)*
Commands reject through a typed union and never log SQL, row values, notice
payloads, credentials, or structured database detail.

The backend is registered dark; Plan 002 owns activation. Rollback removes the
manager and new command registrations while the unchanged `run_query` path
continues to operate.

## Implementation notes

The macOS manual focus characterization is operator-run: minimized and
occluded windows must remain unfocused without expiry, and refocus grants the
full 120-second recovery allowance.
