# Schema drafts and per-engine change planners

**Status:** Accepted (Plan 033, 2026-10-12)

Users edit one table's structure in place on PostgreSQL, MySQL and SQLite.
The editor holds a **Schema Draft**: an editable copy of a captured **Schema
Baseline** in an engine-neutral model (`backend/src/schema_edit/model.rs`).
Every column, key, index and constraint carries an origin, `Existing(id)` or
`New(id)`, and keys, indexes and foreign keys reference columns by origin. A
rename changes the name and keeps the origin, so it is never planned as drop
plus add, and a renamed column stays a member of its keys.

`diff(baseline, draft)` is a pure function that returns the **Schema Change
Set** (object and field changes, with destructive flags). `plan_steps` orders
the engine-neutral operations: drops, renames, column alterations, adds,
column drops, constraint and index adds, foreign keys, then comments. The UI
calls `diff` on every edit; the backend runs `validate`, `diff` and the
engine planner again at review and at apply.

Each engine owns its capture, planner and executor
(`backend/src/schema_edit/{postgres,sqlite,mysql}`). There is no shared
engine trait (ADR-0002); `plan_review` and the facade
(`backend/src/backend/schema_edit.rs`) route with a `match`. A planner turns a
draft into a **Schema Edit Preview**: statements with summaries and
destructive flags, atomic or standalone groups, and engine notes (rewrite,
lock, rebuild, non-atomic apply). The UI never sends SQL (ADR-0011).

Apply re-plans and refuses unless the preview equals the reviewed one, asserts
`WriteIntent::Ddl` (ADR-0004), relies on the host having journaled the
attempt, then the engine rechecks the **Drift Token**, executes and verifies
by re-introspecting. A drifted baseline is refused with `BaselineChanged`.

## Considered options

- **One `Engine` trait with capture/plan/execute.** Rejected: the engines
  differ in transactionality, connection ownership and failure semantics,
  and ADR-0002 keeps per-engine records.
- **Diffing generated `CREATE TABLE` text.** Rejected as unsound (ADR-0013);
  a definition line diff is display only.
- **Matching objects by name.** Rejected: a rename would read as drop plus
  add, which loses data.

## Consequences

- Expressions (defaults, checks, generated columns, predicates) are compared
  byte for byte. An untouched expression is the server's own rendering and is
  never a change; an edited one is never declared equal unless the bytes match.
  Types compare as normalized text.
- Destructive classification (drop, non-widening type change, set NOT NULL,
  NOT NULL without default, re-creation) drives review emphasis only. Gating
  stays `WriteIntent::Ddl`.
- A baseline that cannot be captured faithfully (unsupported server version,
  unparseable definition, catalog and definition disagreement) is `ReadOnly`
  with a reason. There is no guessing, because MySQL `MODIFY` and a SQLite
  rebuild restate whole columns and would silently drop unknown attributes.
- MySQL DDL commits implicitly, so a multi-statement plan reports the
  committed prefix (`Stopped`) and runs on a dedicated connection that never
  commits the user's session transaction. SQLite DDL runs on the session
  connection and is refused while the user has a transaction open.
- SQLite and MySQL definitions are read with hand-written parsers that keep
  every expression's original bytes; `sqlparser` only cross-checks that the
  definition parses. An untouched expression must be restated exactly, which
  a parse-and-print round trip does not guarantee.
- SQLite refuses an edit that breaks another table's foreign key (dropping a
  referenced column, or the key it matches), because sessions run with
  `foreign_keys` off and nothing else would catch the dangling reference.
- The draft and the apply attempt are journaled with the workspace
  (`WorkspaceSchemaEdit`). Restoring one restores the draft, never authority.
