# ADR-0004 — `lastActivityAt` lives on the connection record

**Status**: Accepted (2026-05-10)

**Update (ADR-0007, 2026-05-11)**: the timestamp still lives on the connection
record, but the record is now stored in SQLite rather than `connections.json`.

## Context

Connection cards show "Last activity 12m ago". The data has to come from
somewhere. Two options:

1. **Derive from query history** — compute on-the-fly as the most recent
   `query_history` entry per connection. No schema change.
2. **Store on the connection record** — bump `lastActivityAt` whenever a
   query or connect succeeds.

Option 1 reads cheap on small histories but degrades when history grows or
when a user clears their history (a common privacy action). It also can't
account for a successful `connect_connection` that didn't run a query.

## Decision

Option 2. `StoredConnection` carries an `Option<String>`
`lastActivityAt`, written via the helper `touch_connection_activity` after
every successful `run_query` and `connect_connection` (and any future
operation that meaningfully exercises the connection).

## Consequences

- One SQLite write per query/connect. Cheap; happens off the hot result path.
- The frontend gets a stable timestamp it can render directly without
  scanning history.
- `touch_connection_activity` updates the `connections` table only. It does
  not hydrate credentials because it does not need passwords.
- If a query path ever returns a result without invoking the helper, the
  card silently goes stale. Prefer adding a single helper call at the end
  of every command rather than threading bumps through error branches.
