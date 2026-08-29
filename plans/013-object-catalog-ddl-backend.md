# Plan 013: PostgreSQL object catalog and DDL workflow backend — typed object model, rich introspection, drop impact, and backend-owned preview/apply

> **Executor instructions**: Plans 001–012 must be `DONE` in
> `plans/README.md` before starting (012 is `READY FOR REVIEW` at
> authoring time — wait for the reviewer to record it). Follow this plan
> step by step. This is a **dark** plan: no user-visible UI change may
> land here — new commands, types, and TS union mirrors ship
> unreferenced by any rendering path. Unlike Plan 011, the frontend
> touch really is type-only: every TS type here is a *new* union, not a
> new arm on an existing exhaustive switch, so no formatter or decoder
> needs an arm to keep `typecheck` green. Run every verification command
> and confirm the expected result before moving on. Update this plan's
> README row after each step and mark `READY FOR REVIEW` after all
> gates. A reviewer/operator records `DONE: <completion SHA>` after an
> authorized commit.
>
> **Fresh-start drift check**:
>
> ```sh
> git status --short -- src src-tauri infrastructure plans/README.md
> git log --oneline -1
> grep -rn "ObjectKind\|object_kind" src-tauri/src | wc -l   # expect 0 at b45e294
> grep -rn "pg_depend" src-tauri/src | wc -l                 # expect 0 at b45e294
> ls infrastructure/test-db/postgres/                        # 004_*.sql must not exist yet
> ```
>
> Expected on a fresh run: clean tree at or after `b45e294`. A
> load-bearing mismatch with the excerpts below is a STOP condition.

## Status

- **Priority**: P1
- **Effort**: L
- **Risk**: HIGH (generated DDL executes against user databases; a wrong
  quote, a missing `RESTRICT`, or a mis-grouped transaction destroys
  data or leaves schemas half-migrated)
- **Depends on**: Plans 001–012 complete
- **Category**: foundation (dark)
- **Planned at**: commit `b45e294`, 2026-08-29
- **Amended**: 2026-08-29 (pre-execution review) — see
  `## Review correction record` at the end of this file for the list of
  contract changes; the sections below already incorporate them.
- **Gap**: `PAR-007` in `plans/parity-gap-register.md`

## Why this matters

`PAR-007`'s audit found browsing breadth without lifecycle depth. The
working tree at `b45e294` is starker than the register's phrasing:

- The backend collects **fourteen** PostgreSQL object kinds
  (`SchemaExplorer`, `src-tauri/src/types.rs:2020-2051`) as bare
  `Vec<String>` name lists — no OID, no owner, no comment, no
  definition, and function names pre-concatenated with their identity
  arguments into an unparseable display string. The frontend consumes
  four of them (`src/lib/open-anything.ts:117-137`); the navigator
  renders **tables only** (`src/components/workbench/database-navigator.tsx`).
  Ten kinds are fetched over the wire and dropped on the floor.
- There is no `ObjectKind` type anywhere in the Rust backend
  (`grep -rn "ObjectKind" src-tauri/src` → zero hits). The one prior
  attempt, `src/lib/object-navigator.ts`, was reverted (recover the
  vocabulary with `git show 9370a5e^:src/lib/object-navigator.ts`).
- DDL generation lives in the frontend (`src/lib/ddl/postgres.ts:18-38`)
  and covers exactly six column-level `ALTER TABLE` changes. No
  constraint, index, view, sequence, enum, schema, or comment operation
  exists anywhere. Defaults are an untagged `string | null` run through
  a keyword-allowlist heuristic (`src/lib/ddl/shared.ts:24-57`) — the
  register's "explicit tagged literal-versus-expression defaults" gap.
- The single apply path, `execute_ddl`
  (`src-tauri/src/postgres/ddl.rs:404-428`), wraps the *whole*
  multi-statement string in one `BEGIN`/`COMMIT`, so
  `CREATE INDEX CONCURRENTLY` — which
  `src/components/table-editor/specialized-editors.tsx:186` generates
  with `concurrently: true` as the default — can never run through it.
  Errors come back as one opaque `String`: no SQLSTATE, no position, no
  statement attribution, and rollback failure is swallowed
  (`let _ = conn.execute("ROLLBACK")`).
- Dependency-aware drops have no data source: nothing queries
  `pg_depend`, `pg_description`, `pg_get_functiondef`, `pg_enum`,
  `pg_sequences`, or ownership.
- The schema-explorer fetch is the last PostgreSQL read on sqlx-**Any**
  (`src-tauri/src/dispatch/relational.rs:513-518`), issuing N schemas ×
  11 sequential single-column queries plus 3 database-wide ones —
  outside the `ResolvedTls`/driver-options path every other PG read
  uses.

This plan lands the non-visual foundation Plan 014 activates: a typed
object model with overload-safe routine identity, a batched object
catalog on the native driver, per-object description with reconstructed
DDL, drop-impact introspection over `pg_depend`, and a backend-owned
DDL workflow — typed operations in, inspectable per-statement preview
out, gated transactional apply with typed errors — mirroring the Result
Mutation analyze → preview → apply shape (ADR-0023) instead of the
frontend-generates-string shape of `execute_ddl`.

## Reconciliation against the register (read before scoping questions)

`PAR-007` is `XL`; this pair deliberately delivers its core and records
the rest. **In scope across Plans 013/014 (PostgreSQL only, ADR-0001):**

- Typed catalog, viewers-grade description, comments, and owners for:
  schemas, tables, views, materialized views, foreign tables, sequences,
  functions, procedures, aggregates, types (enum/composite/range/
  multirange), domains, and extensions. Event triggers, roles, and
  tablespaces stay **list-only** (named in the catalog, no description).
- Structured lifecycle operations: schemas (create/rename/drop/comment);
  tables (rename/drop/comment; the six column changes moved
  backend-side with tagged literal-vs-expression defaults and a `USING`
  clause; primary key / unique / foreign key / check add and constraint
  drop; index create — including `CONCURRENTLY` — and drop); views and
  materialized views (create / create-or-replace from a SQL body,
  rename, drop, comment); sequences (create/alter/rename/drop/comment);
  enums (create, add value, rename value, drop, comment).
- SQL preview for every operation, transactional apply where PostgreSQL
  permits it with disclosed non-transactional exceptions, RESTRICT-by-
  default drops with dependency impact shown first, overload-safe
  function/procedure/aggregate identity, and a typed DDL error union
  (the `PAR-007` slice of the legacy typed-error migration; the rest of
  that migration stays `PAR-014`).

**Deferred, staying in the register with rationale:**

- **Database create/alter/drop** — `CREATE DATABASE` cannot run inside a
  transaction and requires connecting to a different maintenance
  database than the one the pool is bound to; it needs its own
  connect-and-execute path and UX. Schemas deliver the daily namespace
  need.
- **Roles, users, memberships, ownership transfer, grants, and default
  privileges** — a security-administration surface with its own model
  (ACL parsing, `pg_default_acl`) and its own blast radius; belongs with
  the `PAR-011` administration work. The existing GRANT generator panel
  stays as-is.
- **Row-level security policy lifecycle, rules, partitions,
  tablespaces, event triggers** — same `PAR-011`-adjacent reasoning;
  the RLS generator panel stays.
- **Trigger lifecycle** — triggers are table-scoped programmability;
  structured trigger management belongs with a table programmability
  viewer that also covers rules/policies/partitions, not with this
  pair's schema-level catalog. The trigger generator panel stays.
- **Structured create/alter forms for functions, procedures, and
  aggregates** — routine bodies are SQL; the honest v1 is the
  description viewer (full `pg_get_functiondef` output) plus an
  edit-in-SQL-editor scaffold (Plan 014). Structured drop with impact
  **is** delivered.
- **Extension create/drop** — listed and described (version, schema);
  install/remove touches server-side packaging concerns and superuser
  expectations. Follow-on.
- **A full create-table designer** — multi-column table creation is its
  own mock-and-form project; column/constraint/index lifecycle on
  existing tables is delivered, and `CREATE TABLE` remains a query-tab
  task. Follow-on plan candidate.
- **Materialized-view refresh policy** (schedules) — `PAR-012` task
  territory; on-demand `refresh_materialized_view` already exists.
- **Typed-error migration for the remaining legacy commands** —
  explicitly `PAR-014`; this plan types only the new object-DDL surface.

## Current state (verified at `b45e294`)

### Backend

- `src-tauri/src/types.rs:2020-2051` — `SchemaExplorer`: `name`,
  `tables`, and twelve optional `Vec<String>` kind lists
  (`views` … `tablespaces`), the last three database-scoped and emitted
  only in a synthetic `"Database"` pseudo-schema. Function/procedure/
  aggregate names arrive pre-joined with
  `pg_get_function_identity_arguments`.
- `src-tauri/src/dispatch/relational.rs:510-718` —
  `fetch_schema_explorer_sqlx`: opens an `AnyConnection` (`:513-518`),
  PostgreSQL arm at `:521-645` runs the per-schema kind queries
  (`pg_tables` `:529`, `pg_views` `:535`, `pg_matviews` `:541`,
  `information_schema.sequences` `:547`, `…foreign_tables` `:553`,
  `pg_proc` by `prokind` `:559-571`, `pg_type` `typtype IN
  ('c','e','r','m')` `:577`, `…domains` `:583`, `pg_extension` `:589`)
  plus database-wide `pg_event_trigger` `:610`, `pg_roles` `:616`,
  `pg_tablespace` `:622`. This command stays untouched here — Plan 014
  decides how the PG navigator consumes the new catalog.
- `src-tauri/src/postgres/ddl.rs` — `export_ddl` renderers
  (`pg_get_viewdef` `:101`, columns via `pg_attribute` + `format_type` +
  `pg_get_expr` `:120-138`, `pg_get_constraintdef` `:167`, `pg_indexes`
  `:196`); `relation_names` `:222-243` filters
  `relkind IN ('r','p','v','m','f')` so schema/database exports silently
  omit every non-relation kind; `execute_ddl` `:404-428` (pooled sqlx
  connection, blanket `BEGIN`/`COMMIT`, string errors, best-effort
  rollback).
- `src-tauri/src/commands/relational.rs:254-281` — `execute_ddl` /
  `execute_ddl_inner`: `with_gated_active_connection(…,
  WriteIntent::Ddl, …)` with **no** `classify_script` call — `DROP
  TABLE` and `ADD COLUMN` are indistinguishable to the gate.
- Safety: `src-tauri/src/safety/policy.rs` — `WriteIntent` (`:24-38`,
  twelve variants incl. `Ddl`), `assert_permitted` `:104-128`,
  `read_only_permits` `:152-163` (never permits `Ddl`),
  `protected_requires_confirmation` `:165-178` (`Ddl` → true),
  `SafetyRefusal::fold` `:51-62`, `AuditDisposition::RequiredAfterSuccess`.
  Gate coverage is pinned by `commands/safety.rs:101-148`
  (`write_intents()` + the strict-refusal loop over legacy gates) and
  `LegacyCommand::ALL: [Self; 12]` at `commands/relational.rs:886-932`.
  The fifteen gated surfaces are enumerated in `plans/README.md:94`.
- Typed-error template: `result_mutation/protocol.rs:339-388`
  (`#[serde(rename_all = "camelCase", rename_all_fields = "camelCase",
  tag = "kind")]`, `policyBlocked`/`policyNeedsConfirmation` arms via
  `SafetyRefusal::fold`, `database { code, message, severity, position,
  opIndex }`); apply-side gating at `result_mutation/mod.rs:197-215`
  with the audit recorded in `commands/result_mutation.rs:66-79`.
- Pool substrate: `postgres/pool.rs:51-123` (cached native `PgPool` per
  connection id, full `ResolvedTls`); the dedicated tokio-postgres
  driver serves only the three actors. `find_connection`
  (`commands/mod.rs:47-56`) hydrates credentials and resolves tunnels.
- Storage: `MIGRATIONS` latest version is **18** (`storage.rs:420-429`).
  **This plan needs no migration** — nothing here persists app-side
  state.
- Live fixtures: plain PostgreSQL on 15432, TLS fixture on 15433, both
  initialized from `infrastructure/test-db/postgres/*.sql` (`001…003`;
  the TLS image *sources* its init script and applies the shared SQL —
  Plan 011 Step 6 correction). Next free init file is `004`.

### Frontend (type-only touch points for this dark plan)

- `src/lib/store/types.ts:990-1007` — `SchemaExplorer` mirror.
- `src/lib/ddl/postgres.ts`, `src/lib/ddl/shared.ts` — untouched here;
  Plan 015 decides their fate (ClickHouse keeps the frontend builder).
- `src/lib/decode-transport-error.ts`, `src/lib/safety-policy.ts:153-181`
  (`SharedTransportError`, `formatSharedTransportError`) — untouched;
  new formatters are Plan 014's.

## Decided architecture

### 1. Object model (`src-tauri/src/postgres/objects.rs` + `types.rs`)

```rust
#[derive(Debug, Serialize, Deserialize, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum PgObjectKind {
    Schema, Table, View, MaterializedView, ForeignTable, Sequence,
    Function, Procedure, Aggregate, Type, Domain, Extension,
}

#[derive(Debug, Serialize, Deserialize, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum PgTypeClass { Enum, Composite, Range, Multirange }

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PgObjectRef {
    pub kind: PgObjectKind,
    /// None only for `Schema` itself. Extensions carry their *placement*
    /// schema (the catalog groups them per schema and the navigator /
    /// tab key need it); renderers ignore it — `DROP EXTENSION name`
    /// and `COMMENT ON EXTENSION name` are database-scoped.
    pub schema: Option<String>,
    pub name: String,
    /// `pg_get_function_identity_arguments` output. Required for
    /// Function/Procedure/Aggregate (overload-safe identity), None for
    /// every other kind. Rendered as `name(identity_args)` in DROP /
    /// COMMENT / describe lookups.
    pub identity_args: Option<String>,
}
```

- The kind vocabulary is the eleven schema-scoped kinds plus `Schema`;
  `Type` carries its class as data rather than four kinds, so the
  navigator groups one "Types" section and DROP renders `DROP TYPE` for
  all four classes.
- **Event triggers, roles, and tablespaces are not kinds.** They are
  list-only in this pair (no describe, no drop, no palette entry), so
  they appear in the catalog as plain `PgCatalogEntry` lists and never
  form a `PgObjectRef`. This removes the `DescribeUnsupported` arm and
  three exhaustive-match obligations that existed only to say "no";
  promoting one of them to a kind later is the three-decision change in
  the maintenance notes.
- Function display names are no longer pre-concatenated: the catalog
  returns `name` and `identity_args` separately; presentation joins
  them.
- Quoting: **reuse the crate's existing `quote_double`**
  (`src-tauri/src/lib.rs:80`, already used by `postgres/ddl.rs` whose
  table renderer this plan reuses) for identifiers, and add
  `quote_literal` (PostgreSQL `E'…'` with apostrophe and backslash escaping)
  beside it in `lib.rs`. Do not
  create a second identifier quoter. Every renderer in this plan uses
  these two — no format-string quoting anywhere.

### 2. `load_pg_object_catalog` — batched catalog on the native pool

`load_pg_object_catalog(connection_id) -> Result<PgObjectCatalog, PgObjectError>`

```rust
#[serde(rename_all = "camelCase")]
pub(crate) struct PgObjectCatalog {
    pub schemas: Vec<PgSchemaObjects>,       // user schemas, sorted
    pub event_triggers: Vec<PgCatalogEntry>, // database-scoped, list-only (not a PgObjectKind)
    pub roles: Vec<PgCatalogEntry>,
    pub tablespaces: Vec<PgCatalogEntry>,
    /// Any per-kind list hit `CATALOG_KIND_CAP` (2000) and was cut.
    /// Names the (schema, kind) pairs that were truncated so the UI can
    /// say so instead of silently showing a partial group.
    pub truncated: Vec<PgCatalogTruncation>, // { schema: Option<String>, kind: String }
}
pub(crate) struct PgSchemaObjects {
    pub name: String,
    pub tables: Vec<PgCatalogEntry>, pub views: Vec<PgCatalogEntry>,
    pub materialized_views: Vec<PgCatalogEntry>, pub foreign_tables: Vec<PgCatalogEntry>,
    pub sequences: Vec<PgCatalogEntry>, pub functions: Vec<PgCatalogEntry>,
    pub procedures: Vec<PgCatalogEntry>, pub aggregates: Vec<PgCatalogEntry>,
    pub types: Vec<PgCatalogEntry>, pub domains: Vec<PgCatalogEntry>,
    pub extensions: Vec<PgCatalogEntry>,
}
#[serde(rename_all = "camelCase")]
pub(crate) struct PgCatalogEntry {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")] pub identity_args: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")] pub comment: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")] pub type_class: Option<PgTypeClass>,
}
```

- Runs on the **native sqlx PG pool** (`postgres::pool`), not sqlx-Any:
  one query per kind across *all* schemas, grouped in Rust. ~12 round
  trips total instead of N×11+3. Comments come from `obj_description`
  in the same queries.
- Schema filter **matches the existing explorer exactly**:
  `n.nspname <> 'information_schema' AND n.nspname NOT LIKE 'pg\_%'`
  (`dispatch/relational.rs:524`). Not the `NOT IN (…)` form — that
  admits `pg_temp_*`/`pg_toast_temp_*`.
- **Extension members are excluded** from every schema-scoped kind
  query: `NOT EXISTS (SELECT 1 FROM pg_depend d WHERE d.classid =
  <catalog>::regclass AND d.objid = <oid> AND d.deptype = 'e')`. The
  fixture already installs `pgcrypto`, `citext`, and `btree_gist` into
  `public` (`infrastructure/test-db/postgres/001_schema_and_seed.sql:3-5`);
  without this filter `public.functions` carries hundreds of extension
  internals and the navigator is unusable on any PostGIS database.
  Extensions themselves still appear under `extensions`.
- **Bounded**: each per-(schema, kind) list is fetched with
  `LIMIT CATALOG_KIND_CAP + 1` semantics (cap 2000, sorted by name),
  cut to the cap, and the overflow recorded in `truncated`. The catalog
  is fetched on every PostgreSQL connect in Plan 014; an unbounded
  `pg_roles` or a 10k-routine schema must not be serialized wholesale
  over IPC.
  The schema list itself is capped first; every schema-scoped kind query
  binds only those retained schema names, so objects in omitted schemas do
  not consume database work before Rust discards them.
- PostgreSQL-only: other engines return
  `PgObjectError::UnsupportedEngine` (actor precedent). The legacy
  `load_schema_explorer` is untouched **in this plan**; Plan 014
  switches every PostgreSQL consumer onto the catalog and then deletes
  the sqlx-Any PostgreSQL arm (`dispatch/relational.rs:521-652`) in its
  truth pass as a pre-authorized backend amendment. MySQL/SQLite/
  ClickHouse keep `load_schema_explorer`.
- Registered in `lib.rs` next to `load_schema_explorer`; command wrapper
  in a new `commands/pg_objects.rs` following
  `commands/result_mutation.rs` (find_connection → typed error fold; no
  gate — read-only).

### 3. `describe_pg_object` — structured description + reconstructed DDL

`describe_pg_object(connection_id, reference: PgObjectRef) -> Result<PgObjectDescription, PgObjectError>`

```rust
#[serde(rename_all = "camelCase")]
pub(crate) struct PgObjectDescription {
    pub reference: PgObjectRef,
    pub owner: Option<String>,
    pub comment: Option<String>,
    /// Reconstructed DDL. None only where PostgreSQL cannot render it
    /// (aggregates: pg_get_functiondef raises for prokind='a'; facts
    /// still describe them).
    pub definition_sql: Option<String>,
    pub facts: PgObjectFacts,
}
#[serde(tag = "kind", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub(crate) enum PgObjectFacts {
    Schema,
    Table,                                  // structure stays load_table_structure's job
    View { definition: String },
    MaterializedView { definition: String, populated: bool },
    ForeignTable { server: String },
    Sequence { data_type: String, start: String, increment: String,
               min_value: String, max_value: String, cycle: bool, cache: String,
               last_value: Option<String>,
               owned_by: Option<String> },   // "schema.table.column"
    Routine { language: String, returns: Option<String>,
              volatility: Option<String>, arguments: String },
    Type { class: PgTypeClass,
           enum_labels: Option<Vec<String>>,          // Enum, enumsortorder order
           attributes: Option<Vec<PgTypeAttribute>>,  // Composite
           subtype: Option<String> },                 // Range/Multirange
    Domain { base_type: String, not_null: bool,
             default_value: Option<String>, checks: Vec<String> },
    Extension { version: String, schema: String },
}
```

- `definition_sql` sources: view/matview `pg_get_viewdef` with its trailing
  terminator removed before it is wrapped as
  `CREATE [MATERIALIZED] VIEW … AS`; routines `pg_get_functiondef`
  (aggregates → `None`); sequences rendered from `pg_sequences`; enums
  rendered `CREATE TYPE … AS ENUM (…)` from `pg_enum` ordered by
  `enumsortorder`; composites rendered from attributes; domains
  rendered with `pg_get_constraintdef` checks; extensions
  `CREATE EXTENSION …`; tables reuse the existing `export_ddl` table
  renderer (`postgres/ddl.rs`) — one renderer, not two.
  Foreign tables preserve per-column FDW options, explicit non-default
  collations, defaults, nullability, and CHECK constraints as well as table
  options and server identity.
- Event triggers/roles/tablespaces cannot be described — they are not
  kinds and cannot form a `PgObjectRef` (§1), so there is no
  `DescribeUnsupported` arm to render.
- A described object that no longer exists →
  `PgObjectError::ObjectNotFound { reference }` (rename/drop races).
- Run every description in a `REPEATABLE READ READ ONLY` transaction;
  several object kinds require multiple catalog queries and must describe
  one snapshot. Range reconstruction preserves the qualified multirange
  name, subtype opclass, collation, canonical function, and subtype-diff
  function. Sequence ownership follows both serial (`deptype = 'a'`) and
  identity (`deptype = 'i'`) dependencies, is fetched as separate
  schema/table/column fields, and each identifier is quoted without parsing a
  display string. Sequence quantities are decimal strings on the wire so the
  complete PostgreSQL `bigint` range never passes through a JavaScript number.

### 4. `load_pg_drop_impact` — dependency-aware drops

`load_pg_drop_impact(connection_id, reference) -> Result<PgDropImpact, PgObjectError>`

```rust
#[serde(rename_all = "camelCase")]
pub(crate) struct PgDropImpact {
    pub dependents: Vec<PgDropDependent>,  // transitive closure, capped at 200
    pub truncated: bool,
}
#[serde(rename_all = "camelCase")]
pub(crate) struct PgDropDependent {
    pub object_type: String,   // pg_identify_object.type, verbatim
    pub identity: String,      // pg_identify_object.identity, verbatim
    pub depth: u32,            // 1 = depends on the target directly
}
```

- **The closure is transitive, because `CASCADE` is transitive.** A
  deterministic eight-level breadth-first walk over `pg_depend` is seeded
  with the target's `(classid, objid, objsubid)`. Each level follows rows
  with `deptype IN ('n','a','i')` and the exact subobject predicate
  `(walk.sub_id = 0 OR dependency.refobjsubid = walk.sub_id)`, normalizes
  rewrite addresses, deduplicates, excludes visited addresses, fetches 202,
  and retains at most 201. The retained working set is therefore globally
  bounded at 1,609 addresses including the root; seeing candidate 202 or a
  dependency beyond depth 8 saturates `truncated`. Resolve each row through
  `pg_identify_object(classid, objid, objsubid)`, and **resolve
  `pg_rewrite` dependents to their owning view** (a view depends on its
  base table via its rewrite rule — reporting `rule _RETURN on …` would
  be useless; report the view, and keep walking *from the view* so
  `orders → orders_view → orders_mat` reports both). Deduplicate by
  identity (keep the smallest depth) before sorting by depth then identity
  and applying the final 201-row probe. Cap at 200 with a disclosed
  `truncated` flag; address saturation remains disclosed even if several
  addresses collapse to fewer than 200 identities. A direct-only query would
  make Plan 014's "Also drop N dependent objects — CASCADE" copy an
  undercount by construction.
- Resolve the root, walk dependencies, and identify results in one
  `REPEATABLE READ READ ONLY` transaction so every depth comes from the same
  catalog snapshot.
- This is a read; ungated. It informs the UI — generation still
  defaults to `RESTRICT`, and PostgreSQL remains the enforcement
  boundary (mistakes-not-adversaries framing, same as ADR-0024/0025).

### 5. DDL Plan: typed operations, pure generation (`postgres/object_ddl.rs`)

The operation vocabulary (one serde-tagged union, `tag = "op"`,
camelCase fields):

```text
CreateSchema { name }
RenameObject { reference, new_name }        # schema/table/view/matview/sequence via ALTER <kind> … RENAME TO
DropObject   { reference, cascade: bool }   # per-kind syntax; routines render name(identity_args);
                                            # RESTRICT rendered explicitly when cascade=false
SetComment   { target, comment: Option<String> }   # target = PgCommentTarget: Object(PgObjectRef)
                                                   # | Column { schema, table, column }; None → COMMENT … IS NULL
AddColumn        { schema, table, column: NewColumnSpec }
DropColumn       { schema, table, name, cascade }
RenameColumn     { schema, table, name, new_name }
AlterColumnType  { schema, table, name, new_type, using: Option<String> }
SetColumnNullable{ schema, table, name, nullable }
SetColumnDefault { schema, table, name, default: Option<PgDefaultValue> }
AddPrimaryKey { schema, table, name: Option<String>, columns }
AddUnique     { schema, table, name: Option<String>, columns }
AddForeignKey { schema, table, name: Option<String>, columns,
                referenced_schema, referenced_table, referenced_columns,
                on_update, on_delete, deferrable, initially_deferred, not_valid }
AddCheck      { schema, table, name: Option<String>, expression, not_valid }
DropConstraint{ schema, table, name, cascade }
CreateIndex { schema, table, name: Option<String>, unique, method,
              columns: Vec<PgIndexColumn { expression, descending }>,
              include, where_predicate, concurrently }
                                            # name=None → generator derives `<table>_<col1>_<col2>_idx`
                                            # (truncated to 63 bytes) and ALWAYS renders it explicitly,
                                            # so a failed CONCURRENTLY build can be checked for residue
DropIndex   { schema, name, concurrently, cascade }
CreateView             { schema, name, or_replace, sql_body }
CreateMaterializedView { schema, name, sql_body, with_data }
CreateSequence { schema, name, data_type: Option<String>,
                 start/increment/min/max/cache: Option<String>, cycle: Option<bool> }
AlterSequence  { schema, name,
                 restart_with/increment_by/min_value/max_value/cache: Option<String>,
                 cycle: Option<bool> }
CreateEnum      { schema, name, labels }
AddEnumValue    { schema, name, value, position: Option<Before{neighbor}|After{neighbor}> }
RenameEnumValue { schema, name, from, to }
```

```rust
#[serde(tag = "kind", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub(crate) enum PgDefaultValue { Literal { value: String }, Expression { sql } }
```

`PgDefaultValue::Literal` is always quoted with `quote_literal`;
`Expression` remains SQL but is enclosed in renderer-owned parentheses so
tokens such as `0 NOT NULL` cannot become an `ADD COLUMN` option. Column type
fragments reject top-level constraints/options while retaining PostgreSQL's
multiword, qualified, quoted, typmod, and array type syntax. Sequence types
are restricted to PostgreSQL's supported smallint/integer/bigint names and
their `pg_catalog` equivalents. This retires the
`RAW_DEFAULT_KEYWORDS` heuristic for the backend path and closes the
register's tagged-defaults gap. `AlterColumnType.using` closes the
missing-`USING` gap of `postgres.ts:71`.
Every sequence numeric string must parse as a signed 64-bit integer before
rendering; the string representation prevents both SQL injection and
JavaScript precision loss.

**What crosses IPC, honestly stated.** The vocabulary does carry SQL
*fragments*: `sql_body` (views/matviews), `AddCheck.expression`,
`PgIndexColumn.expression`, `where_predicate`, `AlterColumnType.using`,
and `PgDefaultValue::Expression.sql`. These are unavoidable — a view
body *is* SQL — so the contract is not "never SQL over IPC" but
**"never a whole statement over IPC, and every generated statement is
verified to be exactly one statement of the expected class before it
runs."** Concretely, `generate_object_ddl` runs
`classify_script(&statement.sql)` (`src-tauri/src/postgres/sql_class.rs:126`;
the lexer handles dollar quoting) over each rendered statement and:

- more or fewer than one statement → `InvalidOp { op_index, reason:
  "fragment contains a statement boundary" }` (this is what stops a
  `sql_body` of `SELECT 1; DROP TABLE x` from running both), and
- `destructive` becomes `shape_destructive || class.destructive`, so a
  fragment that smuggles a destructive keyword past the shape-based
  classifier is still flagged.

Because generation is pure, this check costs nothing at preview time
and runs again at apply (apply regenerates).

`generate_object_ddl(ops: &[PgObjectOp]) -> Result<DdlPlanPreview, PgObjectError>`
— **pure**, no I/O:

```rust
#[serde(rename_all = "camelCase")]
pub(crate) struct DdlPlanPreview {
    pub statements: Vec<PlannedStatement>,
    pub groups: Vec<StatementGroup>,   // ordered, covering all statements
}
#[serde(rename_all = "camelCase")]
pub(crate) struct PlannedStatement {
    pub sql: String,
    pub summary: String,       // "Drop view lifecycle.orders_view (RESTRICT)"
    pub destructive: bool,     // drops, cascade anything, AlterColumnType, SET NOT NULL
    pub transactional: bool,   // false only for CONCURRENTLY index ops here
}
#[serde(tag = "kind", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub(crate) enum StatementGroup {
    Atomic { statement_indexes: Vec<usize> },       // one transaction, all-or-nothing
    Standalone { statement_index: usize },          // non-transactional, runs alone
}
```

- Grouping rule: contiguous transactional statements form one `Atomic`
  group; each non-transactional statement is `Standalone`. In this
  vocabulary only `CreateIndex { concurrently: true }` and
  `DropIndex { concurrently: true }` are non-transactional. The preview
  is the disclosure surface: Plan 014 renders group boundaries and the
  "runs outside a transaction" caveat.
- Validation is typed: empty names, empty column lists, an
  `identity_args`-less routine drop, a `RenameObject` on a kind without
  a rename renderer (only schema/table/view/matview/sequence rename —
  Plan 014 hides Rename for the others), a statement-boundary inside a
  fragment (above) → `PgObjectError::InvalidOp { op_index, reason }` —
  never a panic, never silently skipped.
- Destructive classification — **criterion: the statement can lose or
  reject existing data, or requires `cascade`**. It supersets today's
  frontend `classifyDestructive` (`drop`, `set_type`,
  `set_nullable: false`) with every `DropObject`/`DropColumn`/
  `DropConstraint`/`DropIndex`, every `cascade: true`, and
  `AddPrimaryKey`/`AddUnique`/`AddCheck { not_valid: false }`/
  `AddForeignKey { not_valid: false }` (they scan and can fail on
  existing rows). Lock weight is *not* the criterion — nearly every
  `ALTER TABLE` takes ACCESS EXCLUSIVE; that is handled by
  `lock_timeout` in §6, not by the flag.

### 6. Preview and apply commands, gate, typed error union

- `preview_object_ddl(connection_id, ops) -> Result<DdlPlanPreview, PgObjectError>` —
  pure generation behind a command (connection id only for engine
  validation); **ungated** (it executes nothing). Preview reads the stored
  connection record directly and never hydrates credentials or resolves an
  SSH tunnel.
- `apply_object_ddl(payload { connection_id, ops, confirmed }) -> Result<DdlApplyResult, PgObjectError>`:
  1. `find_connection`, engine check, resolve safety policy.
  2. **Regenerate server-side from `ops`** — apply never accepts a
     statement over IPC (fragments are re-verified by the §5
     `classify_script` pass); generation is deterministic and
     test-pinned, so what was previewed is what runs.
  3. Gate: `assert_permitted(policy, WriteIntent::Ddl, confirmed)`
     folded to `PolicyBlocked { reason }` /
     `PolicyNeedsConfirmation { statements }`. **The fold must
     synthesize the summaries itself**: `statement_summaries(&Ddl)`
     returns an empty vec (`safety/policy.rs:141-149`) and
     `SafetyRefusal::fold`'s closure receives that empty vec — ignore
     it and build `DdlStatementSummary` from the regenerated preview.
     No new `WriteIntent` variant: `Ddl` already confirms on
     protected/strict and is never read-only permitted.
  4. Execute on a **dedicated connection**: acquire from the native PG
     pool, then `PoolConnection::detach()` so it never returns to the
     pool (a pooled socket left in aborted-transaction state after a
     failed rollback would poison the next catalog read — Result
     Mutation's `rollback_and_verify` exists for this reason,
     `result_mutation/postgres.rs:1007-1010`). On the detached
     connection run `SET statement_timeout = 0` (the pool's
     `after_connect` timeout, `postgres/pool.rs:79-87`, would cancel a
     table rewrite or a `CONCURRENTLY` build mid-flight) and
     `SET lock_timeout = '10s'` (the Result Mutation precedent,
     `postgres.rs:18`, so an `ALTER TABLE` queued behind an open
     query-session transaction fails typed instead of hanging). Then
     group by group: `Atomic` → `sqlx::Transaction` (rollback-on-drop,
     so a rollback failure cannot leak an open transaction) with
     explicit rollback on error; `Standalone` → single execute with no
     open transaction. Close the connection at the end regardless of
     outcome. DDL is rare; one socket per apply is the right trade.
  5. **Residue check.** If a `Standalone` `CreateIndex { concurrently:
     true }` fails, query `pg_index.indisvalid` for the (always
     explicit, §5) index name; an invalid leftover is reported in
     `Database.residue` or `LockTimeout.residue` so Plan 014 can say "an invalid index
     `<name>` was left behind; drop it before retrying".
  6. Audit `RequiredAfterSuccess` under `"apply_object_ddl"` (class
     labels only, per ADR-0024).

```rust
#[serde(rename_all = "camelCase")]
pub(crate) struct DdlStatementSummary {
    pub index: usize,
    pub summary: String,       // PlannedStatement.summary
    pub destructive: bool,
    pub transactional: bool,
}
#[serde(rename_all = "camelCase")]
pub(crate) struct DdlApplyResult {
    pub applied_statements: usize,   // == preview.statements.len() on success
    pub runtime_ms: u64,
}
#[serde(rename_all = "camelCase", rename_all_fields = "camelCase", tag = "kind")]
pub(crate) enum DdlResidue { InvalidIndex { schema: String, name: String } }

#[serde(rename_all = "camelCase", rename_all_fields = "camelCase", tag = "kind")]
pub(crate) enum PgObjectError {
    UnsupportedEngine { engine: String },
    ObjectNotFound { reference: PgObjectRef },
    InvalidOp { op_index: usize, reason: String },
    PolicyBlocked { reason: String },
    PolicyNeedsConfirmation { statements: Vec<DdlStatementSummary> },
    Connection { message: String },     // pool connect / friendly_sqlx_error text
    LockTimeout { statement_index: usize, applied_statements: usize,
                  #[serde(skip_serializing_if = "Option::is_none")] residue: Option<DdlResidue> },
    Database { #[serde(skip_serializing_if = "Option::is_none")] statement_index: Option<usize>,
               code: Option<String>, message: String, position: Option<u32>,
               applied_statements: usize,
               #[serde(skip_serializing_if = "Option::is_none")] residue: Option<DdlResidue> },
}
```

- `DdlStatementSummary` is **not** `StatementClassSummary`
  (`sql_class.rs:86-91` — `index/class/unbounded/destructive`, rendered
  by `safety-confirm-dialog.tsx:79-93` as a class label only). The DDL
  confirmation must show the human-readable statement summary, so Plan
  014 adds a `{ kind: "ddl"; statements: DdlStatementSummary[] }`
  subject to `src/lib/safety-confirmation.ts` and the dialog. This
  plan's job is to emit the shape above and pin it in wire tests.
- `Database` covers apply-time *and* catalog/describe read failures
  (`statement_index: None` for reads — the `op_index: Option` pattern
  of `result_mutation/protocol.rs:384-385`). Read-path positions are `None`
  because the query text is not retained for conversion from PostgreSQL's
  one-based character index to the wire contract's byte offset. There is no separate
  `Storage` arm: in this codebase "storage" means the SQLite app store.
- `Database.applied_statements` counts statements from *earlier
  completed groups* — a failed `Atomic` group contributes zero (it
  rolled back), a failure after a `Standalone` succeeded reports it.
  This is the honest partial-progress disclosure the register's
  "clear non-transactional exceptions" demands. `position` is a byte
  offset into the *failing statement's* SQL (statements execute one at
  a time, so it is unambiguous).
- `code`/`position` come from sqlx's `as_database_error()`
  (`PgDatabaseError` exposes SQLSTATE and position), matching the
  actor `database` arms' fidelity. SQLSTATE `55P03` maps to
  `LockTimeout` before the generic `Database` arm without discarding any
  concurrent-index residue.
- No cancel command in this plan: with `statement_timeout = 0` and
  `lock_timeout = 10s`, the only unbounded phase is the DDL's own work
  (a table rewrite, a concurrent index build), which cannot be safely
  interrupted from a second connection without `pg_cancel_backend`
  privileges. Record as a follow-on in ADR-0026.
- Wire-shape serde tests in the module, copying
  `query_session/protocol.rs:254-262`.
- Safety coverage: `apply_object_ddl` is a *typed* surface — it does
  **not** join `LegacyCommand::ALL`, the `NON_DESTRUCTIVE_LEGACY_COMMANDS`
  allowlist, or `commands/safety.rs`'s `write_intents()` inventory
  (that inventory feeds `assert_legacy_permitted`'s string tags,
  `commands/safety.rs:14-26`). Its gate test lives beside the typed
  precedent `apply_command_core_refusal_and_failure_never_audit`
  (`commands/result_mutation.rs:274-351`, using `test_app_state`):
  read-only → `PolicyBlocked`; strict unconfirmed →
  `PolicyNeedsConfirmation` with summaries equal to the preview's;
  confirmed → executes and audits; refusal never audits. The fifteen
  write-capable surfaces become **sixteen**; update the
  `plans/README.md:94` phrasing in the Plan 014 truth pass, not here.
- `execute_ddl` is untouched here. Plan 015 switches the PostgreSQL
  structure-commit path to `apply_object_ddl`; `execute_ddl` remains
  the ClickHouse structure path.

### 7. TS mirrors (type-only)

`src/lib/store/types.ts` gains `PgObjectKind`, `PgTypeClass`,
`PgObjectRef`, `PgObjectCatalog`/`PgSchemaObjects`/`PgCatalogEntry`,
`PgObjectDescription`/`PgObjectFacts`, `PgDropImpact`, `PgObjectOp`,
`PgDefaultValue`, `DdlPlanPreview`/`PlannedStatement`/`StatementGroup`,
`DdlStatementSummary`, `DdlApplyResult`, `DdlResidue`, and
`PgObjectError` — exported through the `src/lib/store/index.ts` barrel.
**No decoder, formatter, store slice, or component references them
until Plan 014**; these are new unions, so no existing exhaustive
switch needs an arm. The flip side: nothing in this plan's gates
catches a Rust arm that has no TS mirror. Plan 014's
`formatObjectDdlError` (exhaustive, no default arm) is the backstop —
any arm added during this plan's review must be mirrored here or it
surfaces as a typecheck failure in Plan 014 Step 4.

### 8. Live fixture: one of every in-scope kind

`infrastructure/test-db/postgres/004_lifecycle_objects.sql` (shared init
dir — both fixtures pick it up; the TLS image's sourced-init mechanism
from Plan 011 Step 6 applies it too):

- Schema `lifecycle` with comment; table `orders` (comment, serial PK);
  view `orders_view` over `orders` (comment); materialized view
  `orders_mat` WITH DATA; sequence `order_seq` (non-default start /
  increment, `OWNED BY` a column); overloaded functions
  `add_nums(integer, integer)` and `add_nums(text, text)`; procedure
  `bump_orders()`; aggregate `sum_squares(numeric)`; enum
  `order_status` (three labels, comment); composite type `money_pair`;
  domain `positive_int` (CHECK); `CREATE EXTENSION IF NOT EXISTS
  hstore` (ships in the official image's contrib; `citext`, `pgcrypto`,
  and `btree_gist` are *already* installed by `001_schema_and_seed.sql:3-5`,
  so use one that is not, and the describe test asserts the version of
  `hstore`).
- The overload pair is the overload-safety test bed; `orders →
  orders_view → orders_mat` (make `orders_mat` select **from
  `orders_view`**, not from `orders`) is the transitive drop-impact
  test bed: impact for `orders` must report both at depths 1 and 2.
- Extension-member exclusion test bed: `public.functions` must not
  contain any `pgcrypto`/`citext`/`btree_gist` routine, and
  `public.types` must not contain `citext`.

### 9. ADR-0026

`docs/adr/0026-backend-owned-object-ddl-workflow.md` records: typed
operations cross IPC, never whole statements — SQL fragments do cross
and are re-verified by `classify_script` as exactly one statement — and
apply regenerates server-side; preview-first with per-statement
summaries; the transactional grouping rule and the `CONCURRENTLY`
exception with its invalid-index residue disclosure; the dedicated
detached connection with `statement_timeout = 0` / `lock_timeout = 10s`
and the no-cancel follow-on; transitive drop impact with
RESTRICT-by-default, PostgreSQL as the enforcement boundary;
overload-safe routine identity via
`pg_get_function_identity_arguments`; the kind vocabulary and why event
triggers/roles/tablespaces are lists, not kinds; `WriteIntent::Ddl`
reuse with `DdlStatementSummary` (distinct from
`StatementClassSummary`) carrying the human-readable summary. One-line
pointers from ADR-0014 (specialized editors: index/FK panels migrate to
typed ops in Plan 015) and ADR-0023 (the preview/apply shape now has a
DDL sibling).

## Commands you will need

```sh
just fmt && just lint && just test
pnpm db:postgres        # plain fixture, 15432 (rebuild after editing init SQL)
pnpm db:postgres-tls    # TLS fixture, 15433
cargo test --manifest-path src-tauri/Cargo.toml -- --ignored object    # live suites, Steps 3–6
pnpm format && pnpm lint && pnpm typecheck && pnpm vitest run
grep -rn "ObjectKind" src-tauri/src        # 0 before Step 2; grows after
grep -rn "apply_object_ddl" src src-tauri  # command + registration + types only
git diff --stat -- src                     # types.ts + store barrel only, at every step
```

## Scope

Expected files touched (creation marked ＋):

- ＋ `src-tauri/src/postgres/objects.rs` — kinds, refs, catalog +
  describe + drop-impact queries and renderers (+ inline tests).
- ＋ `src-tauri/src/postgres/object_ddl.rs` — op union, generation,
  `classify_script` verification, grouping, classification (+ inline
  tests).
- ＋ `src-tauri/src/commands/pg_objects.rs` — the five commands, gate,
  detached-connection executor, residue check, audit, wire-shape and
  gate tests; `src-tauri/src/lib.rs` registration + `quote_literal`
  beside `quote_double`.
- `src-tauri/src/types.rs` — shared payload/response types (or re-export
  from `postgres/objects.rs`, following the crate's existing split).
- ＋ `infrastructure/test-db/postgres/004_lifecycle_objects.sql`.
- `src/lib/store/types.ts`, `src/lib/store/index.ts` — mirrors +
  barrel exports (type-only).
- ＋ `docs/adr/0026-…md`; pointers in ADR-0014 / ADR-0023.
- `plans/README.md` — status row.

Out of scope (STOP if you find yourself editing them): any rendering
component, `database-navigator.tsx`, `open-anything.ts`,
`workspace-tabs.ts`, `relational-tables.ts`, `src/lib/ddl/*`,
`execute_ddl` / `export_ddl` behaviour, `load_schema_explorer`,
`storage.rs` (no migration), MySQL/SQLite/ClickHouse arms beyond typed
`UnsupportedEngine`, CONTEXT/register truth pass (Plan 014).

## Resume protocol

Each step ends with all gates green. Live (`--ignored`) tests are
required for Steps 3–6 — a session without Docker records
`BLOCKED: live fixture unavailable` on the README row rather than
skipping them. Never mark a later step done while an earlier step's
gate is red.

## Git workflow

Work on the current branch in the working tree. **No commits, pushes, or
PRs without explicit operator authorization.** The completion SHA is
recorded by the operator after review.

## Steps

### Step 1: Record contract decisions

Confirm against the working tree: no `ObjectKind`/`pg_depend` hits; the
`SchemaExplorer` and `execute_ddl` excerpts above still match;
`quote_double` is at `lib.rs:80` and `classify_script` at
`sql_class.rs:126`; `apply_command_core_refusal_and_failure_never_audit`
is at `commands/result_mutation.rs:274-351`; `LegacyCommand::ALL` is as
described; init file `004` is free; migration 18 is still the latest
(no migration needed here — a mismatch means the landscape moved).
Record deviations under the `## Review correction record` heading
before proceeding.

### Step 2: Object model + pure DDL generation

Land `objects.rs` (kinds, refs), `quote_literal` in `lib.rs`, and
`object_ddl.rs` per §1/§5. Unit tests: a rendered-SQL snapshot per op
variant (including quoting of `weird"name` identifiers and `'quoted'`
literals/labels/comments); tagged default literal-vs-expression; `USING`
clause; overload-safe `DROP FUNCTION add_nums(integer, integer)`;
RESTRICT rendered when `cascade: false`; derived index name when
`name: None`; grouping (transactional runs coalesce, `CONCURRENTLY`
index ops isolate as `Standalone`); destructive classification table
(including the `not_valid: false` constraint adds); every `InvalidOp`
branch **including** a `sql_body` / `expression` / `using` / default
`Expression` containing a statement boundary (`; DROP TABLE x`) and a
fragment whose class is destructive flipping the flag. Nothing calls
the modules yet. Gate: `just` trio.

### Step 3: Fixture + catalog command

Land `004_lifecycle_objects.sql` per §8 and `load_pg_object_catalog`
per §2 on the native pool, with the command wrapper and typed errors.
Rebuild and start the plain fixture. Live ignored tests: the catalog
lists every fixture object under `lifecycle` with comments where set;
the overloaded functions appear as two entries with distinct
`identity_args`; `types` entries carry the right `PgTypeClass`;
event triggers/roles/tablespaces arrive database-scoped; no
`pg_temp*` schema appears; `public.functions`/`public.types` contain
no extension members (§8); the cap is exercised on a synthetic schema
with `CATALOG_KIND_CAP + 1` sequences (create in the test, drop after)
and `truncated` names it; a non-PostgreSQL connection returns
`UnsupportedEngine`. Gates: `just` trio + `cargo test … -- --ignored
object`.

### Step 4: Describe + drop impact

Per §3/§4. Live tests: view/matview definitions round-trip
(`definition_sql` starts `CREATE VIEW`/`CREATE MATERIALIZED VIEW`);
`pg_get_functiondef` for both overloads; aggregate describe returns
facts with `definition_sql: None`; sequence facts include `owned_by`
and `last_value`; enum labels in `enumsortorder` order; domain checks;
extension version; describing a dropped object →
`ObjectNotFound`. Drop impact: `orders` reports `orders_view` at depth
1 (resolved through `pg_rewrite`, not the `_RETURN` rule) **and**
`orders_mat` at depth 2; the owned sequence relationship handling is
deliberate (an `OWNED BY` sequence is an `'a'` dep — assert it appears
or is excluded, and pin the choice with a comment); the cap +
`truncated` flag on a synthetic many-dependent case; the depth limit
on a synthetic 10-deep view chain. Gates as Step 3.

### Step 5: Preview, apply, gate

Per §6. Unit tests: apply regenerates from ops (the payload type has no
statement-level SQL field; fragments are covered by Step 2's boundary
tests); gate fold (read-only → `PolicyBlocked`; strict unconfirmed →
`PolicyNeedsConfirmation` with summaries equal to the preview's,
non-empty even though `statement_summaries(&Ddl)` is empty; confirmed
passes; refusal never audits) in the `commands/result_mutation.rs`
style with `test_app_state`. Live tests: create schema → rename →
comment → drop round-trip; add check + unique + FK on a scratch table
then `DropConstraint`; `AlterColumnType` with `USING`; enum
`AddEnumValue`/`RenameEnumValue`; `CreateIndex { concurrently: true }`
succeeds (proving it runs outside a transaction — it would error inside
one); an `Atomic` group with a failing second statement leaves the
first's effect rolled back and reports `statement_index: 1,
applied_statements: 0` with the SQLSTATE; a `Standalone`-then-failing
plan reports `applied_statements: 1`; **pool health**: after every
failing case above, a `load_pg_object_catalog` on the same connection
id succeeds (the detached socket never returned to the pool);
**residue**: a `CONCURRENTLY` index whose expression fails on existing
rows reports `residue: InvalidIndex { … }` and the test drops it;
**lock timeout**: hold an open transaction with a row lock on the
scratch table from a second connection, run `AddColumn`, and assert
`LockTimeout { statement_index: 0 }` within ~10s rather than a hang;
audit row recorded after a confirmed strict apply. Gates: `just` trio
+ live run.

### Step 6: TS mirrors, ADR, re-run

Per §7/§9. `git diff --stat -- src` must show only `types.ts` and the
store barrel. Full re-run of every suite in this plan plus
`pnpm format && pnpm lint && pnpm typecheck && pnpm vitest run` (the
frontend suite must stay green with type-only changes). Mark
`READY FOR REVIEW`.

## Test plan

Step 2 is pure and needs no server. Steps 3–5 need `pnpm db:postgres`
(15432) with the rebuilt fixture; run the suite once against 15433 as
well to confirm the TLS fixture sources the new init file. Step 6 is
type-only plus reruns.

## Done criteria

- A typed `PgObjectKind`/`PgObjectRef` model exists with overload-safe
  routine identity; nothing bakes identity into display strings.
- The catalog returns all eleven schema-scoped kinds plus the three
  list-only groups, batched on the native pool with comments, filtered
  of extension members, capped and disclosed; the sqlx-Any explorer is
  untouched and still serves the legacy path.
- Describe returns owner, comment, facts, and reconstructed DDL for
  every kind, with typed `ObjectNotFound` for races.
- Drop impact reports the transitive `pg_depend` closure with
  `pg_rewrite` resolution and depth, capped and disclosed.
- Preview/apply round-trips typed ops: every generated statement
  verified as exactly one statement, per-statement summaries,
  destructive and transactional flags, atomic groups that roll back on
  a detached socket that never poisons the pool, `CONCURRENTLY` proven
  to run standalone with residue disclosure, `lock_timeout` proven to
  fail typed, SQLSTATE + statement index + applied-count on failure,
  gate + audit wired, sixteen gated surfaces covered by tests.
- Zero rendering diffs; all gates plus the `--ignored object` suite
  green.

## STOP conditions

- Plan 012 not recorded `DONE` in `plans/README.md`.
- Any step requires editing a rendering component, a store slice, or
  `src/lib/ddl/*` to keep gates green.
- The apply payload turns out to need a *whole statement* across IPC
  for any op — that is the design this plan exists to avoid; extend
  the op vocabulary or defer the op instead. (Fragments are expected;
  see §5.)
- `PoolConnection::detach` is unavailable on the pinned sqlx version —
  fall back to a fresh `PgConnection::connect_with` using the pool's
  connect options, never to a pooled socket.
- `pg_identify_object` or `pg_sequences` is unavailable on the fixture's
  PostgreSQL major version (both exist on every supported version; a
  miss means the fixture image changed).
- A live test needs `cascade: true` as a default anywhere.

## Maintenance notes

- Adding a `PgObjectOp` variant is a four-place change: the enum, the
  renderer match, the destructive/transactional classifiers, and the
  snapshot tests — the exhaustive matches fail until all are done.
- Adding a `PgObjectKind` (e.g. promoting event triggers from
  list-only) means deciding three things explicitly: catalog query,
  describe arm, and drop renderer (or `InvalidOp`).
- `quote_double`/`quote_literal` in `lib.rs` are the only quoting
  sites; a renderer using `format!` interpolation for an identifier is
  a bug.
- The generation function must stay pure and deterministic — apply's
  "regenerate server-side" contract depends on it. The
  `classify_script` pass is part of generation, not of apply.
- Any op that adds a new SQL-fragment field must be covered by a
  statement-boundary test in the Step 2 `InvalidOp` table.

## Review correction record

Pre-execution review, 2026-08-29 (no code landed; the sections above
are already corrected). Recorded so an executor comparing this file to
its README description or to Plan 014's earlier contract excerpt
understands why they differ:

1. "Apply never accepts SQL over IPC" was false by the plan's own
   vocabulary (`sql_body`, `expression`, `using`, `where_predicate`,
   `Expression.sql`). Restated as "never a whole statement", and
   `classify_script` now verifies each generated statement (§5).
2. `PolicyNeedsConfirmation.statements` claimed the query-session
   `StatementClassSummary` shape; it is a new `DdlStatementSummary`
   with the human-readable summary, and the fold must synthesize it
   because `statement_summaries(&Ddl)` is empty (§6). Plan 014 adds
   the matching confirmation subject.
3. Apply ran on a pooled socket with the pool's `statement_timeout`,
   no `lock_timeout`, and warn-only rollback failure. Now a detached
   connection with `statement_timeout = 0`, `lock_timeout = 10s`,
   `sqlx::Transaction`, a `LockTimeout` arm, and a `CONCURRENTLY`
   residue check (§6).
4. Drop impact was direct-only while `CASCADE` is transitive; now a
   bounded recursive closure with `depth` (§4).
5. Catalog schema filter did not match the explorer it claimed to
   match, extension members were not excluded, and the catalog was
   unbounded (§2).
6. `DdlApplyResult` was undefined; `Storage` duplicated `Database`;
   `quote_ident` duplicated `quote_double`; `EventTrigger`/`Role`/
   `Tablespace` were kinds that only ever returned errors — all fixed
   (§1, §6).
7. The gate-coverage test was pointed at the legacy string-tag
   inventory; now at the typed precedent in `commands/result_mutation.rs`
   (§6). `plans/README.md:55` → `:94`.
8. The fixture's `citext` extension was already installed by `001`;
   now `hstore`, and `orders_mat` selects from `orders_view` for the
   depth-2 test (§8).
9. The structure-editor switchover is Plan 015, not 014.
10. Fresh-start drift check at `b45e294`: the legacy explorer's actual
    schema predicate is `nspname NOT IN ('information_schema') AND
    nspname NOT LIKE 'pg_%'`, while §2 described the escaped
    `n.nspname <> 'information_schema' AND n.nspname NOT LIKE 'pg\_%'`
    form as an exact match. The new catalog uses the escaped predicate
    promised by §2 so temporary and toast schemas remain excluded;
    the legacy explorer stays untouched per scope.
11. Final implementation review closed typed-fragment and catalog fidelity
    gaps: data types now reject column/sequence option smuggling and expression
    defaults are parenthesized; preview does not hydrate secrets or tunnels;
    descriptions use one repeatable-read/read-only snapshot; range and sequence
    DDL preserve separately identified catalog options; read errors do not leak
    raw character positions; `LockTimeout` retains concurrent-index residue.
    The dependency closure changed from an execution-unbounded recursive CTE to
    an eight-level BFS capped at 201 normalized, unvisited addresses per level
    (1,609 including the root), with exact column/subobject joins and saturation
    carried independently from final identity deduplication.
12. Post-fix review tightened replay and wire fidelity: top-level `USING` is
    forbidden inside type fragments; `pg_get_viewdef` terminators are removed
    before reconstruction; sequence quantities are validated decimal strings
    end to end; catalog kind queries bind only the retained schema cap;
    `quote_literal` emits escaped PostgreSQL `E` strings; foreign-table DDL
    retains defaults, explicit collations, and CHECK constraints; serial and
    identity sequence ownership are both recognized; and the complete drop
    impact walk runs in one repeatable-read, read-only snapshot.
13. Release review removed a global materialized-view whitespace replacement
    that could mutate `WITH  DATA` inside the query body, kept the primary-key
    and unique-operation TypeScript discriminants exact, and synchronized the
    PostgreSQL module map. Live validation also pinned drop-impact identity
    ordering to the `C` collation so it is stable across database locales.
