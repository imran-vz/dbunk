# Plan 033: Schema editor and schema change diff (PostgreSQL, MySQL, SQLite)

**Status:** Phases 0 to 2 implemented on `t3/schema-editor-diff`, and phase 3 in part (rebase on drift, OutcomeUnknown recovery for PostgreSQL drafts, large-table notes). The native app has not been built yet; `just check-native` and the `just dev-native` walkthrough run after merge to main. Not done: cancellation during apply, MariaDB live tests, phase 4.
**Decisions taken:** `sqlparser-rs` is approved as a backend dependency. MySQL
8.0+ and MariaDB 10.6+ are editable; older servers stay read-only. Build the
shared base first, then implement all three engines in parallel.

## Goal

Users edit a table's structure in place: columns, keys, foreign keys, checks,
indexes, table name and comment. They see a structural diff of what will
change, its impact and the exact SQL, then apply it through the existing
safety gate. The same editor and diff view serve PostgreSQL, MySQL and SQLite.

## Where we start

| Engine | Today | Gap |
|---|---|---|
| PostgreSQL | Read-only Structure tab. Typed `PgObjectOp` set (about 35 variants: add/drop/alter column, constraints, indexes, create table) in `backend/src/postgres/object_ddl/`. Native exposes only comment/rename (`backend/src/backend/table_ddl.rs`) and five object operations (`backend/src/backend/object_ddl/types.rs:63`). | Column, constraint and index ops have no native claims or `verify` arms (`postgres/native_object_ddl/catalog.rs`). No batching per table. No diff UI. |
| MySQL | Read-only structure (`mysql_lane/document.rs:845`), fetched through the legacy sqlx-Any path (`mysql_sessions/worker.rs:224`). | No DDL (`dispatch/relational.rs:912`). Introspection misses charset, collation, comments, generated expression, `ON UPDATE`, INVISIBLE, index prefix and order, table options. `MODIFY COLUMN` restates a whole column, so these gaps would silently drop attributes. |
| SQLite | Read-only structure from the session (`sqlite_session.rs:1089`). | No DDL. CHECK, COLLATE, generated expressions, AUTOINCREMENT, STRICT and WITHOUT ROWID only exist in `sqlite_master.sql`, which nothing parses. No table rebuild. |

Reusable pieces:
- The observe, review, apply and confirm lifecycle with re-render-and-compare at
  apply (`backend/src/backend/table_ddl.rs:122-311`).
- Journal-before-dispatch (`apps/native/src/apply_flow.rs`,
  `backend/src/backend/development/workspace_*_ddl.rs`).
- The engine-neutral safety policy (`backend/src/safety/policy.rs`,
  `WriteIntent::Ddl`) and the per-dialect statement classifier
  (`backend/src/postgres/sql_class.rs`).
- The row-change review dialog shell, `TypedConfirm` and `sql_list`
  (`apps/native/src/table_changes/review.rs`).
- The staged-edit tints `edited_fill`, `deleted_fill` and `ok_fill`
  (`apps/native/src/style.rs`, `DESIGN.md:59-67`).
- The naming from schema compare (`DifferenceKind`, `FieldPath`); its types
  are PG16-catalog shaped and are not reused directly.

## Architecture

```
Baseline (captured, per engine) ──┐
                                  ├─ diff() ─▶ SchemaChangeSet ─▶ engine planner ─▶ SchemaEditPreview ─▶ apply
SchemaDraft (user edits)        ──┘                                                  (statements, groups, notes)
```

1. **Baseline.** The complete structure of one table, normalized into the
   engine-neutral model, plus engine facts and a drift token:
   - PostgreSQL: relation OID, plus the xmin of `pg_class` and of each
     sub-object.
   - SQLite: `PRAGMA schema_version`.
   - MySQL: a hash of the full introspection, which is re-read at apply.
2. **Schema Draft.** An editable copy of the baseline. Every column, index,
   key and constraint carries `origin: Existing(BaselineId) | New`. A rename
   changes the name and keeps the origin, so it is never planned as drop plus
   add.
3. **Schema Change Set.** `diff(&Baseline, &SchemaDraft)` is a pure function
   that returns object-level and field-level changes. Native calls it on every
   edit for the live view. The backend runs it again at review and at apply.
4. **Planner (one per engine).** Turns a change set into ordered typed
   operations and renders a `SchemaEditPreview`: statements with summaries,
   destructive flags, atomic or standalone groups, and engine notes such as
   table rewrite, rebuild, non-atomic apply or required lock. The UI never
   sends SQL (ADR-0011).
5. **Apply.**
   1. Re-render and compare with the reviewed preview.
   2. `assert_permitted(WriteIntent::Ddl)`.
   3. Journal the attempt durably.
   4. Recheck drift, execute, verify by re-introspecting.
   5. Return an outcome.

   A drifted baseline refuses with `BaselineChanged`. The user can rebase the
   draft onto a fresh baseline: unchanged origins carry over, and conflicts
   are flagged per field.

There is no shared `Engine` trait (ADR-0002). What is shared is the data model,
`diff()`, the preview type and the UI. Each engine owns its planner and
executor, and `backend/src/backend/schema_edit.rs` routes to them with a
`match` on the engine.

### Engine-neutral model (sketch)

```rust
pub struct SchemaBaseline {
    pub engine: SchemaEngine,             // Postgres | MySql | Sqlite
    pub server: ServerFacts,              // version, flavor (MySQL vs MariaDB), feature flags
    pub table: TableModel,
    pub drift: DriftToken,                // opaque, engine-specific
    pub editable: Editability,            // Editable | ReadOnly { reasons }
}
pub struct TableModel {
    pub schema: String, pub name: String, pub comment: Option<String>,
    pub columns: Vec<ColumnModel>,
    pub primary_key: Option<KeyModel>,
    pub uniques: Vec<KeyModel>,
    pub foreign_keys: Vec<ForeignKeyModel>,
    pub checks: Vec<CheckModel>,
    pub indexes: Vec<IndexModel>,
    pub options: TableOptions,            // engine-specific, typed per engine (STRICT, WITHOUT ROWID, ENGINE, charset, ...)
}
pub struct ColumnModel {
    pub origin: Origin,                   // Existing(BaselineId) | New(DraftId)
    pub name: String,
    pub data_type: String,                // validated per dialect, compared as text
    pub nullable: bool,
    pub default: DefaultValue,            // None | Literal(String) | Expression(SqlFragment)
    pub generated: Option<Generated>,     // expression + stored/virtual
    pub identity: Identity,               // PG identity, MySQL AUTO_INCREMENT, SQLite AUTOINCREMENT
    pub collation: Option<String>,
    pub comment: Option<String>,
    pub engine_extra: ColumnExtra,        // MySQL charset/ON UPDATE/INVISIBLE; empty elsewhere
}
pub enum FieldPath { Table(TableField), Column(Origin, ColumnField), Key(Origin, KeyField), ... }
pub enum ChangeKind { Added, Removed, Renamed, Changed { field: FieldPath } }
```

- **Expressions** (defaults, checks, generated columns, index predicates) are
  `SqlFragment`s checked by the dialect lexer. That means one expression, no
  statement terminators, and balanced quotes and parentheses.
- **No-op detection.** An untouched expression compares equal because it is
  the server's own rendering, copied byte for byte from the baseline. Edited
  text is never declared equal to the baseline unless the bytes match.
- **Types** are compared as normalized text: case-folded keywords and
  canonical spacing.

### Operation ordering (all engines)

1. Drop foreign keys, checks, uniques and indexes that are removed or that
   depend on a changed column.
2. Rename columns, then the table.
3. Alter column types, nullability, defaults and generation.
4. Add columns.
5. Drop columns.
6. Add the primary key, uniques, checks and indexes.
7. Add foreign keys last.
8. Comments.

### Destructive classification

Destructive operations: drop column, drop constraint or index, type change
(unless provably widening, such as `varchar(n)` to a larger `n`, `int` to
`bigint`, or `text` from `varchar`), set NOT NULL, add NOT NULL without a
default, a SQLite rebuild, and any MySQL `MODIFY` that changes the type. These
flags drive the review UI. Gating stays `WriteIntent::Ddl` per ADR-0004.

## Engine planners

### PostgreSQL
- Map changes onto the existing `PgObjectOp`s: AddColumn, DropColumn,
  RenameColumn, AlterColumnType (with an optional `USING`), SetColumnNullable,
  SetColumnDefault, the Add* constraints, DropConstraint, CreateIndex,
  DropIndex, RenameObject and SetComment. New ops:
  - RenameConstraint
  - SET/DROP IDENTITY
  - SET/DROP EXPRESSION
  - ALTER COLUMN COLLATE (rides on type change)
- Extend `native_object_ddl` claims and `verify` arms for each op. Grouping
  follows the existing rules: one atomic group, with `CONCURRENTLY` index
  statements standalone (offered as a per-index toggle).
- Notes:
  - Table rewrite for type changes, except binary-coercible ones.
  - ACCESS EXCLUSIVE lock.
  - `NOT VALID` plus a later `VALIDATE` option for FK and CHECK on large
    tables.
  - `lock_timeout` 10s, as in `object_service.rs`.
- Column reorder is not offered.

### SQLite (version from `SqliteSessionInfo.sqlite_version`)
- **Introspection.** Parse `sqlite_master.sql` with `sqlparser-rs`
  (`SQLiteDialect`) and cross-check the result against `pragma_table_xinfo`,
  `foreign_key_list`, `index_list`/`index_xinfo` and `pragma_table_list`
  (STRICT, WITHOUT ROWID). If the parse fails or the two disagree, the
  baseline is `ReadOnly` with a reason; there is no guessing.
- **Fast path:**
  - ADD COLUMN, within its documented restrictions: no PK or UNIQUE, a
    constant default, NOT NULL only with a default, no STORED generated
    column.
  - RENAME COLUMN (3.25+) and table rename.
  - DROP COLUMN (3.35+), when the column is not in a key, an index, an FK, a
    generated expression, a view or a trigger.
- **Rebuild** for everything else, following the documented 12-step
  procedure:
  1. Check that the session is in autocommit. If the user has an open
     transaction on the shared connection, refuse with "commit or roll back
     first".
  2. Read `PRAGMA foreign_keys`. If it is ON, turn it OFF before BEGIN; it is
     restored after.
  3. BEGIN.
  4. Create the new table from the rendered draft.
  5. `INSERT INTO new SELECT … FROM old`, with CAST for changed types.
  6. Drop the old table and rename the new one.
  7. Recreate indexes and triggers from their baseline SQL.
  8. Verify inside the transaction, before COMMIT:
     - `PRAGMA foreign_key_check`
     - `PRAGMA integrity_check` (bounded to this table where possible)
     - re-introspect and compare with the draft

     Any failure means ROLLBACK.
  9. COMMIT, then restore `foreign_keys`.
- SQLite DDL is transactional, so a plan is always one atomic group.
- Execution happens on the session worker as a new `Work::SchemaEdit*` variant,
  because a second connection would contend for the write lock. The existing
  progress handler gives it cancellation.

### MySQL / MariaDB
- **Introspection** moves onto the session connection through new `Request`
  variants. It reads:
  - `information_schema` COLUMNS (`COLUMN_TYPE`, `CHARACTER_SET_NAME`,
    `COLLATION_NAME`, `COLUMN_COMMENT`, `EXTRA`, `GENERATION_EXPRESSION`,
    `COLUMN_DEFAULT` with literal or expression detection)
  - STATISTICS (`SUB_PART`, `COLLATION` for order, `EXPRESSION`)
  - TABLE_CONSTRAINTS, REFERENTIAL_CONSTRAINTS, CHECK_CONSTRAINTS
  - TABLES (`ENGINE`, `TABLE_COLLATION`, `ROW_FORMAT`, `TABLE_COMMENT`)

  The result is cross-checked against `SHOW CREATE TABLE`, parsed with
  `sqlparser-rs` (`MySqlDialect`). Any disagreement makes the baseline
  `ReadOnly`.
- **Version gate.** The version string tells MySQL from MariaDB. Below 8.0
  (MySQL) or 10.6 (MariaDB) the baseline is `ReadOnly`. Feature flags: RENAME
  COLUMN, INSTANT ADD COLUMN (8.0.12+, any position 8.0.29+), enforced CHECK.
- **Planner.**
  - All column, key and index changes for one table go into a single
    `ALTER TABLE … ADD/MODIFY/CHANGE/DROP/RENAME …` statement. With InnoDB
    that statement is atomic.
  - FK adds and drops that the server rejects inside the same statement are
    split into separate statements.
  - Column reorder via `FIRST`/`AFTER` is offered.
  - An optional "online only" toggle appends `ALGORITHM=INSTANT|INPLACE,
    LOCK=NONE`, so the server refuses instead of copying or locking.
- **Apply** runs on a dedicated short connection to the same resolved
  endpoint. DDL commits implicitly and must not commit a transaction the user
  has open in the query tab. The connection sets `lock_wait_timeout` (10s) so
  a metadata lock fails fast.
- **Outcomes.** A multi-statement plan reports the committed prefix
  (`Stopped { committed, failed_statement }`), because MySQL cannot roll back
  earlier DDL. The review says this up front.

## Native UI

- **Structure tab draft mode.** An "Edit structure" control, shown only when
  the baseline is `Editable` and the connection is not read-only.
  - Rows are tinted `ok_fill` for added, `edited_fill` for changed with old →
    new shown inline, and `deleted_fill` with strikethrough for removed.
  - Discard reverts the draft; Undo works per field.
  - The "Review N ⌘S" accent button follows Plan 032.
- **Review dialog,** built on the `table_changes/review.rs` shell:
  1. **Changes:** grouped by Columns, Keys, Foreign keys, Checks, Indexes and
     Table, with field-level old → new.
  2. **Impact:** destructive items, rewrite or lock notes, the SQLite rebuild
     notice, MySQL non-atomic apply, and inbound FKs and dependents.
  3. **SQL:** exact statements per group, in a read-only SQL editor
     (`Editor::for_buffer`, `set_read_only`, `sql::language`).
  4. **Definition diff (Phase 4):** before/after `CREATE TABLE` as a line
     diff. Display only; the structural diff remains the source of truth
     (ADR-0013).
- **Gate.** Environment chip, `env_notice` and `TypedConfirm` for Production,
  using the existing `ApplyFlow` (Saving → Dispatched or Confirmation).
- **Draft persistence.** The draft is journaled with the workspace tab
  (`WorkspaceSchemaEdit`, alongside `workspace_table_ddl.rs`) so a restart
  restores it. The apply attempt is journaled before dispatch, and an
  OutcomeUnknown attempt surfaces after restart.
- **Engine entry points:**
  - PostgreSQL: `table_structure_view.rs`
  - MySQL: `mysql_lane/document.rs`
  - SQLite: `sqlite_documents.rs`

  All three host the same `schema_edit_view` component.

## Delivery: base first, then parallel lanes

Per `CLAUDE.md`, subagents and worktrees never build or run tests. They write
code and tests and commit. Every build and check runs in the main checkout
after merge.

### Phase 0: Base, single agent in the main checkout
Owns every shared file, so the later lanes never touch them.
- `backend/Cargo.toml`: add `sqlparser`.
- `backend/src/schema_edit/{mod,model,diff,change,preview,fragment}.rs`: the
  model, `diff()`, ordering, destructive classification and the preview type.
- `backend/src/schema_edit/{postgres,sqlite,mysql}/mod.rs`: stubs with fixed
  signatures (`capture`, `plan`, `execute`) returning `Unsupported`.
- `backend/src/backend/schema_edit.rs`: the public facade (`observe`,
  `review`, `apply`, `confirm`), outcomes, errors, journal record type and
  engine routing. Registered in `backend.rs`.
- `Work`/`Request` enum variants added to `sqlite_session.rs` and
  `mysql_sessions.rs`, delegating to the stub modules.
- ADR-0016 "Schema drafts and per-engine change planners", and glossary
  entries (Schema Baseline, Schema Draft, Schema Change Set, Schema Edit
  Preview, Drift Token).
- Unit tests for `diff()`: renames by origin, reorder, no-op on untouched
  expressions, ordering, destructive flags.
- **Exit:** `just fmt`, `just lint` and `just test` pass, `just
  check-native` still passes, and the result is committed. The lanes branch
  from this commit.

### Phase 1: Four parallel lanes in worktrees, with exclusive file ownership

| Lane | Owns | Delivers |
|---|---|---|
| A: PostgreSQL | `backend/src/schema_edit/postgres/**`, `backend/src/postgres/object_ddl/**`, `backend/src/postgres/native_object_ddl/**` | Capture from `table_structure` into the model, planner onto `PgObjectOp`, new ops, claims and verify arms, executor, golden SQL tests, live tests. |
| B: SQLite | `backend/src/schema_edit/sqlite/**`, the bodies of the SQLite `Work::SchemaEdit*` handlers | Parser plus PRAGMA cross-check capture, fast path, 12-step rebuild, in-transaction verification, golden SQL and in-memory-database tests. |
| C: MySQL | `backend/src/schema_edit/mysql/**`, the bodies of the MySQL `Request::SchemaEdit*` handlers | Full introspection, `SHOW CREATE` cross-check, version gate, combined-ALTER planner, online toggle, dedicated apply connection, partial-apply outcomes, golden SQL and live tests. |
| D: Native UI | `apps/native/src/schema_edit_model/**`, `apps/native/src/schema_edit_view/**`, `apps/native/src/schema_edit_review/**`, plus the three entry-point hooks | Draft editing, live diff tints, review dialog, apply flow and persistence, built against the Phase 0 facade. Model tests use fixture baselines. |

Each lane commits on its own branch. Lanes A to C may add private helpers only
inside their own directories. A change a lane needs to a Phase 0 type is
recorded in its hand-back, not made directly.

### Phase 2: Merge and verify, main checkout
- Merge lanes in the order A, B, C, D. Resolve the Phase 0 type requests in
  one commit.
- Run `just fmt`/`lint`/`test` and `just fmt-native`/`lint-native`/
  `test-native`.
- Run the live round-trip suite against the native fixture servers (Postgres
  16, MySQL 8, MariaDB 10.6) and against SQLite files.
- Drive the real app with `just dev-native` through edit, review and apply on
  each engine.

### Phase 3: Hardening
- Rebase on drift, and OutcomeUnknown recovery after restart.
- Large-table notes, cancellation during apply, and keyboard and accessibility
  passes.

### Phase 4: Extensions
- Create and drop table in the same editor (replaces the retired Table
  Designer).
- Definition line diff. This needs a diff crate such as `similar`, which
  requires approval first.
- "Copy SQL" and "Save as migration".
- Feeding schema-compare results into a draft to generate migration SQL.

## Verification

- **Golden SQL** per planner and per operation kind, including ordering and
  version-gated variants.
- **Round-trip invariant (live):** `capture(apply(plan(baseline, draft))) ==
  draft` for every operation kind on every engine. This is what catches
  attributes lost by MySQL `MODIFY` or by a SQLite rebuild.
- **Failure modes:**
  - drift between review and apply → `BaselineChanged`, nothing dispatched
  - lock timeout (PostgreSQL `lock_timeout`, MySQL `lock_wait_timeout`)
  - SQLite rebuild failing mid-copy or on `foreign_key_check` → table
    unchanged
  - SQLite open user transaction → refused
  - SQLite `foreign_keys` ON → restored after apply
  - MySQL failure on statement 2 of 3 → committed prefix reported
  - read-only and Strict policy blocks
  - unsupported server version or unparseable definition → `ReadOnly` with a
    reason
- **Diff unit tests:** rename versus drop-plus-add, swapped column names,
  untouched expression no-ops, and every destructive classification.

## Risks

- **Attribute loss on MySQL `MODIFY` and SQLite rebuild.** Mitigated by full
  introspection, the parser cross-check, `ReadOnly` on any mismatch, and the
  round-trip suite.
- **Shared-connection state.** SQLite DDL must not run inside the user's open
  transaction. MySQL DDL must not commit it implicitly. Mitigated by the
  autocommit check (SQLite) and the dedicated apply connection (MySQL).
- **Scope creep.** Out of scope: views, routines, triggers editing,
  partitions, policies and grants (these already have PostgreSQL object
  flows), and multi-table drafts.

## Docs to update when shipped

- ADR-0016 (new).
- `CONTEXT.md`: new terms, and rewrite the stale "DDL Statement" and "Table
  Designer" entries.
- `ROADMAP.md` §8: the schema editor row, and drop the "dead-end MySQL/SQLite"
  note.
