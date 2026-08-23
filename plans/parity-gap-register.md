# DBeaver and TablePlus parity gap register

## Purpose

This is the canonical inventory of capability gaps found in the 2026-08-18
source audit of dbunk. It records missing behavior, partial implementations,
evidence, priority, and dependencies. It is not a claim that every item should
be built immediately.

The recommended product target is the union of:

- DBeaver Community's PostgreSQL daily-driver workflows.
- Paid TablePlus desktop workflows that materially affect individual database
  users.
- Selected DBeaver paid features, tracked as later professional or stretch
  work.

Literal parity with every DBeaver edition also implies dozens of database
drivers, enterprise identity, team administration, cloud tooling, and a mature
plugin ecosystem. Those obligations are listed under `PAR-015`; they are not
part of the recommended PostgreSQL-first critical path.

## Audit boundary and confidence

- Audited repository commit: `24432fb`.
- Primary engine: PostgreSQL, following
  `docs/adr/0001-postgres-first-engine-coverage.md`.
- Competitor sources: official DBeaver and TablePlus product documentation.
- dbunk evidence: static inspection of frontend, Tauri commands, Rust database
  code, tests, ADRs, and project planning documents.
- Not audited: exhaustive runtime UX behavior, every platform build, every
  DBeaver edition entitlement, and feature-by-feature behavior for every
  non-PostgreSQL driver.
- TablePlus documentation is primarily macOS-oriented. Platform-specific
  availability must be verified before treating a feature as a universal
  TablePlus requirement.

## Status vocabulary

| Status | Meaning |
|---|---|
| Complete | The selected parity scope is implemented and verified. |
| Missing | No credible implementation was found. |
| Partial | A useful surface exists but lacks parity-grade depth or correctness. |
| Deferred | Real literal-parity work outside the recommended current boundary. |

Effort is coarse: `S` is hours, `M` is roughly a day, `L` is multi-day, and
`XL` is a multi-stage feature or architectural change. Risk describes the
implementation's blast radius, not the severity of the missing capability.

## Priority summary

| ID | Capability | Status | Priority | Effort | Risk | Confidence |
|---|---|---|---:|---:|---:|---:|
| PAR-001 | Query sessions, execution, results, and transactions | Partial | P0 | XL | High | High |
| PAR-002 | Server-backed table browsing | Complete | P0 | L | Medium | High |
| PAR-003 | Editable query results and staged mutation review | Partial | P0 | L | High | High |
| PAR-004 | Production safety policy | Complete | P0 | L | High | High |
| PAR-005 | Workspace persistence and global navigation | Partial | P0 | L | Medium | High |
| PAR-006 | Connection security and organization | Partial | P1 | L | High | High |
| PAR-007 | PostgreSQL object lifecycle management | Partial | P1 | XL | Medium | High |
| PAR-008 | Schema and data comparison with migration | Partial | P1 | XL | High | High |
| PAR-009 | Diagram editing and visual query building | Partial | P1 | L | Medium | High |
| PAR-010 | Transfer, DDL export, backup, and restore | Partial | P1 | XL | High | High |
| PAR-011 | PostgreSQL administration and observability | Partial | P1 | XL | Medium | High |
| PAR-012 | Reusable tasks, scheduling, and extensibility | Missing | P2 | XL | High | High |
| PAR-013 | Desktop platform and release parity | Partial | P2 | XL | High | High |
| PAR-014 | Non-PostgreSQL engine depth | Partial | P2 | XL | High | High |
| PAR-015 | Literal DBeaver enterprise and ecosystem parity | Deferred | P3 | XL | High | High |
| PAR-016 | Redis advanced-client depth | Partial | P3 | XL | Medium | High |
| PAR-017 | Product claims and documentation accuracy | Partial | P1 | M | Low | High |

## P0: daily-driver foundations

### PAR-001: Query sessions, execution, results, and transactions

**Current state:** Partial. The persistent PostgreSQL query-session foundation
is complete; narrower execution follow-ons remain.

**Progress (2026-08-19):** Plans 001 and 002 are DONE through commit `26268ca`.
PostgreSQL query tabs now own dedicated backend sessions with bounded streamed
results, cancellation, multiple result sets, notices, explicit transaction
state and controls, retained-result budgeting, and lifecycle fencing. This
satisfies the dependency needed to begin `PAR-002` without claiming the
remaining items below are complete.

**Evidence:**

- `src-tauri/src/query_session/mod.rs` owns persistent actors, admission,
  execution, acknowledgement credit, cancellation, transaction actions, and
  deterministic teardown.
- `src-tauri/src/query_session/observer.rs` derives transaction state without
  consuming the SQLx metadata/mutation pool.
- `src/lib/store/query-sessions.ts` owns the tab-keyed frontend lifecycle and
  typed transaction commands.
- `src/lib/query-session-budget.ts` enforces the retained-result budget and
  preserves compact execution summaries when payloads are released.
- `src/components/query-editor/results-view.tsx` and
  `transaction-controls.tsx` expose multiple results, output/notices,
  cancellation outcomes, and manual transaction recovery.

**Remaining pieces:**

- Configurable maximum-row policy per execution, separate from hard retained
  result limits.
- Script policies to stop, continue, or prompt after a statement error.
- Command tags. The current public driver exposes affected-row counts but not
  tags.
- Driver-bound parameters. Current bind-variable support is literal SQL
  substitution.
- Savepoint controls.

**Target outcome:** A query tab owns a typed session descriptor and, while
connected, a backend session handle. Executions have IDs and explicit state;
results arrive in bounded batches; cancellation and cleanup are idempotent;
transaction state is server-derived and visible. PostgreSQL is implemented
first behind contracts that do not force other engines to pretend they support
identical semantics.

**Implementation split:** Plan 001 landed the driver, actor, observer, IPC
contract, and lifecycle fencing as a dark backend. Plan 002 added the selected
UI, typed frontend reducer, retained-result LRU release, controls, and
activation. Both plans are DONE; the remaining items above are follow-on work
and do not block the completed `PAR-002` scope or the selected `PAR-003`
planning target.

**Dependency:** None. This unblocks `PAR-002`, `PAR-003`, `PAR-004`, and
`PAR-005`.

### PAR-002: Server-backed table browsing

**Current state:** Complete for the selected PostgreSQL table-browse scope.

**Progress (2026-08-21):** Plans 003 and 004 are DONE through commit
`ecefce8`. PostgreSQL table tabs now use the typed browse contract end to end:
server-side typed/raw filtering and multi-column sorting, bounded keyset/offset
pagination, explicit count semantics, cancellation and stale-response fencing,
backend-authoritative identity, query inspection, partial-result disclosure,
durable preferences, history, and presets. The implementation keeps legacy
engines and query-result grids on their existing paths and supplies the
identity/query metadata needed by `PAR-003`.

**Evidence:**

- `src-tauri/src/table_browse/` owns parameterized SQL construction,
  admission, cancellation, bounded execution, identity, and result metadata.
- `src/lib/table-browse-client.ts` and `src/lib/store/table-browse.ts` own
  monotonic requests, tab-scoped state, newest-request-wins, preferences,
  history, presets, counts, and cleanup.
- `src/components/data-grid/browse-controls.tsx` and
  `src/components/table-editor/use-table-session.ts` activate the contract in
  PostgreSQL table tabs while preserving legacy grid consumers.
- Store, component, client, protocol, builder, executor, and live PostgreSQL
  tests cover the delivered contract and failure paths.

**Target outcome:** Sorting and filtering operate on the relation, not just the
visible page. Large tables remain responsive and bounded, and every request is
cancelable and keyed to the current grid state.

**Dependency:** `PAR-001` result batching and cancellation contract.

### PAR-003: Editable query results and staged mutation review

**Current state:** Partial for table tabs, effectively missing for query
results.

**Progress (2026-08-21):** Plans 005 and 006 were authored at commit
`ecefce8` and are the selected execution path. Plan 005 is a dark
PostgreSQL-only backend: updatability analysis through extended-protocol
`Parse`/`Describe` column origins, a catalog descriptor with
generated/identity-column awareness, persisted virtual keys, a pure
parameterized DML builder with per-operation preview, and an
all-or-nothing transactional apply with typed per-operation conflict
detection. Plan 006 activates one identity-keyed staged Mutation Draft
model across browse-mode table tabs and query results, with generated-DML
review, per-change inclusion/exclusion/revert, and keyless editing via
virtual keys and guarded `ctid`. Deep editors, Quick Look, batch paste,
and copy formats stay in this register as follow-on scope after Plan 006.
Both plans passed an independent adversarial review on 2026-08-21 and were
amended for its findings, most notably: a conservative
temp-table-shadowing refusal plus qualified-target display against
cross-session name-resolution drift, indexable null-safe guard rendering
instead of `IS NOT DISTINCT FROM`, full projected-row guards on keyed
deletes with the keyed-update conflict limits documented rather than
overclaimed, PostgreSQL 18 virtual generated columns classified
non-writable, explicit draft-loss policy (budget release never drops a
draft; re-run and close confirm), and a review-integrity rule that apply
never runs DML differing from the previewed statements.

**Evidence:**

- `src/lib/store/edit-strategies.ts:95-193` supports safe PostgreSQL table-row
  identity from primary or non-null unique keys.
- `src/components/query-editor/toolbar.tsx:115-124` renders a query-result Save
  control without a save handler.
- PostgreSQL table mutation code supports parameterized transactional writes,
  but the query editor has no equivalent mutation lifecycle.

**Missing pieces:**

- Updatability analysis for arbitrary result sets.
- Safe editing of eligible single-table and join results.
- Virtual-key selection and persistence.
- Bulk edit, duplicate row, batch paste, and multi-row delete.
- Generated-column and identity-column awareness.
- Generated DML preview before commit.
- Per-change inclusion, exclusion, and revert.
- Conflict detection when source rows change between fetch and save.
- Transactional apply with a clear partial-failure policy.
- Deep editors and Quick Look for JSON, XML, arrays, BLOB/bytea, images, and
  spatial values.
- Configurable copy formats for cells, rows, and result sets.

**Target outcome:** Eligible query and table results share one typed pending
mutation model. Users can inspect generated parameterized DML, commit it
transactionally, or discard it without ambiguity.

**Dependencies:** `PAR-001` and the identity/query metadata from `PAR-002`.

### PAR-004: Production safety policy

**Current state:** Complete for the selected relational safety-policy scope.
Plans 007 and 008 are implemented and merged to `main`.

**Progress (2026-08-23):** Plans 007 and 008 delivered the selected execution
path. Plan 008's implementation commit `5409d66` landed on `main` as
`5991dbf`. Plan 007 added a dark backend:
per-connection `environment` (development/test/staging/production) and
`safeMode` (inherit/disabled/protected/strict) fields plus a relational
`readOnly` flag reusing the migration-7 column; a fail-closed PostgreSQL
statement classifier built on the extracted Plan 005 lexer (unknown/`DO`/
`CALL`/lex-failure treated as destructive writes, `EXPLAIN ANALYZE`
unwrapping, `COPY` direction, `WITH` write detection, paren-depth-aware
unbounded UPDATE/DELETE detection); one shared `assert_permitted` gate at
all fifteen write-capable surfaces including the query-session admission
point, the result-mutation apply, the `copy_table_rows` destination, and
the subprocess restore; typed `policyBlocked` / `policyNeedsConfirmation`
refusals on the actor surfaces and `[policy:…]`-tagged strings on legacy
`Result<T, String>` commands; a `default_transaction_read_only` session GUC
as belt-and-braces (documented as not the boundary); and a capped,
cascade-deleted audit of confirmed overrides. Plan 008 activated it in the
UI: form controls, environment badges and production identity across
sidebar/header/tabs/banner/status bar, one shared confirmation dialog that
re-sends with `confirmed: true` after deliberate acknowledgment (typed
confirm for destructive/production), read-only affordance gating in
`resolveEditContext`, and the override audit in Settings. Deliberately
deferred from this pair: environment-scoped smart-commit/manual-transaction
defaults (`PAR-001` savepoint follow-ons), user-picked connection colors
(`PAR-005`/`PAR-006`), script stop/continue/prompt error policies, and
typed error migration for legacy commands (`PAR-007`/`PAR-014`).
Both plans passed an independent adversarial review on 2026-08-23 and were
amended for its findings, most notably: a read-class escalation denylist
closing four write-through-`read` smuggling routes (`SELECT setval(…)`
sequence templates, `SELECT set_config('default_transaction_read_only',
…)`, the legacy no-parentheses `EXPLAIN ANALYZE <dml>` form, and write
CTEs inside `COPY (…) TO`); an explicit mistakes-not-adversaries threat
model in the ADR replacing an overclaimed "unreachable" for GUC
overrides; fully fail-closed `unknown` classification for all
non-PostgreSQL `run_query` text instead of reusing the defeatable
`should_fetch_rows` heuristic; strict-prefix-only matching for
`[policy:…]` refusal tags (substring matches are spoofable via
user-controlled error text); apply-path confirmation summaries
synthesized from staged operations; explicit pass-through semantics for
`transaction`/`session` classes; and a corrected Scope list covering the
`managed.rs` production struct literals and five test-literal files that
would otherwise trip the plan's own STOP condition.

**Implementation evidence:**

- `src/components/connection-form/safety-fields.tsx` exposes environment,
  Safe Mode, and relational read-only controls.
- `src/components/environment-badge.tsx` and the workspace surfaces keep
  production identity visible where users act on a connection.
- `src/components/safety-confirm-dialog.tsx` and
  `src/lib/invoke-with-safety-confirmation.ts` provide the shared deliberate
  confirmation and retry boundary.
- `src/lib/store/edit-strategies.ts` applies read-only policy to edit
  affordances, while `src-tauri/src/safety/` remains the enforcement boundary.
- `src/components/workspace-overview/settings-tab.tsx` exposes the persisted
  confirmed-override audit.

**Remaining follow-ons outside the completed Plans 007/008 scope:**

- User-picked connection colors.
- Smart commit or manual transaction defaults for production.
- Safety behavior for scripts containing mixed safe and destructive statements.
- Typed policy errors for legacy command surfaces.

**Target outcome:** One backend-enforced safety policy applies to the SQL
editor, table grid, object editors, imports, generated scripts, and admin
actions. UI warnings explain policy but are not the enforcement boundary.

**Dependencies:** `PAR-001` transaction state and `PAR-003` mutation preview.

### PAR-005: Workspace persistence and global navigation

**Current state:** Partial.

**Evidence:**

- `src/lib/store/workspace-tabs.ts:88-89` initializes workspace tabs and the
  active tab empty.
- `src/components/command-palette/command-palette.tsx:26-100` searches only a
  capped portion of loaded store data.
- `src/components/connection-actions.tsx:18-55` exposes connect, disconnect,
  edit, and delete only.

**Missing pieces:**

- Autosave and restore for tabs, unsaved SQL, caret/selection, and layout.
- Restoration that distinguishes durable tab descriptors from ephemeral server
  sessions and transactions.
- SQL files, folders, recent files, and external-change handling.
- Split editors, split results, and multiple windows.
- Global Open Anything across connections, schemas, objects, actions, and
  saved queries.
- Complete metadata search and optional data search.
- Deep links to connections, objects, tables, and saved queries.
- Connection folders, groups, tags, colors, favorites, and recent targets.
- Duplicate connection, copy URI/DSN, URI import, and encrypted profile
  exchange.
- Predictable reconnect and tab rehydration after failure or restart.

**Target outcome:** Restarting dbunk restores the user's durable workspace
without falsely restoring live connections, transactions, or running queries.
Navigation can reach any known object without requiring it to be preloaded.

**Dependency:** Use `PAR-001` session descriptors, but never persist live
backend handles.

## P1: professional PostgreSQL workflows

### PAR-006: Connection security and organization

**Current state:** Partial. SSH and credential-storage depth are strong.

**Evidence:**

- `src/lib/store/types.ts:239-316` models relational connections and advanced
  fields.
- `src-tauri/src/postgres/pool.rs:15-50` supports only limited PostgreSQL TLS
  modes.
- Stored TCP keepalive is not applied to PostgreSQL connections.

**Missing pieces:**

- TLS require, verify-ca, and verify-full modes.
- CA, client certificate, and client key selection and validation.
- Trust-store and hostname-verification diagnostics.
- Applied TCP keepalive settings.
- Connection test details that separate DNS, tunnel, TLS, authentication, and
  database failures.
- Connection groups, tags, colors, favorites, and environment type.
- Duplicate, URL import/export, copy DSN, and secure connection bundles.
- IAM and external-secret integrations as later adapters.
- Driver manager, custom drivers, and network profiles if literal DBeaver
  parity is accepted.

**Target outcome:** dbunk can connect safely to certificate-enforced production
PostgreSQL deployments and can explain which transport stage failed.

### PAR-007: PostgreSQL object lifecycle management

**Current state:** Partial. Browsing breadth is strong; structured lifecycle
depth is not.

**Evidence:**

- `src-tauri/src/types.rs:1614-1643` includes broad PostgreSQL object kinds.
- `src-tauri/src/dispatch/relational.rs:511-718` frequently opens catalog SQL
  for non-table objects.
- `src/lib/ddl/postgres.ts:26-31` supports a limited set of queued column
  changes.
- `src/components/table-editor/specialized-editors.tsx` primarily generates SQL
  and opens it in the query editor.

**Missing pieces:**

- Structured create, inspect, alter, rename, and drop for databases and schemas.
- Complete table, column, primary key, foreign key, check, unique, and index
  lifecycle.
- Views and materialized views, including dependencies and refresh policy.
- Functions, procedures, aggregates, triggers, and event triggers.
- Sequences, types, enums, domains, and extensions.
- Roles, users, memberships, ownership, grants, and default privileges.
- Row-level security policies, tablespaces, partitions, rules, and comments.
- Existing-definition introspection in every editor.
- Explicit tagged literal-versus-expression defaults.
- SQL preview, transactional apply where PostgreSQL permits it, and clear
  non-transactional exceptions.
- Dependency-aware drop warnings and generated DDL.
- Overload-safe function/procedure identity.

**Target outcome:** Routine PostgreSQL administration no longer requires users
to leave dbunk or hand-write catalog queries, while generated SQL always remains
inspectable.

### PAR-008: Schema and data comparison with migration

**Current state:** Prototype.

**Evidence:**

- `src/components/workspace-overview/compare-tab.tsx:20-46` compares selected
  object-name sets.
- `src/components/workspace-overview/compare-tab.tsx:268-325` compares the first
  100 rows positionally.

**Missing pieces:**

- Cross-connection source and destination selection.
- Complete normalized metadata snapshots.
- Definition comparison for columns, constraints, indexes, views, routines,
  policies, grants, comments, and dependencies.
- Rename detection with explicit user confirmation.
- Dependency-ordered migration SQL.
- Selective include/exclude, SQL review, export, and apply.
- Keyed and unordered data comparison.
- Column and key mapping.
- Chunked hashing or another bounded large-table comparison strategy.
- Changed-column detail rather than generic row differences.
- Query-result comparison.
- Synchronization DML and dry-run summaries.
- Clear handling of irreversible or non-transactional operations.

**Target outcome:** Comparison is definition-aware, bounded, cross-connection,
and able to produce reviewable migration or synchronization SQL.

### PAR-009: Diagram editing and visual query building

**Current state:** Strong PostgreSQL relationship viewer, partial parity.

**Evidence:**

- `src/components/schema-relationship-map.tsx:207-322` supports multi-schema
  relationship rendering.
- `src/components/schema-map-toolbar.tsx:94-224` supports display modes,
  routing, reset, PNG, and SVG.
- `docs/adr/0015-visual-query-builder-scaffold-then-text.md` defines a visual
  query builder direction, but no implementation exists.

**Missing pieces:**

- Custom table subset picker.
- Search, minimap, and large-canvas navigation.
- Notes, annotations, grouping, and canvas sections.
- Virtual or user-drawn relationships.
- Alternate notation and print-ready export.
- Editing relationships and schema objects from a diagram.
- Cross-connection diagrams.
- Visual query builder for tables, joins, projections, filters, grouping,
  ordering, and limits.
- One-way scaffold-to-SQL handoff that respects ADR 0015.

**Target outcome:** Diagrams support intentional design and documentation, and
the query builder accelerates query scaffolding without creating a fragile
round-trip SQL parser.

### PAR-010: Transfer, DDL export, backup, and restore

**Current state:** Partial.

**Evidence:**

- `src/lib/export.ts:18-37` supports CSV, JSON, SQL, HTML, Markdown, text, and
  XLSX exports.
- `src/components/table-editor/data-import-wizard.tsx:35-158` supports CSV and
  multi-sheet XLSX import.
- `src/components/table-editor-panel.tsx:768-805` accumulates whole-table export
  pages in frontend memory.
- `src-tauri/src/postgres/mutations.rs:287-323` rebuilds a complete CSV payload
  before PostgreSQL COPY.
- `src-tauri/src/postgres/ddl.rs:53-221` generates relation-oriented DDL rather
  than a complete database definition.

**Missing pieces:**

- File-to-database and database-to-file streaming.
- Backpressure, cancellation, progress, and bounded memory.
- Delimiter, quote, escape, encoding, locale, and date/time controls.
- Error-row or reject-file output.
- Transform expressions and source-to-destination column mapping.
- Upsert/merge, truncate-first, batch tuning, and resumability.
- JSON, XML, and SQL import.
- XML and Parquet export, consistent with ADR 0017.
- Split-file and compression options.
- Correct MySQL and SQLite import targets or explicit capability hiding.
- Complete generated PostgreSQL database DDL for routines, sequences, types,
  triggers, policies, grants, comments, extensions, roles, and tablespaces.
- Saved backup/restore profiles, background progress, history, validation, and
  scheduling.
- Equivalent native backup integrations for accepted non-PostgreSQL engines.

**Target outcome:** Large transfers remain bounded and cancelable, capability
claims match real engine support, and users can distinguish generated DDL from
canonical native backups.

### PAR-011: PostgreSQL administration and observability

**Current state:** Partial.

**Evidence:**

- `src/components/workspace-overview/admin-tab.tsx:95-159` exposes sessions,
  locks, pending transactions, sizes, cache hit rate, and activity counts.
- `src-tauri/src/postgres/admin.rs:14-344` implements current PostgreSQL admin
  queries and cancel/terminate operations.

**Missing pieces:**

- `pg_stat_statements` slow-query digest.
- Historical query and relation-size trends.
- Replication, slots, publications, subscriptions, and lag.
- WAL, checkpoints, archiving, and connection headroom.
- Autovacuum and bloat diagnostics.
- Server logs and structured filtering.
- Tablespace usage, backup status, and scheduled-job visibility.
- Role and extension administration integrated with object lifecycle.
- Configurable live dashboards and saved metric boards.
- Query plan and execution-history correlation.
- Alerts or explicit threshold indicators.

**Target outcome:** dbunk can diagnose common PostgreSQL production issues
without becoming a full monitoring platform.

## P2: automation, platform, and engine breadth

### PAR-012: Reusable tasks, scheduling, and extensibility

**Current state:** Missing.

**Missing pieces:**

- Typed reusable task model for queries, transfers, backups, and maintenance.
- Composite tasks with ordered steps and failure policies.
- Scheduling and headless execution.
- Task history, logs, cancellation, retry, and secret-safe parameterization.
- Plugin manifest and versioned extension API.
- Driver or provider extension points.
- Sandboxed scripting or automation surface.
- Plugin discovery, install, enable/disable, update, and compatibility checks.
- Clear trust and permission model.

**Target outcome:** Repetitive workflows become durable tasks, and specialized
capabilities can grow without permanently expanding the core application.

### PAR-013: Desktop platform and release parity

**Current state:** Partial.

**Evidence:** The public release documentation currently emphasizes an unsigned
Apple Silicon macOS artifact.

**Missing pieces:**

- Signed and notarized macOS releases.
- Intel macOS support if retained as a product requirement.
- Signed Windows builds and installer behavior.
- Packaged Linux builds across an explicit supported matrix.
- Automatic updater with rollback-safe release metadata.
- Crash reporting and privacy-controlled diagnostics.
- Platform CI, installation tests, migration tests, and rollback tests.
- Multiple native windows and OS-level deep links.
- Platform-specific feature parity documentation.
- iOS companion only if literal TablePlus platform parity is accepted.

**Target outcome:** Supported desktop platforms install, update, restore state,
and recover from failed updates predictably.

### PAR-014: Non-PostgreSQL engine depth

**Current state:** Partial and intentionally asymmetric.

**Observed gaps:**

- MySQL and SQLite expose browsing and structure but not row mutation.
- Generic import routes toward unsupported MySQL and SQLite inserts.
- MySQL and SQLite relationship-map dispatch returns no graph despite available
  foreign-key metadata.
- MySQL, SQLite, and ClickHouse lack PostgreSQL-level object management,
  overview, admin, DDL export, backup, and restore.
- ClickHouse mutation support is intentionally engine-gated and asynchronous;
  it still lacks full lifecycle and operational parity.
- Managed local servers cover PostgreSQL and MySQL, not every supported engine.

**Missing pieces if multi-engine parity is accepted:**

- Per-engine capability contracts that hide unsupported UI paths.
- Safe mutations and imports for MySQL and SQLite.
- Relationship graphs for MySQL and SQLite.
- Native object identity, DDL, backup, explain, admin, and session behavior per
  engine.
- Engine-specific result types and editors.
- Driver-specific integration and recovery tests.

**Target outcome:** No workflow is advertised for an engine unless it works
end to end. Engine differences remain explicit rather than being flattened into
the PostgreSQL model.

### PAR-015: Literal DBeaver enterprise and ecosystem parity

**Current state:** Deferred pending a product boundary decision.

Literal parity would additionally require:

- DBeaver's wide driver matrix and custom-driver administration.
- Enterprise authentication, external secrets, policy, and audit controls.
- Team workspaces, shared configurations, access control, and collaboration.
- Cloud database and cloud-account explorers.
- Git-backed project assets and team distribution.
- A mature public plugin ecosystem and compatibility lifecycle.
- Advanced visual query building, schema migration, data comparison, charts,
  dashboards, debugger, and AI capabilities that vary by DBeaver edition.
- TablePlus platform-specific plugin, LLM, MCP, metrics, and mobile surfaces.

**Decision required:** Treat this as a separate product strategy. It should not
silently expand the PostgreSQL daily-driver program.

### PAR-016: Redis advanced-client depth

**Current state:** Strong standalone surface with advanced gaps.

**Missing pieces:**

- Sentinel and Cluster discovery, topology, and routing awareness.
- RediSearch, TimeSeries, Bloom, and other module-specific viewers.
- Scripting and Redis Functions workbench.
- MONITOR capture and filtering.
- Visual transaction builder.
- Geo map visualization.
- Cross-tab database switching. Today scanning can change DB while inspector
  and CLI remain on the connection default.
- Pub/Sub automatic reconnect and restored subscriptions.
- Rich typed multi-key comparison beyond strings.

**Target outcome:** Redis remains a first-class engine without distracting from
the PostgreSQL parity dependency chain.

## P1 documentation integrity

### PAR-017: Product claims and documentation accuracy

**Current state:** Partial. Several planning documents are stale or overstate
the implemented behavior.

**Known corrections:**

- `ROADMAP.md:48` says multi-schema diagrams are absent, but all-schema mode is
  implemented.
- `docs/PENDING_TASKS.md:34-36` and `docs/design/PHASES.md:52-57` still list
  one-to-one detection and multi-schema canvases as pending even though both
  are implemented and tested.
- `ROADMAP.md:103` says XLSX import selects only the first sheet, but the sheet
  selector is implemented.
- `ROADMAP.md:104` calls PostgreSQL COPY import streaming, but the current path
  materializes all parsed rows and rebuilds a complete payload.
- `docs/design/PHASES.md:64` calls whole-table export streaming, but the frontend
  accumulates all pages before serialization.
- `designs/FOLLOWUPS.md:88-93` says pagination is client-side; backend paging is
  implemented, although filtering remains page-local and some controls are
  unwired.
- `ROADMAP.md:77` describes literal bind substitution as parameterized
  execution.
- `ROADMAP.md:107-111` presents specialized SQL generators as complete object
  editors.
- `docs/design/PHASES.md:162` requires schema diff plus migration SQL, while the
  implementation compares object names and emits no migration SQL.
- Generic MySQL and SQLite import affordances exceed backend write capability.
- Generated database DDL can be mistaken for a complete schema backup even
  though it aggregates relation DDL only.

**Missing pieces:**

- One capability matrix derived from executable behavior.
- Clear distinction among implemented, partial, experimental, and planned.
- Engine-specific support markers.
- Automated or review-time checks that force capability claims to change when
  their implementation changes.
- Archive or correction of superseded plans and follow-ups.

**Target outcome:** Roadmaps and user-facing capability claims never imply
correctness, streaming, parameter binding, editor depth, or engine support that
the implementation does not provide.

## Existing strengths to preserve

Parity work should reuse rather than replace these credible foundations:

- Saved connections with OS keychain, encrypted SQLite, or plain local storage.
- Deep SSH, bastion, proxy, fingerprint, compression, and keepalive modeling.
- Docker-backed local PostgreSQL and MySQL servers.
- Broad PostgreSQL navigator and table metadata.
- Monaco SQL editing, completion, snippets, formatting, history, saved queries,
  and visual EXPLAIN.
- PostgreSQL row-identity safety and transactional parameterized mutations.
- Specialized JSON, array, geometry, and related cell editors.
- Foreign-key drilldown and detailed table sub-tabs.
- CSV/XLSX import, multi-format export, PostgreSQL dump/restore, table copy, and
  constraint-aware data seeding.
- Multi-schema relationship visualization with cardinality, persisted layout,
  and PNG/SVG export.
- PostgreSQL sessions, locks, pending transactions, and backend cancellation or
  termination from the admin surface.
- Deep Redis browsing, editing, CLI, Pub/Sub, server, safety, and transaction
  workflows.

## Recommended planning sequence

1. `PAR-001`: query-session foundation delivered by Plans 001 and 002;
   remaining execution follow-ons stay tracked above.
2. `PAR-002`: server-backed table browsing delivered by Plans 003 and 004
   through commit `ecefce8`.
3. `PAR-003`: editable query results and generated DML review delivered by
   Plans 005 and 006 through `4e52c8a`; deep
   editors, Quick Look, batch paste, and copy formats remain register
   scope.
4. `PAR-004`: backend-enforced production safety delivered by Plans 007 and
   008; both are implemented and merged to `main`.
5. **Selected next:** `PAR-005`, durable workspace restoration and global
   navigation.
6. `PAR-006` and `PAR-007`: secure connections and full PostgreSQL object
   lifecycle.
7. `PAR-008` through `PAR-011`: compare, diagrams/query design, transfer, and
   administration.
8. Revisit platform, automation, non-PostgreSQL breadth, and literal enterprise
   parity only after the daily-driver foundation is stable.

## Official competitor references

### DBeaver

- <https://dbeaver.com/docs/dbeaver/SQL-Editor/>
- <https://dbeaver.com/docs/dbeaver/SQL-Execution/>
- <https://dbeaver.com/docs/dbeaver/Auto-and-Manual-Commit-Modes/>
- <https://dbeaver.com/docs/dbeaver/Data-Editor/>
- <https://dbeaver.com/docs/dbeaver/ER-Diagrams/>
- <https://dbeaver.com/docs/dbeaver/Structure-and-Data-Compare/>
- <https://dbeaver.com/docs/dbeaver/Task-Management/>
- <https://dbeaver.com/docs/dbeaver/Task-Scheduler/>
- <https://dbeaver.com/edition/>

### TablePlus

- <https://docs.tableplus.com/getting-started>
- <https://docs.tableplus.com/gui-tools/manage-connections>
- <https://docs.tableplus.com/gui-tools/code-review-and-safemode/safe-mode>
- <https://docs.tableplus.com/gui-tools/code-review-and-safemode/code-preview>
- <https://docs.tableplus.com/gui-tools/filter>
- <https://docs.tableplus.com/query-editor/streaming-results-and-async-loading>
- <https://docs.tableplus.com/query-editor/split-results-into-tabs>
- <https://docs.tableplus.com/gui-tools/the-interface/quick-look>
- <https://docs.tableplus.com/gui-tools/import-and-export>
- <https://docs.tableplus.com/gui-tools/backup-and-restore>
- <https://docs.tableplus.com/gui-tools/metrics-board>
- <https://docs.tableplus.com/utilities/plugin>
