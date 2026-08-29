# ADR-0023: Staged PostgreSQL Result Mutation contract

Status: Accepted

## Decision

Result Mutation is a dark, PostgreSQL-only backend for analyzing result
updatability, previewing parameterized DML, and applying a reviewed Mutation
Plan atomically. The existing mutation commands and every frontend caller stay
unchanged until Plan 006 activates the contract.

### Analysis and socket ownership

Mutation Analysis prepares the executed single statement with the extended
protocol and inspects each column's `table_oid` and `column_id`. PostgreSQL's
wire RowDescription includes origin fields for both simple and extended query
protocols, but tokio-postgres's simple-query API discards them. Analysis is
therefore a separate Parse/Describe step, which does not execute the statement,
on one dedicated read-write mutation socket per Connection. That socket sits
alongside, but is distinct from, the read-only Table Browse socket.

Preparation happens on a different session from query execution. Runtime
`search_path`, `SET ROLE`, and temporary-table state can therefore differ. OID
resolution prevents most name drift, and analysis fails closed with
`possibleTempShadowing` whenever a resolved relation name also exists in any
live `pg_temp_%` schema. The remaining runtime `search_path` risk is explicit:
Plan 006 must show every fully qualified resolved target in the review surface.

Analysis requests are supersedable by newer requests for the same tab. Apply
requests are exclusive, never superseded, and reject concurrent admission as
`busy`; only explicit cancellation or lifecycle teardown interrupts apply.
Analysis snapshots live in a bounded per-executor cache. Preview and apply
refer to an `analysisId`; eviction or invalidation returns `analysisExpired`
and requires analysis recovery before any DML can run.

### Identity and writability

ADR-0022's identity choice has one shared implementation: primary key, then the
smallest qualifying unique index, then `ctid` for ordinary heaps or
`(tableoid, ctid)` for partitioned tables, then none. A persisted virtual key
is an ordered user-selected column claim used only when a proven identity is
absent or not projected. A virtual key is never treated as a proven constraint.

Any non-empty `attgenerated`, including PostgreSQL 18 virtual generated
columns, is non-writable. `attidentity = 'a'` is identity-always and is also
non-writable in v1. System origins with `column_id <= 0` are non-writable and
never resolved through `pg_attribute`. Insert omits an unset column with
`atthasdef` or any `attidentity`, allowing its default to run; explicit NULL is
still a value.

### Preview, guards, and apply

The pure DML builder quotes identifiers and binds every user value as a text
parameter with a catalog-derived cast: `($N::text)::<format_type>`. DML Preview
returns the exact statements and ordered parameters that apply will build from
the same analysis snapshot. Neither SQL nor values may be logged.

Predicates are value-dependent and null-safe: non-NULL values use indexable
`"column" = ($N::text)::<cast>`, while NULL uses `"column" IS NULL` without a
parameter. `IS NOT DISTINCT FROM` was rejected because PostgreSQL does not
treat its DistinctExpr as an indexable equality clause.

Guard strength depends on identity kind. Primary-key and unique-index updates
guard the old values of edited columns. Their deletes guard every projected
column. Virtual-key and `ctid` operations always guard the full projected row,
including NULLs. Thus keyed updates deliberately do not detect changes to
unedited columns, and value-identical A to B to A rewrites pass. Full-row
guards limit stale `ctid` reuse but cannot turn a virtual key into a constraint.

Apply reloads the descriptor, rebuilds the plan, starts one transaction, sets
`SET LOCAL lock_timeout = '10s'`, and executes operations in order. An update
or delete affecting zero rows returns an attributed conflict; more than one
returns `identityNotUnique`. Database errors, `lock_timeout`, conflict,
uniqueness failure, cancellation, and teardown all roll back the entire plan.
No partial success is reported. Rollback is followed by an idle-state check,
with reconnect if the socket is unusable. This all-or-nothing policy makes a
reviewed Mutation Plan the sole unit of commit.

## Consequences

- Query execution stays on the simple protocol and is behaviorally unchanged.
- Analysis can classify direct columns, expressions, joins, generated and
  identity columns without executing user SQL.
- Keyless editing is explicit and guarded, but the user owns the uniqueness
  claim of a virtual key.
- Bounded queueing, `lock_timeout`, cancellation, descriptor invalidation, and
  `analysisExpired` recovery keep failure behavior predictable.
- Plan 006 owns the client-side Mutation Draft and activation; this ADR adds no
  frontend mutation path.
- ADR-0026 applies the same preview and backend-regeneration boundary to typed
  PostgreSQL object DDL, with explicit standalone groups for operations that
  cannot run transactionally.
