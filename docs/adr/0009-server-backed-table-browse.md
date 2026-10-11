# Server-backed table browse

**Status:** Accepted (Plan 003, rewritten 2026-10-11)

PostgreSQL table tabs page on the server over a dedicated read-only socket
(`backend/src/table_browse/`), never by loading the table. Filters bind values
as text with an explicit cast; paging is keyset on the identity order and
falls back to offset; totals are estimates until the user asks for an exact
count; a newer request supersedes a stale one.

## Considered options

- **`COUNT(*)` on every page.** Rejected: unbounded on large tables.

## Consequences

- Cast types come from `format_type`, not `information_schema`.
- Partitioned tables without a key use `(tableoid, ctid)`; `ctid` alone is
  not unique across partitions.
