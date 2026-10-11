# Bounded file and transfer jobs

**Status:** Accepted (Plans 018 and 020, rewritten 2026-10-11)

Long PostgreSQL operations that touch files run as backend-owned jobs, not
inside a request:

- **Backup and restore** run the installed `pg_dump`, `psql` and `pg_restore`
  (`backend/src/postgres/backup/`), with a patched-client version floor and
  restricted-mode plain restore.
- **CSV import and export** stream `COPY` in bounded chunks behind a review
  token (`backend/src/postgres/transfer/`).

Both kinds are limited to four jobs app-wide and one per connection, are
observed by polling rather than pushed events, write output to a private
partial file that is published without overwriting, and are covered by every
socket fence that closes or reconnects the connection. A commit whose outcome
is unknown is reported as `outcomeUnknown`; a lost acknowledgement is never
retried automatically, the client reconciles through the job list.

## Considered options

- **Parallel restore.** Excluded: it conflicts with `--single-transaction`.

## Consequences

Leftover partial files are never promoted or swept automatically.
