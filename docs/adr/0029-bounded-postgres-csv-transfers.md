# ADR-0029: Bounded PostgreSQL CSV transfers

**Status**: Accepted and implemented (Plan 020, `7745946`, selected mock A; completion confirmed 2026-09-05)

## Decision

CSV import and whole-table CSV export use a PostgreSQL Transfer Job, separate
from ADR-0028's native backup/restore job. Transfer execution uses the existing
dedicated PostgreSQL transport and local files, with typed admission, polling,
cancellation and explicit release. The table Transfer sub-tab owns setup only;
the native manager and the app observer own work beyond the tab's lifetime.

Inspection freezes the connection, relation description, CSV settings and a
bounded file sample into a five-minute review token. Tokens are explicitly
releasable, capped at eight, and invalidated by connection/global resource fences.
A start consumes the token only after safety authorization and resolution. An
uncertain start response is reconciled by list, never automatically retried.

Four active or resolving transfers are allowed globally, one per connection.
These limits are additional to the four native backup/restore jobs. Terminal
history is capped at 32 records for one hour. Snapshots retain filenames, never
absolute paths, rows, SQL or credentials. Source sample values remain transient
inspection data. Byte/row counters fit JavaScript's exact integer range.

## Files and data

V1 is UTF-8 CSV with an explicit header choice, single-byte delimiter/quote/escape,
and NULL token. Source-column indices identify mapping entries; header labels do
not identify columns. Quoted NULL tokens remain text. Omitted target columns use
server defaults. Imports reject mapped generated/identity columns, unsupported
relations and RLS-protected targets. They do not silently switch to a buffered
fallback. Session date interpretation is ISO and UTC.

The parser caps fields at 1 MiB, records at 8 MiB and source width at 1,600 columns.
Inspection scans at most 256 KiB and returns at most 50 records / 64 KiB of values.
File and COPY chunks are at most 64 KiB with backpressure. There is no whole-file
or whole-table renderer array. Invalid input after a valid prefix still aborts
the transaction. File metadata and relation signatures are checked again during
execution. Keeping an open source handle does not provide a snapshot against
in-place edits; the user must keep the source unchanged.
The relation signature includes fixed-size server-side fingerprints of column
defaults, so replacing a default after inspection requires a fresh review.

Four filesystem worker permits bound blocking OS calls separately from job
admission. A cancelled async caller cannot release the permit held inside a
still-running filesystem operation. Export also checks field and conservative
encoded-record sizes on the server before COPY emits a complete row message.
The guard and COPY use the same canonical type-output text, with UTF-8 byte
counts. User-defined casts to text must not change either the exported data or
the size calculation. A streaming subquery keeps that projection stable without
retaining the entire relation in a materialized result.

Import appends in one transaction and stops on the first error. COPY completion
is not COMMIT. Defaults, triggers and constraints remain server behavior; rollback
cannot undo sequence increments or external effects from triggers. Export runs a
generated SELECT through COPY TO STDOUT in a read-only transaction so partitioned
relations include their children and server SELECT policies remain authoritative.
It does not apply grid filters, staged edits or a row-order guarantee.

Exports write a private create-new sibling partial, flush/sync, then publish with
no replacement. Empty exports are valid. Cancellation/failure removes only the
job's partial. A hard crash can leave that clearly named partial behind.

## Completion and teardown

An atomic claim serializes cancellation with the start of COMMIT or publication.
Cancellation can win before that boundary. Once finalization starts, cancellation
cannot promise rollback or file deletion. Lost acknowledgement around COMMIT is
`outcomeUnknown`, not cancelled, failed-with-rollback, or completed. The UI asks
for target inspection before retrying. Only acknowledged commit/publication runs
success activity and any required safety override audit, with a bounded wait.

All existing connection and global socket fences include transfers before tunnel
or pool invalidation. They close admission, invalidate reviews, cancel pending
resolution and stop cancellable jobs. Aborted workers retain capacity until their
join completes. Finalizing work is not relabelled cancelled by a timeout. The app
warns about transfer impact alongside backup/restore jobs before lifecycle changes.

An observed committed import invalidates connection data caches conservatively
because triggers can change other relations. Pending edits and query results stay
visible, but stale mutations must be refreshed and reviewed before applying.
Unknown commit outcomes also mark data stale. Poll failures describe unavailable
observation rather than inventing a database result.

## Boundaries

Visible-row and selection exports retain their loaded-data path. XLSX and other
formats, compression, alternative encodings, upsert, truncate-first, skip-error
mode, reject files, resumability, scheduling and other engines are follow-ups.
The UI must identify legacy buffered paths honestly. This decision does not
claim complete transfer parity.
