# Backend-owned object DDL

**Status:** Accepted (Plans 013 and 016, rewritten 2026-10-11)

The UI never sends DDL text. It sends typed operations; the backend validates
each fragment, renders and classifies one statement per operation, and runs
the same generator again at apply time (`backend/src/postgres/object_ddl/`,
`backend/src/postgres/object_service.rs`). Statements that cannot run in a
transaction run in their own groups. Drop impact comes from a bounded
`pg_depend` walk.

Security and routine DDL follow the same path. Routine bodies are opaque and
sealed in a `$dbunk$` dollar quote. Triggers and policies are addressed by
schema, table and name; privileges come from a closed list. The renderer, not
the statement classifier, marks revokes and trigger or RLS disables as
destructive.

## Consequences

- `ALTER TYPE … ADD VALUE` runs standalone (SQLSTATE 55P04).
- DDL cancellation is deliberately not supported yet.
