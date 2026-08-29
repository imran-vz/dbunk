# ADR-0026: Backend-owned PostgreSQL object DDL workflow

- **Status:** Accepted (Plan 013, `PAR-007`)
- **Date:** 2026-08-29

## Context

PostgreSQL object browsing previously returned untyped name lists, while the
frontend generated a small subset of `ALTER TABLE` statements and sent whole
SQL scripts to `execute_ddl`. That path could not describe object identity
safely for overloaded routines, disclose transitive drop impact, attribute a
failure to one statement, or run `CREATE INDEX CONCURRENTLY` outside a
transaction.

Object lifecycle needs the same reviewed boundary as Result Mutation: typed
intent first, an inspectable preview, and backend-owned execution that
regenerates what it will run.

## Decision

### Typed object identity and catalog

`PgObjectRef` identifies an object by kind, schema, name, and, for functions,
procedures, and aggregates, the exact output of
`pg_get_function_identity_arguments`. Routine identity is never derived from a
display string.

The object-kind union covers schemas, relations, sequences, routines, types,
domains, and extensions. Event triggers, roles, and tablespaces remain
database-scoped list entries because this lifecycle does not describe or
mutate them. Promoting one later requires explicit catalog, describe, and DDL
decisions.

The native PostgreSQL pool loads the catalog in bounded, cross-schema batches.
It caps schemas first and binds that retained name set into every schema-scoped
kind query, bounding database work as well as the serialized result.
Every per-kind group is capped and reports truncation, and extension-owned
members are excluded from normal object groups. Object description returns a
typed fact union plus reconstructed DDL where PostgreSQL can represent it.
Descriptions run in a repeatable-read, read-only transaction so multi-query
facts come from one snapshot. Reconstructed ranges preserve the catalog's
qualified multirange name, subtype opclass, collation, canonical function, and
subtype-diff function. View definitions have PostgreSQL's trailing terminator
removed before reconstruction. Foreign-table DDL retains column FDW options,
defaults, explicit non-default collations, nullability, and CHECK constraints.
Sequence ownership follows serial and identity dependencies and is fetched as
separate identifiers;
the human-readable fact never becomes the source for executable quoting.
All sequence quantities are decimal strings over IPC, preserving the complete
signed 64-bit range without JavaScript number coercion.

### Operations cross IPC, statements do not

The frontend sends a `PgObjectOp[]`. It never sends a complete DDL statement
for execution. Some operations necessarily contain SQL fragments, including
view bodies, check and index expressions, predicates, `USING` casts, and
expression defaults. Generation validates each fragment for statement
boundaries, renders one complete statement per operation, and verifies the
result with `classify_script` as exactly one DDL statement.

Type fragments receive a stricter context-specific check: top-level column
constraints and options are rejected, while PostgreSQL multiword, typmod,
array, qualified, and quoted type forms remain valid. Sequence types accept
only PostgreSQL's supported integer types. Expression defaults are enclosed in
renderer-owned parentheses so expression tokens cannot become column options.

Preview and apply use the same pure deterministic generator. Apply discards
any earlier preview and regenerates from the operations at the backend trust
boundary. Preview reads only the persisted connection record for its engine
check; it does not hydrate a credential or resolve an SSH tunnel. Tagged
defaults distinguish quoted literals from raw expressions;
identifiers and literals use the shared `quote_double` and `quote_literal`
helpers. Literal rendering always uses PostgreSQL `E` strings with both
apostrophes and backslashes escaped. Sequence numeric strings must parse as
signed 64-bit integers before they are rendered.

### Preview, grouping, and execution

Preview returns SQL, a human-readable summary, destructive and transactional
flags for each statement, and ordered execution groups. Contiguous
transactional statements form one atomic group. `CREATE INDEX CONCURRENTLY`,
`DROP INDEX CONCURRENTLY`, and `ALTER TYPE ... ADD VALUE` each form a
standalone group: a new enum label cannot be used inside the transaction that
added it (SQLSTATE `55P04`), so grouping it with a following default or check
would fail atomically for a plan that succeeds sequentially.

Apply acquires a native pooled connection and immediately detaches it, so a
failed transaction can never return a poisoned socket to the pool. The
connection keeps the driver options the pool applied, including any
user-configured `statement_timeout`, exactly as the legacy DDL path does; a
long rewrite or index build is bounded by the operator's own setting rather
than silently unbounded. Apply adds `lock_timeout = '10s'` so DDL waiting
behind another transaction fails in a bounded, typed way. Atomic groups execute in a `sqlx::Transaction` and count
as applied only after commit. Standalone groups can create honest partial
progress; failures report the statement index and the count from earlier
completed groups.

A failed concurrent index build or concurrent index drop is checked in
`pg_index`, since both leave an `INVALID` index behind once their first phase
has run. An invalid leftover is returned as typed residue, including when SQLSTATE `55P03` maps the failure
to `LockTimeout`, so the client can tell the operator exactly what must be
dropped before retrying. Apply-time database errors preserve SQLSTATE and a
query-relative character position only after converting it to a byte offset
against the generated statement. Read errors report no position because their
query text is not retained. DDL cancellation is deferred: safely
cancelling an unbounded rewrite or concurrent build from another connection
requires a separately designed `pg_cancel_backend` permission and lifecycle
contract.

### Drop impact and safety policy

Drop impact is a deterministic breadth-first walk through `pg_depend`, because
PostgreSQL `CASCADE` is transitive. It is capped at eight levels and 201
normalized, unvisited addresses per level, or 1,609 retained addresses
including the root. View `_RETURN` rules in `pg_rewrite` are normalized
to their owning views before each cap; other rules stay rules, because
`CASCADE` drops the rule and not the table it is defined on. Cycles are
excluded through the visited set, and subobject walks match the exact
referenced column. Final
identity/minimum-depth deduplication precedes the 201-row result probe. Address
or depth saturation sets truncation even when identities later collapse below
the 200-result cap. The walk follows normal (`n`), auto (`a`), and internal (`i`) dependencies,
but reports what PostgreSQL itself discloses for `CASCADE`: normal dependents,
plus owned and identity sequences. Other auto and internal dependents (a
table's own defaults, constraints, indexes, and row type) are dropped as parts
of their owner and are walked silently so their dependents are still found. Root resolution, every breadth, and final identification run in
one repeatable-read, read-only transaction. The UI receives identity and depth. Generated
drops use explicit `RESTRICT` unless the operator opts into `CASCADE`;
PostgreSQL remains the enforcement boundary.

Apply reuses `WriteIntent::Ddl`. Read-only connections are blocked and
protected or strict connections require confirmation. DDL confirmation uses a
dedicated `DdlStatementSummary`, rather than the query classifier's
`StatementClassSummary`, so the review surface can show the exact
human-readable operation. Confirmed overrides are audited once any group has
committed or the whole plan succeeds, because a later standalone failure does
not undo the schema change an earlier group already made.

## Consequences

- Plan 014 can activate object browsing and lifecycle UI without moving SQL
  generation into a rendering path.
- Plan 015 can move PostgreSQL structure edits, index creation, and foreign-key
  creation onto the same typed operation union. ClickHouse keeps the legacy
  frontend builder.
- Non-transactional groups make partial progress possible, but that progress
  is explicit and attributed instead of hidden behind a single script error.
- The legacy `execute_ddl` remains until its PostgreSQL callers migrate; it is
  not the object-lifecycle path.

## Related

- ADR-0023: the staged Result Mutation contract that established the
  analyze, preview, apply shape.
- ADR-0024: backend-enforced production safety policy and audit rules.
- ADR-0025: native PostgreSQL connection and TLS resolution used by the
  catalog and detached apply socket.
