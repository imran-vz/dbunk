# ClickHouse and Redis write semantics

**Status:** Accepted (2026-05-11, rewritten 2026-10-11)

ClickHouse and Redis do not fit the relational edit model, so they keep their
own write semantics under the shared safety policy (ADR-0004).

**ClickHouse** has no unique row key, applies `ALTER … UPDATE/DELETE` as
background mutations, and has no transactions. The native app is therefore a
query and browse surface: statements run one at a time through the policy,
with `SYSTEM`, `RENAME`, `EXCHANGE`, `KILL`, `DETACH`, `REPLACE` and
data-moving `ALTER` actions classed as destructive, and external table
functions refused on read-only connections. Stop sends `KILL QUERY`. The
backend keeps capability-gated cell edits for MergeTree tables
(`commit_cell_edits`, `poll_mutation_status`), reported as queued, never as
applied; the native app does not expose them yet.

**Redis** writes take effect immediately and most users point dbunk at local
or development servers, so writes are allowed unless the policy says
otherwise. The native console (`backend/src/backend/development/redis_session.rs`)
refuses blocking and connection-changing commands, treats anything outside
the read allowlist as a write, and requires confirmation for the destructive
command list (`backend/src/redis/destructive-commands.toml`) whatever the
safe mode.

## Considered options

- **Unifying cell edits across engines.** Rejected: ClickHouse mutations are
  asynchronous and the others are not.
- **Matching ClickHouse's read-only default for Redis.** Rejected: the
  audiences and risks differ.

## Consequences

The older replica-role guard (`backend/src/redis/key_ops.rs`, writes blocked
when `INFO replication` reports a replica) is not wired into the native Redis
session; read-only is the connection policy's flag.
