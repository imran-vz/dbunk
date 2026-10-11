# Staged result mutations and typed cell editors

**Status:** Accepted (Plans 005 and 032, rewritten 2026-10-11)

Row edits in PostgreSQL results are staged, reviewed and applied as one unit.
The backend analyses the result with Parse/Describe on a dedicated read-write
socket to find editable columns and row identity, renders parameterised DML
whose `WHERE` guards the original values of edited columns, and applies the
whole set in one transaction with `lock_timeout = 10s`
(`backend/src/result_mutation/`). The review shows the diff and the exact SQL;
the safety policy (ADR-0004) decides whether a confirmation is required.

If the connection is lost during apply, the outcome is recorded as unknown
and the draft is locked until the user refreshes and marks it resolved. Apply
is never retried automatically.

Cell editing is typed in the native grid (`apps/native/src/table_changes/`):
plain values edit inline; JSON, arrays, geometry, multiline and long values
open a popover editor with validation. Every editor writes into the same
staged draft.

## Considered options

- **`IS NOT DISTINCT FROM` guards.** Rejected: PostgreSQL cannot use an index
  for it.
- **A per-type editor registry for every cell.** Rejected to keep bulk
  editing fast.

## Consequences

Keyed updates guard only edited columns, so an A → B → A change by someone
else passes the guard.
