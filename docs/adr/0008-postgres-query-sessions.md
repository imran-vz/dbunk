# Dedicated PostgreSQL query sessions with statement-scoped execution

**Status:** Accepted (Plans 001 and 023, rewritten 2026-10-11)

PostgreSQL query tabs run on a dedicated tokio-postgres connection owned by a
session actor (`backend/src/query_session/`), not the SQLx pool used
elsewhere. Only the native protocol gives notices, cancellation, zero-row
column metadata and the transaction status byte the transaction controls
need. Sessions are bounded (per connection and app-wide, with a lease) and
stream results under a credit window the host acknowledges, so a slow UI
cannot buffer unbounded rows.

Execution is statement-scoped:

- Named `:params` are rewritten to `$k` bound values
  (`backend/src/postgres/sql_params.rs`); the safety policy classifies the
  rewritten text.
- Row-limited or parameterised reads run through a server cursor with one
  eager `FETCH`, so the client's credit never holds the wrapper transaction
  open. Other parameterised statements run as one bound command.
- Validation refusals happen before the policy check, and cancellation is a
  distinct `cancelled` outcome.

## Considered options

- **Fetching the cursor in chunks.** Rejected: a cancel between chunks is lost.
- **Interpolating parameters as literals.** Rejected: not string-, comment- or
  slice-aware.

## Consequences

The server's type inference limits bound parameters (`:x IS NULL` needs a
cast); that is a documented limit, not a bug.
