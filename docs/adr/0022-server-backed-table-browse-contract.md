# ADR-0022: Server-backed PostgreSQL Table Browse contract

Status: Accepted

## Decision

Table Browse is a PostgreSQL-only, dark, typed command surface that pages,
filters, and sorts a relation on a dedicated tokio-postgres connection. SQLx
and `load_table_data` remain the live path for every engine until Plan 004
activates this contract in the grid.

The query-session actor cannot carry bind parameters because it executes
through the simple query protocol. Table Browse therefore does not reuse that
actor for execution. It reuses the same `ResolvedPostgresConnectSpec`, TLS
connector, ordered driver-option SQL, and `CancelToken` machinery, then runs
statements through the extended query protocol so every user value is a `$N`
text parameter.

The browse socket additionally runs `SET default_transaction_read_only = on`.
That is a robustness boundary against raw-filter side effects, not a security
boundary: the user already holds the credentials, and PostgreSQL rejects an
attempt to escalate the session back to read-write.

Value parameters are always bound as text and cast in SQL as
`($N::text)::<cast_type>`. The cast type comes from
`pg_catalog.format_type(atttypid, atttypmod)`, never from
`information_schema.data_type`, whose display strings (for example `ARRAY`)
are not valid cast targets. tokio-postgres checks `ToSql::accepts` against
server-inferred types, so binding a Rust integer against a text-inferred
parameter, or text against an integer parameter, fails at runtime without the
explicit text-then-cast scheme.

keyset pagination is supported only when the effective sort is exactly the
identity order (no user sort keys), including `ctid` virtual identity on
`server_version_num >= 140000`. It is forward-only and uses a row-value
comparison over the identity columns. User sorts, page-number jumps,
previous-page navigation, pre-14 keyless tables, and relations with identity
`none` use labeled `LIMIT/OFFSET` fallback. An invalid or stale cursor is the
typed error `invalidCursor`.

`browse_table_data` never runs `COUNT(*)`. Estimated counts with no filters
read `pg_class.reltuples` and report `unknown` when `reltuples <= 0` (never
analyzed). Estimated counts with filters run `EXPLAIN (FORMAT JSON)` and
report the top plan's `Plan Rows`. Exact counts are a separate, user-initiated
`count_table_browse_rows` command using the same generated WHERE, supersedable
and cancelable like any browse request.

Keyless ordinary heaps (`relkind` `r`) receive a `ctid` virtual identity
so paging is total and rows remain addressable. Keyless partitioned tables
(`relkind` `p`) use `(tableoid, ctid)` because `ctid` is unique only within
a child partition; a `ctid`-only keyset can skip rows when equal CTIDs
cross a page boundary. `tid` gained full btree comparison operators in
PostgreSQL 14; older servers fall back to offset paging for keyless tables.
Foreign tables and other non-heap relations keep identity `none`. These
virtual identities are stable only between fetches while rows are not
updated or vacuumed; they are browse continuity, not a mutation key.

Requests are keyed by `(connectionId, tabId, requestId)`. A newer
`requestId` for the same tab supersedes the older one: a queued request is
dropped, an in-flight request is protocol-cancelled, and the superseded
command resolves `superseded`. `cancel_table_browse` cancels the current
request without replacing it. One browse socket per Connection serializes
tabs; queue wait is bounded at 10 seconds.

Identity for browse mode is backend-authoritative: primary key, else the
smallest qualifying unique index (valid, immediate, non-partial,
non-expression, all-non-nullable; ties by index name), else a virtual
identity (`ctid` or `(tableoid, ctid)`), else `none`. This supersedes the
drifting pair `stable_order_columns` and `pickRowIdentity` for browse mode
only. Neither legacy implementation changes in this decision.

Admission is at most one browse socket per Connection and eight across the
app, accounted separately from the query-session budget. An idle executor
closes after 300 seconds. Teardown fencing matches query sessions: close the
executor before the Connection, bastion, managed server, or credentials
disappear. Storage-mode migration does not close executors.

Commands reject through a typed `TableBrowseError` union. SQL, filter text,
row values, parameter values, and structured database error detail are never
logged. `tokio_postgres` stays at `Warn`.
