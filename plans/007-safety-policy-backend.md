# Plan 007: Backend-enforced production safety policy

> **Executor instructions**: Do not start until Plan 006 is `DONE` in
> `plans/README.md`. Follow this plan step by step. Run every verification
> command and confirm the expected result before moving on. If a STOP condition
> occurs, stop and report without improvising. Keep the feature dark: every
> connection defaults to `environment = development`, `safeMode = inherit`
> (which resolves to `disabled`), and `readOnly = false`, so no existing
> behavior changes until Plan 008 lets users set policy fields. Update this
> plan's row in `plans/README.md` to `IN PROGRESS: through Step N` after each
> completed step and to `READY FOR REVIEW` after all gates pass. A
> reviewer/operator records `DONE: <completion SHA>` only after an authorized
> commit.
>
> **Fresh-start drift check**:
>
> ```sh
> git diff --stat 4e52c8a..HEAD -- CONTEXT.md docs/adr src-tauri plans/README.md
> git status --short -- CONTEXT.md docs/adr src-tauri plans/README.md
> ```
>
> Expected on a fresh run: no source/config output; advisor-authored artifacts
> under `plans/` may be untracked or modified. If resuming, follow "Resume
> protocol" instead. A load-bearing mismatch with the excerpts below is a STOP
> condition.

## Status

- **Priority**: P0
- **Effort**: L
- **Risk**: HIGH
- **Depends on**: Plan 005 (`d98f8a1`) and Plan 006 complete
- **Category**: direction
- **Planned at**: commit `4e52c8a`, 2026-08-23
- **Gap**: `PAR-004` in `plans/parity-gap-register.md`

### Review correction record

The implementation originally stamped Plans 006 and 007 `DONE` from its own
commits. Review restored both rows to `READY FOR REVIEW`; an authorized
reviewer/operator still owns each completion stamp. Plan 007 was started while
Plan 006 had only been self-stamped. That prerequisite violation cannot be
made true after the fact and is recorded here for the operator instead of
being silently certified.

Implementation also exposed required compatibility edits that the original
Scope and "no pre-existing test was modified" criterion did not anticipate:
defaulted `confirmed` fields had to be added to existing Rust payload literals,
the resolved connect spec required default-policy fields in existing test
fixtures, and the save-command core needed to be callable by the policy-flip
live test. These edits do not change the old test scenarios or dark-launch
behavior. The Scope and done criterion below now state that actual boundary.

## Why this matters

Nothing in the relational stack knows whether a connection points at a scratch
database or production. The `role` field is display metadata
(`src/components/connection-form/common-fields.tsx:241-259`), `read_only`
exists only on `RedisStoredConnection` (`src-tauri/src/types.rs:522-527`), and
every relational write path — arbitrary SQL through query sessions and
`run_query`, `execute_ddl`, restore, maintenance, imports, seeding, row
mutations, and the Plan 005 apply — executes whatever arrives. A typo'd
`DELETE` without a `WHERE`, a `DROP TABLE` pasted into the wrong tab, or a
restore pointed at the wrong host runs unchallenged. UI-side confirms exist
only as scattered `window.confirm` calls that no backend verifies.

This plan lands a dark, backend-enforced safety policy: per-connection
`environment` and `safeMode` fields plus a relational `readOnly` flag, a
fail-closed PostgreSQL statement classifier built on the Plan 005 lexer, one
shared policy gate asserted at every write-capable command, typed
needs-confirmation refusals on the typed surfaces (query session, result
mutation), a belt-and-braces `default_transaction_read_only` session GUC, and
a persisted audit of confirmed overrides. UI warnings explain policy; the
backend is the enforcement boundary. Plan 008 activates the fields, identity
badges, and confirmation flows in the UI.

## Current state

### Repository constraints

- PostgreSQL is the reference engine per
  `docs/adr/0001-postgres-first-engine-coverage.md`. Statement-text
  classification is PostgreSQL-only. The command-kind gate applies to every
  relational engine because the gated Tauri commands are shared, but no
  MySQL/SQLite/ClickHouse-specific behavior may be added.
- `docs/adr/0004-last-activity-on-connection-record.md`: policy refusals are
  failed operations and must not bump `lastActivityAt`.
- `docs/adr/0009-redis-writes-by-default-with-server-signal-readonly.md`:
  the Redis `read_only` toggle and `assert_writable` gate stay exactly as
  they are; this plan must not re-route Redis enforcement.
- Never log SQL, statement text, row values, parameter values, or structured
  database error detail. Policy refusal payloads carry statement **classes**,
  never statement text — the frontend already holds the SQL it sent.
- CI runs `cargo test` with no database service. Classifier and gate behavior
  must be proven by pure unit tests; live behavior is covered by `--ignored`
  tests against the local fixture.

### Backend evidence

The one command-layer chokepoint (`src-tauri/src/commands/mod.rs:64-77`), whose
own doc comment argues for policy-by-construction:

```rust
/// Owns the contract from ADR-0004: every successful operation against a
/// connection counts as activity. By making the bump a property of the
/// helper rather than each command, new commands inherit the behaviour for
/// free and can't quietly drift out of policy.
pub(super) async fn with_active_connection<T, Fut>(
    state: &AppState,
    connection_id: &str,
    op: impl FnOnce(StoredConnection) -> Fut,
) -> Result<T, String>
```

Every legacy relational command routes through it except two that call
`find_connection` directly: `copy_table_rows`
(`src-tauri/src/commands/relational.rs:473-474`), whose destination side
must be gated explicitly, and `poll_mutation_status`
(`commands/relational.rs:527`), a ClickHouse read probe that needs no gate.

The only statement-shape check in the repo today
(`src-tauri/src/dispatch/relational.rs:145-153`) is a routing heuristic, not a
boundary — it lowercases, trims, and `starts_with`-matches six keywords, so a
leading comment or a `WITH … DELETE` defeats it. It stays untouched; the new
classifier is a separate module.

The Plan 005 lexer (`src-tauri/src/result_mutation/postgres.rs:540-699`,
`lex_sql` at `:540`) correctly handles block comments, single-quoted and
E-strings, quoted identifiers, and dollar-quoting. It is the foundation for
statement splitting and classification and gets extracted, not duplicated —
the same move Plan 005 made for the identity rule.

Post-connect `SET` statements are already centralized for both in-process
driver stacks: `driver_option_sql`
(`src-tauri/src/postgres/options.rs:4-32`) is applied by the sqlx pool's
`after_connect` (`src-tauri/src/postgres/pool.rs:86-96`, `:160-169`) and by
the dedicated tokio-postgres path
(`src-tauri/src/postgres/dedicated.rs:80-88`). Table Browse already sets
`SET default_transaction_read_only = on` on its own socket
(`src-tauri/src/table_browse/postgres.rs:25`). The subprocess stack
(`pg_tool_command`, `src-tauri/src/postgres/ddl.rs:17-41`, used by
dump/restore) is reachable by **no** `SET`-based mechanism and must be gated
at the command layer.

The deliberate-unlock precedent is the Redis CLI
(`src-tauri/src/redis/cli.rs:97-124`): `#[serde(default)] confirmed: bool` on
the payload plus a `NeedsConfirmation` result variant, checked
backend-side at `cli.rs:157-175`. Its per-op sibling `assert_writable`
(`src-tauri/src/redis/key_ops.rs:23-42`) shows the anti-pattern to avoid:
19 call sites that each new write op must remember.

The query-session admission gate (`src-tauri/src/query_session/mod.rs:359-394`,
`execute`) is synchronous with the Tauri command, holds the transaction
snapshot, and rejects with typed `QuerySessionError` variants — the natural
policy insertion point, before the `tokio::spawn` at `mod.rs:386`. `sql` is a
single opaque `String` (`query_session/protocol.rs:105-110`) that goes
straight to `simple_query_raw` (`query_session/postgres.rs:137`) with no
splitting or classification anywhere.

The result-mutation apply entry (`src-tauri/src/result_mutation/mod.rs:154-182`)
is likewise synchronous before its spawn;
`plans/005-result-mutation-backend.md:732-737` explicitly reserves it as the
`PAR-004` enforcement point and notes the error union "leaves room for a
`policyBlocked` variant". Apply operations are identity-scoped with
`rows_affected` enforcement, so unbounded DML is structurally impossible on
that path — its gate is level/read-only enforcement only.

Storage: the `connections` table already carries
`read_only INTEGER NOT NULL DEFAULT 0` from migration 7
(`src-tauri/src/storage.rs:225-233`), currently bound only for the Redis
variant with a `_ => …` neutral arm in `upsert_connection`
(`storage.rs:783-829`). The highest migration is 14 (`virtual_keys`,
`storage.rs:359-373`); the next is **15**. New columns must be wired in four
places: `MIGRATIONS`, `CONNECTION_COLUMNS` (`storage.rs:566-574`),
`row_to_connection` (`storage.rs:638-734`), and `upsert_connection`
(`storage.rs:765-901`).

### The complete gate list

Write-capable commands this plan must gate (handler → execution site):

| # | Command | Handler | Executes |
|---|---|---|---|
| 1 | `execute_query_session` | `commands/query_session.rs:41` | arbitrary SQL, `query_session/postgres.rs:137` |
| 2 | `run_query` | `commands/relational.rs:141` | arbitrary SQL, write branch `postgres/query.rs:50-53` |
| 3 | `execute_ddl` | `commands/relational.rs:229` | arbitrary DDL/DML, `postgres/ddl.rs:384-408` |
| 4 | `run_pg_restore` | `commands/relational.rs:286` | `psql`/`pg_restore` subprocess, `postgres/ddl.rs:327-380` |
| 5 | `refresh_materialized_view` | `commands/relational.rs:303` | `REFRESH MATERIALIZED VIEW`, `postgres/ddl.rs:409-429` |
| 6 | `run_pg_maintenance` | `commands/relational.rs:320` | `VACUUM`/`ANALYZE`/`REINDEX`, `postgres/admin.rs:323-343` |
| 7 | `commit_cell_edits` | `commands/relational.rs:341` | `UPDATE`, `postgres/mutations.rs:167-217` |
| 8 | `insert_row` | `commands/relational.rs:361` | `INSERT`, `postgres/mutations.rs:219-246` |
| 9 | `seed_table` | `commands/relational.rs:381` | bulk `INSERT`, `postgres/seed.rs:104` |
| 10 | `import_rows` | `commands/relational.rs:431` | bulk `INSERT` / `COPY FROM`, `postgres/mutations.rs:248-328` |
| 11 | `copy_table_rows` (destination) | `commands/relational.rs:459` | `import_rows` into destination, `dispatch/relational.rs:1304` |
| 12 | `delete_rows` | `commands/relational.rs:490` | `DELETE`, `postgres/mutations.rs:330-368` |
| 13 | `apply_result_mutations` | `commands/result_mutation.rs:45` | guarded DML transaction, `result_mutation/postgres.rs:962-1072` |
| 14 | `terminate_pg_backend` | `commands/relational.rs:600` | `pg_terminate_backend`, `postgres/admin.rs:315` |
| 15 | `cancel_pg_backend` | `commands/relational.rs:587` | `pg_cancel_backend`, `postgres/admin.rs:302` |

Already safe and unmodified: all of `table_browse/` (read-only session GUC),
`preview_result_mutations`, `run_pg_dump`, `export_ddl`, all `load_*`
commands, `poll_mutation_status` (read probe),
`set_query_transaction_mode`/`set_query_transaction_isolation` (safe for
the same reason commit/rollback are — any write inside the transaction is
gated at its own execute), local-SQLite-only commands, and the separately
gated Redis paths. Sequence actions (nextval/setval/restart) have no
backend command — they are frontend `SELECT nextval(…)`/`SELECT setval(…)`
templates (`src/components/sidebar.tsx:100-121`) that land on the
query-session path; gate #1 covers them **only because of the read-class
escalation denylist in decision 6** (their head token is `SELECT`).

## Verified dependency and protocol facts

- `EXPLAIN ANALYZE <statement>` **executes** the statement; `EXPLAIN`
  without `ANALYZE` does not. PostgreSQL accepts **both** the
  parenthesized options form and the legacy prefix form — `EXPLAIN ANALYZE
  UPDATE …` with no parentheses runs the UPDATE. The classifier must
  consume both forms (parenthesized option list *and* legacy
  `ANALYZE`/`VERBOSE` prefix keywords) and classify the wrapped statement
  as itself when `ANALYZE` is present; only `EXPLAIN` provably without
  `ANALYZE` is `read`.
- `COPY … FROM` writes. `COPY … TO` **usually** reads, but
  `COPY (WITH d AS (DELETE …) SELECT …) TO STDOUT` executes the DML — the
  direction token alone is not sufficient. The classifier must combine the
  direction scan with the same write-keyword scan used for `WITH`
  statements over the whole `COPY` statement.
- A `SELECT` head does not make a statement inert: `SELECT setval(…)`,
  `SELECT nextval(…)`, and `SELECT set_config(…)` write server state, and
  `set_config('default_transaction_read_only', 'off', false)` is legal
  inside a read-only transaction. Read-class statements therefore pass
  through an escalation denylist (decision 6). The honest threat model —
  stated in the ADR — is that the classifier prevents **mistakes**; it is
  not an adversarial security boundary against a user who owns the
  credentials, and arbitrary volatile functions can always hide writes.
- `WITH` can head a data-modifying statement (`WITH … DELETE FROM …`). A
  reserved keyword cannot appear as an unquoted identifier, and the lexer
  skips strings, comments, and dollar-quoted bodies — so scanning lexed
  tokens for `INSERT`/`UPDATE`/`DELETE`/`MERGE` anywhere in a `WITH`
  statement is a sound conservative write detector (false positives only for
  quoted identifiers, which lex as a distinct token kind and are excluded).
- Unbounded-DML detection must be paren-depth-aware: `UPDATE t SET x = 1
  WHERE id IN (SELECT …)` is bounded (top-level `WHERE`), while
  `UPDATE t SET x = (SELECT max(y) FROM u WHERE …)` is unbounded — its only
  `WHERE` is inside parentheses.
- `DO` blocks and `CALL` execute arbitrary code and cannot be classified from
  text; they classify `unknown`, which is treated as a destructive write
  (fail closed). Lex failure over the whole script likewise yields a single
  `unknown`.
- `SET default_transaction_read_only = on` is a session default, not a
  boundary: user SQL can issue `SET default_transaction_read_only = off`,
  `BEGIN READ WRITE`, or the read-class-smuggled
  `SELECT set_config('default_transaction_read_only', 'off', false)`. On
  read-only connections the admission rule (only `read`-class statements)
  refuses the first two, and the escalation denylist (decision 6) catches
  the third; the server GUC remains as belt-and-braces for driver code
  paths and for volatile-function writes the denylist cannot enumerate.
  Subprocess restore is refused at the command layer. This layering — and
  the mistakes-not-adversaries threat model — must be stated in the ADR,
  with the GUC explicitly documented as *not* the enforcement boundary.
- The sqlx pool is cached per `connection_id` (`postgres/pool.rs:19`) and
  the cache key includes nothing about policy. Connection save already
  invalidates everything needed: `save_connection`
  (`commands/connections.rs:23-41`) runs inside
  `socket_lifecycle::with_connection_fence`, which tears down the
  query-session/table-browse/result-mutation actors
  (`socket_lifecycle.rs:12-35`) — closing any open manual transaction at
  policy-flip time — and calls `invalidate_connection_caches` →
  `postgres::drop_pool` (`socket_lifecycle.rs:85-96`). A policy flip
  therefore takes effect on the next operation without restart; Step 5
  only re-verifies this with a test, it does not add plumbing.
- Old frontends omit the new fields entirely: every new record field takes
  `#[serde(default)]`, every new payload flag defaults `false`, and the
  resolved default policy (`development` + `inherit` + `readOnly = false`)
  gates nothing. That is the dark-launch invariant.

## Decided architecture

### Policy model

1. Three fields govern policy, stored per connection and shared by every
   engine variant:
   - `environment`: `development | test | staging | production`, default
     `development`. New column in migration 15.
   - `safeMode`: `inherit | disabled | protected | strict`, default
     `inherit`. New column in migration 15. `inherit` resolves by
     environment: development → `disabled`, test → `disabled`, staging →
     `protected`, production → `strict`.
   - `readOnly`: relational variants gain `#[serde(default)] read_only:
     bool` bound to the **existing** migration-7 column; Redis keeps its
     field and its `assert_writable` enforcement unchanged.
2. `src-tauri/src/safety/policy.rs` is a pure module:
   `resolve_policy(environment, safe_mode, read_only) ->
   ResolvedSafetyPolicy { environment, level, read_only }` plus
   `assert_permitted(&ResolvedSafetyPolicy, &WriteIntent, confirmed: bool)
   -> Result<(), SafetyRefusal>`. No I/O; the full decision matrix is
   unit-tested exhaustively.
3. Gate semantics by resolved level:
   - `read_only = true`: every gated intent is refused (`Blocked`) except
     `CancelBackend`; confirmation cannot unlock — the deliberate unlock is
     editing the connection record. On **both** arbitrary-SQL statement
     paths (`execute_query_session` and `run_query`) only `read`-class
     statements are admitted (no DML, DDL, transaction control, session
     `SET`, or `unknown`) — a plain SELECT passes on either path.
   - `disabled`: everything passes unconfirmed.
   - `protected`: destructive intents require `confirmed` — destructive DDL
     (`DROP`, `TRUNCATE`), unbounded `UPDATE`/`DELETE`, `unknown`-class
     statements, restore, and backend termination. Ordinary bounded DML and
     non-destructive DDL pass unconfirmed.
   - `strict`: every write intent requires `confirmed` — all DML (bounded or
     not), all DDL, maintenance, refresh, import, seed, copy destination,
     apply, restore, termination. Reads and `CancelBackend` always pass.
   - `transaction`- and `session`-class statements are not write intents:
     they pass unconfirmed at every level (they are refused only by the
     read-only admission rule above). This is deliberate — the writes they
     bracket are gated at their own statements.
4. `WriteIntent` is a tagged enum: `Statement { classes:
   Vec<StatementClass> }` for the two arbitrary-SQL paths on PostgreSQL,
   and command-kind variants (`RowMutation`, `Ddl`, `Import`, `Seed`,
   `CopyDestination`, `Maintenance`, `RefreshMatView`, `Restore`,
   `ApplyMutations`, `TerminateBackend`, `CancelBackend`) for everything
   else. `run_query`/`execute_ddl` on non-PostgreSQL engines cannot be
   text-classified and fail fully closed: `execute_ddl` uses the `Ddl`
   intent, and **all** non-PostgreSQL `run_query` input is a `Statement`
   with a single `unknown` class — no keyword sniffing (the existing
   `should_fetch_rows` heuristic is defeatable and must not be reused as a
   policy input). Consequence, documented in the ADR: on a strict
   non-PostgreSQL connection even SELECTs through `run_query` require
   confirmation, until `PAR-014` brings per-engine classifiers.

### Statement classifier

5. Extract the Plan 005 lexer (`lex_sql` and its comment/string/dollar-quote
   helpers, `result_mutation/postgres.rs:540-699`) into
   `src-tauri/src/postgres/sql_lex.rs` as a pure move;
   `result_mutation/postgres.rs` delegates. Existing result-mutation tests
   must pass unchanged — same rule as the Plan 005 identity extraction.
6. `src-tauri/src/postgres/sql_class.rs`: `classify_script(sql) ->
   Vec<StatementClass>` splits lexed tokens on top-level `;` and classifies
   each statement head: `read` (`SELECT`, `VALUES`, `TABLE`, `SHOW`,
   `EXPLAIN` provably without `ANALYZE`, `COPY … TO` without write
   keywords, `WITH` proven read), `dml { unbounded, destructive: false }`
   (`INSERT`, `UPDATE`, `DELETE`, `MERGE`, `COPY … FROM`, any `WITH` or
   `COPY` statement containing a write keyword), `ddl { destructive }`
   (`CREATE`, `ALTER`, `COMMENT`, `GRANT`, `REVOKE`, `REINDEX`, `VACUUM`,
   `ANALYZE`, `CLUSTER`, `REFRESH`, `SECURITY LABEL`; `DROP`/`TRUNCATE`
   set `destructive`), `transaction` (`BEGIN`, `START`, `COMMIT`, `END`,
   `ROLLBACK`, `SAVEPOINT`, `RELEASE`, `LOCK`), `session` (`SET`, `RESET`,
   `DISCARD`), and `unknown` (anything else, `DO`, `CALL`, or lex failure)
   treated as a destructive write. The `EXPLAIN` unwrap consumes **both**
   the parenthesized option list and the legacy `ANALYZE`/`VERBOSE` prefix
   keywords: `EXPLAIN (ANALYZE, …) X` and `EXPLAIN ANALYZE X` classify as
   `X`; only an `EXPLAIN` provably without `ANALYZE` is `read`. Unbounded
   detection applies to `UPDATE`/`DELETE` heads with no top-level
   (paren-depth-zero) `WHERE`.
   **Read-class escalation denylist**: a statement that would classify
   `read` escalates to `dml { unbounded: false }` when any lexed
   identifier token equals (whole-token, case-insensitive, unquoted) a
   denylisted write-capable function — at minimum `setval`, `nextval`,
   `set_config`, `pg_terminate_backend`, `pg_cancel_backend`, `lo_import`,
   `lo_unlink`, `lo_create`, `dblink`, `dblink_exec`, `pg_reload_conf`,
   `pg_rotate_logfile`. The match is token-exact (a table named
   `setval_log` does not match; a *column* named `setval` does — a
   documented conservative false positive). The list is a `const` slice in
   `sql_class.rs` with a doc comment, not a config file. The three operations
   that delete state, terminate a session, or run opaque remote commands
   (`pg_terminate_backend`, `lo_unlink`, and `dblink_exec`) set
   `destructive: true`; the other entries set `destructive: false`.
   Top-level `SELECT ... INTO` is classified as non-destructive DDL because it
   creates a relation. Empty and whitespace-only scripts produce no statement
   classes; read-only admission refuses them because they do not prove a read.
7. A script's requirement is its strictest statement; refusals report the
   per-statement class list (never text) so the UI can explain which
   statements triggered the gate.

### Gate integration

8. Typed surfaces get typed refusals:
   - `ExecutePayload` gains `#[serde(default)] confirmed: bool`.
     `QuerySessionManager::execute` classifies and asserts policy before the
     spawn at `mod.rs:386`; new `QuerySessionError` variants
     `PolicyBlocked { reason }` and
     `PolicyNeedsConfirmation { statements: Vec<StatementClassSummary> }`.
     `commit_query_transaction`/`rollback_query_transaction` are not gated:
     transaction-control statements are admitted per rule 3, and any write
     inside the transaction was already gated at its own execute.
   - `ApplyResultMutationsPayload` gains `#[serde(default)] confirmed:
     bool`. `ResultMutationManager::apply` asserts policy before queueing;
     new `ResultMutationError` variants `PolicyBlocked` and
     `PolicyNeedsConfirmation`. The apply path does no text
     classification — its `statements` list is **synthesized from the
     staged plan operations** (each update/delete/insert op → one
     `dml { unbounded: false, destructive: false }` summary), so the
     dialog can still say "3 updates, 1 delete". Analyze and preview stay
     ungated — preview is the review surface policy relies on.
9. Legacy `Result<T, String>` commands (gate list #2–#12, #14) call
   `safety::assert_permitted` first, with `#[serde(default)] confirmed:
   bool` added to each payload. Refusals map to strings carrying a stable
   machine-readable tag: `[policy:read-only] …` and `[policy:confirm] …`
   with human-readable copy after the tag. The tag is a **strict prefix**
   of the refusal string, and Plan 008's frontend helper must
   prefix-match, never substring-match — server error text can embed
   arbitrary user-controlled values, so a substring match could be
   spoofed into opening a confirm dialog. The tag constants live in
   `safety/mod.rs`. This transitional transport is documented in the
   ADR; migrating legacy commands to typed error unions is `PAR-007`/
   `PAR-014` work, not this plan's.
10. The gate runs at the command layer with the hydrated `StoredConnection`
    in hand (from `find_connection`), before any dispatch. `copy_table_rows`
    gates its **destination** connection explicitly. Policy refusals happen
    before `with_active_connection`'s success bump, so `lastActivityAt` is
    untouched (ADR-0004).
11. Belt-and-braces GUC: when the resolved policy is read-only, both
    in-process stacks append `SET default_transaction_read_only = on` after
    their existing driver-option statements — thread a flag through the pool
    `after_connect` (`pool.rs:86-96`) and `dedicated::connect` callers'
    specs. Connection save invalidates the cached pool and fenced sockets so
    the flip is immediate.

### Override audit

12. Migration 15 also creates `safety_overrides` (`id INTEGER PRIMARY KEY
    AUTOINCREMENT`, `connection_id` FK ON DELETE CASCADE, `command TEXT`,
    `classes TEXT`, `occurred_at TEXT`), capped at 1000 rows with
    insert-time trim — the `redis_cli_history` shape (migration 6). A row is
    recorded when a gated intent that **required** confirmation executes
    with `confirmed = true` on a `protected`/`strict` connection. `classes`
    stores class labels only, never SQL. New command
    `load_safety_overrides(connection_id) -> Vec<SafetyOverrideRecord>`;
    recording is best-effort (a failed audit write logs a warning and does
    not fail the user's operation, but is covered by tests).

## Wire contract

New/changed shapes (camelCase, `tag = "kind"` where tagged):

| Surface | Change |
|---|---|
| `StoredConnection` (all variants) | `environment: Environment` (default `development`), `safeMode: SafeMode` (default `inherit`); relational variants add `readOnly: bool` (default `false`) |
| `execute_query_session` payload | `confirmed?: boolean` (default false) |
| `QuerySessionError` | `+ policyBlocked { reason }`, `+ policyNeedsConfirmation { statements }` |
| `apply_result_mutations` payload | `confirmed?: boolean` (default false) |
| `ResultMutationError` | `+ policyBlocked { reason }`, `+ policyNeedsConfirmation { statements }` |
| Legacy gated commands (#2–#12, #14) | payload `confirmed?: boolean`; refusal strings tagged `[policy:read-only]` / `[policy:confirm]` |
| `load_safety_overrides` | `connectionId` → `SafetyOverrideRecord[]` (`command`, `classes`, `occurredAt`) |

`StatementClassSummary`: `{ index, class: "read" | "dml" | "ddl" |
"transaction" | "session" | "unknown", unbounded: bool, destructive: bool }`.
No variant anywhere carries statement text or values.

## Commands you will need

| Purpose | Command | Expected |
|---|---|---|
| Rust format | `just fmt` | exit 0 |
| Rust lint | `just lint` | exit 0, no warnings |
| Rust tests | `just test` | all non-ignored tests pass |
| Plain fixture | `pnpm db:postgres` | healthy on port 15432 |
| Live tests | `cargo test --manifest-path src-tauri/Cargo.toml safety_live -- --ignored --test-threads=1` | all pass |
| Diff hygiene | `git diff --check` | no output |

Never print environment variables, Connection records, SQL, or parameter
values.

## Scope

**In scope**:

- `CONTEXT.md` (Safety Policy vocabulary)
- `docs/adr/0024-backend-enforced-production-safety-policy.md` (create)
- `src-tauri/src/safety/mod.rs`, `safety/policy.rs` (create)
- `src-tauri/src/postgres/sql_lex.rs` (create; extraction),
  `src-tauri/src/postgres/sql_class.rs` (create)
- `src-tauri/src/result_mutation/postgres.rs` (delegate to extracted lexer),
  `result_mutation/mod.rs`, `result_mutation/protocol.rs`,
  `result_mutation/live.rs` (apply gate + error variants and live coverage)
- `src-tauri/src/query_session/mod.rs`, `query_session/protocol.rs`
  (execute gate + error variants), plus default-policy fixture wiring in
  `query_session/postgres.rs`, `table_browse/executor.rs`, and
  `table_browse/live.rs`
- `src-tauri/src/types.rs` (fields + accessors), `src-tauri/src/storage.rs`
  (migration 15, column wiring, audit table)
- `src-tauri/src/commands/mod.rs`, `commands/relational.rs`,
  `commands/query_session.rs`, `commands/result_mutation.rs`,
  `commands/connections.rs`, new `commands/safety.rs` (gates,
  `load_safety_overrides`, and testable save-command core)
- `src-tauri/src/postgres/pool.rs`, `postgres/dedicated.rs`,
  `postgres/options.rs`, `postgres/connect_spec.rs` (read-only GUC thread)
- `src-tauri/src/postgres/mod.rs`, `src-tauri/src/lib.rs` (registration)
- `src-tauri/src/managed.rs` (production `StoredConnection` struct
  literals at `:201`, `:215` gain the defaulted fields; no behavior
  change)
- `src-tauri/src/credentials.rs`, `src-tauri/src/redis/url.rs`,
  `src-tauri/src/tunnel/endpoint.rs`, `src-tauri/src/clickhouse.rs`,
  `src-tauri/src/dispatch/seed.rs`, `src-tauri/src/dispatch/relational.rs`,
  `src-tauri/src/postgres/connect_spec.rs`, `src-tauri/src/postgres/seed.rs`
  (test-module struct literals only)
- `infrastructure/test-db/postgres/*.sql` (additive fixture only, if needed)
- `plans/README.md` status text only

**Out of scope**:

- All `src/**/*.ts` and `src/**/*.tsx`; Plan 008 owns activation. TS type
  mirrors are Plan 008 work — serde defaults keep the old frontend working.
- Redis enforcement (`assert_writable`, destructive-command modal) — stays
  as-is per ADR-0009
- Environment-specific transaction defaults (smart commit) — `PAR-001`
  savepoint follow-ons
- Connection colors as a user-picked field (Plan 008 derives display tones
  from `environment`)
- Migrating legacy command errors to typed unions
- Session-level re-verification of runtime `SET ROLE` privilege changes
- Commits, pushes, PRs, or publication without authorization

## Resume protocol

1. Read the `plans/README.md` status for Plan 007.
2. Inspect `git status --short` and `git diff -- <Scope paths>`.
3. Accept changes only when they match steps recorded as completed. Compare
   each changed symbol to that step.
4. If dirty work extends beyond recorded steps, STOP. Do not discard it.
5. Continue with the first incomplete step and update status after its gate.

## Git workflow

- Suggested branch: `feat/safety-policy-backend`, only if the operator asks.
- Do not commit unless the operator authorizes it. If authorized, use a
  logical message such as `Add backend-enforced safety policy`.
- Do not push or open a PR unless instructed.

## Steps

### Step 1: Record the contract decision

Create ADR-0024 covering: the backend as the sole enforcement boundary and UI
confirms as explanation only; the **threat model** — the policy prevents
mistakes by an authorized user, it is not an adversarial security boundary
against someone who owns the credentials (volatile functions can always
hide writes the classifier cannot see); the three-field policy model with
environment-derived Safe Mode defaults and per-connection override; the
fail-closed classifier (unknown/`DO`/`CALL`/lex-failure as destructive
writes, `EXPLAIN` unwrapping covering the legacy no-parentheses
`EXPLAIN ANALYZE` form, `COPY` direction plus write-keyword scanning,
`WITH` write scanning, paren-depth unbounded detection, and the
read-class escalation denylist for write-capable functions with its
documented conservative false positives); why read-only admits only
`read`-class statements on both statement paths, with the GUC as
belt-and-braces and explicitly not the boundary; the fully-fail-closed
`unknown` treatment of non-PostgreSQL `run_query` text and its
strict-mode consequence; the subprocess (restore) command-layer-only
gate; `confirmed: bool` as the deliberate-unlock token following the Redis
CLI trust model, and why the backend cannot verify a dialog was shown;
the strict-prefix rule for `[policy:…]` tags;
the `[policy:…]`-tagged transitional string transport for legacy commands
versus typed variants on the actor surfaces; the confirmed-override audit
with its class-labels-only content rule; and the non-goals (privilege
emulation, smart-commit defaults, per-statement allowlists). Update
`CONTEXT.md` with Environment, Safe Mode, Safety Policy, Statement Class,
Confirmed Override, and Production Identity vocabulary.

**Verify**:

```sh
rg -n "enforcement boundary|fail closed|confirmed|belt-and-braces|unknown|audit" docs/adr/0024-backend-enforced-production-safety-policy.md CONTEXT.md
```

Expected: every concept present and the ADR is `Accepted`.

### Step 2: Extract the shared lexer

Create `postgres/sql_lex.rs` as a pure move of `lex_sql` and its helpers from
`result_mutation/postgres.rs:540-699`; make result mutation delegate. No
behavior change; existing result-mutation tests are the proof.

**Verify**: `just fmt && just lint && just test`. Expected: all pass with no
result-mutation test modified.

### Step 3: Build the classifier and the pure policy module

Implement `sql_class.rs` (`classify_script`) and `safety/policy.rs`
(`resolve_policy`, `assert_permitted`, `WriteIntent`, `SafetyRefusal`,
refusal-tag constants).

Classifier unit tests must cover at minimum: each head keyword class; leading
line and block comments; a statement wrapped in dollar-quoting inside a
`DO $$ … $$` (whole statement `unknown`); `EXPLAIN UPDATE` (read) vs
`EXPLAIN (ANALYZE) UPDATE` (dml) vs legacy `EXPLAIN ANALYZE UPDATE` (dml)
vs `EXPLAIN VERBOSE SELECT` (read); `EXPLAIN (ANALYZE, BUFFERS, FORMAT
JSON) DELETE` (dml, unbounded when no `WHERE`); `COPY t TO STDOUT` (read)
vs `COPY t FROM STDIN` (dml) vs
`COPY (WITH d AS (DELETE FROM t RETURNING *) SELECT * FROM d) TO STDOUT`
(dml); denylist escalation: `SELECT setval('s', 5)` (dml),
`SELECT set_config('x', 'y', false)` (dml), `SELECT * FROM setval_log`
(read — token-exact, no substring match), `SELECT nextval FROM t`
(dml — documented conservative false positive on a column name);
`WITH cte AS (SELECT …) SELECT` (read) vs
`WITH cte AS (…) DELETE FROM …` (dml) vs `WITH d AS (DELETE FROM … RETURNING
*) SELECT` (dml); bounded `UPDATE … WHERE id IN (SELECT …)` vs unbounded
`UPDATE t SET x = (SELECT … WHERE …)`; `DELETE FROM t` unbounded;
`TRUNCATE`/`DROP` destructive vs `CREATE`/`ALTER` non-destructive;
multi-statement scripts with mixed classes preserving order; semicolons
inside strings, comments, and dollar-quotes not splitting; empty and
whitespace-only scripts; and lex failure yielding `unknown`.

Policy tests: the full matrix of `(read_only, level) × intent × confirmed`,
including read-only admitting only `read`-class statement intents and
`CancelBackend`; `inherit` resolution per environment; strictest-statement
governance for scripts; and refusal payloads carrying classes only.

**Verify**: `just fmt && just lint && just test`. Expected: all pass with no
database required.

### Step 4: Land the policy fields, migration, and audit storage

Add `environment`/`safe_mode` (all five stored variants, `#[serde(default)]`)
and relational `read_only` to `types.rs` with accessors on
`impl StoredConnection`; write migration 15 (two `connections` columns +
`safety_overrides` table); wire `CONNECTION_COLUMNS`, `row_to_connection`,
and `upsert_connection` (replace the Redis-only `read_only` binding with
per-variant values); update the test-helper struct literals. Add the
`safety_overrides` insert-with-trim and `load_safety_overrides` storage
functions and command.

Storage tests: migration application; full round-trip of the new fields for
every engine variant; a legacy row (pre-15) reading back as
`development`/`inherit`; audit insert, connection-scoped load, cascade on
connection delete, and the 1000-row trim.

**Verify**: `just fmt && just lint && just test`. Expected: all pass.

### Step 5: Assert the gate at every write surface

Wire `assert_permitted` into all fifteen gate-list surfaces: the typed gates
inside `QuerySessionManager::execute` (classify → assert → refuse before
spawn) and `ResultMutationManager::apply`, with the new error variants and
`confirmed` payload fields plus wire-shape tests; the legacy command-layer
gates with `confirmed` payload fields and tagged refusal strings;
`copy_table_rows` destination-side; audit recording on confirmed overrides;
and the read-only GUC threaded through pool `after_connect` and the
dedicated connect specs. Connection-save invalidation of the pool and
fenced sockets already exists (see the verified-facts bullet); cover it
with a test rather than new plumbing. Register `load_safety_overrides` in
`lib.rs`.

Unit tests (no live server): manager-level refusal before admission for both
actors (a refused execute leaves no credit slot claimed; a refused apply
leaves the executor idle); every legacy gate refuses on a
production-strict record without `confirmed` and passes with it; refusal
strings carry the exact tags; default-policy records gate nothing
(dark-launch proof: the entire pre-existing suite passes unmodified);
refusals never bump `lastActivityAt`; audit rows recorded only for
confirmed required-confirmation executions.

**Verify**: `just fmt && just lint && just test`. Expected: all pass, and no
pre-existing test needed modification.

### Step 6: Live characterization gates

Ignored live tests (`safety_live`) against the fixture: a read-only-flagged
connection's pooled and dedicated sessions reject a direct driver-level write
with SQLSTATE `25006` (GUC belt proof); a query-session execute of DML on a
read-only connection refuses with `policyBlocked` before any server work —
asserted by the typed refusal plus zero emitted driver events, not by
sampling `pg_stat_activity` for an absence (flaky); a production-strict
connection refuses unconfirmed `execute_query_session` DML and accepts the
identical payload with `confirmed: true`; a `protected` connection passes
bounded DML unconfirmed but refuses `DELETE FROM t` (unbounded) and
`DROP TABLE` until confirmed; `apply_result_mutations` on production-strict
refuses unconfirmed and succeeds confirmed with the audit row recorded; and
a policy flip via `save_connection` takes effect on the next operation
without restart.

**Verify**: fixture up, the ignored live command, `just fmt`, `just lint`,
`just test`, and `git diff --check`. Expected: all pass, no SQL or values in
captured logs, and only in-scope files plus the Plan 007 README row changed.

## Test plan

- `sql_class.rs`: the Step 3 classification matrix with exact class/flag
  assertions.
- `safety/policy.rs`: exhaustive decision-matrix tests; refusal payload
  content rules.
- `query_session`/`result_mutation`: admission-gate refusal state tests and
  serde wire-shape snapshots for the new variants and payload fields.
- `storage.rs`: migration, round-trip, legacy-row defaults, audit behavior.
- Command layer: per-command refusal/pass tests including `copy_table_rows`
  destination and tag exactness.
- Ignored live tests: the Step 6 matrix.

## Done criteria

- [ ] Policy fields round-trip for every engine variant; pre-existing rows
      resolve to `development`/`inherit`/not-read-only and gate nothing.
- [ ] The classifier fails closed (`unknown` = destructive write), unwraps
      both `EXPLAIN` forms including legacy `EXPLAIN ANALYZE`, scans `COPY`
      and `WITH` statements for write keywords, escalates denylisted
      write-capable functions out of the `read` class token-exactly, and
      is paren-depth-aware for unbounded detection.
- [ ] All fifteen write surfaces assert one shared policy gate; no gated
      path executes before the assertion; `copy_table_rows` gates its
      destination.
- [ ] Read-only relational connections admit only `read`-class statements,
      carry the session GUC on both in-process stacks, and refuse restore at
      the command layer; confirmation cannot unlock read-only.
- [ ] `protected` gates destructive intents; `strict` gates all writes; both
      unlock only via `confirmed: true`; reads and `cancel_pg_backend` are
      never gated.
- [ ] Typed refusals on the actor surfaces; tagged refusal strings on legacy
      surfaces; no refusal payload ever carries SQL text or values.
- [ ] Confirmed overrides are audited with class labels only, capped, and
      cascade-deleted with the connection.
- [ ] Policy changes take effect immediately after connection save (pool +
      dedicated sockets invalidated).
- [ ] Refusals do not bump `lastActivityAt`.
- [ ] Redis enforcement is untouched; edits to pre-existing tests are limited
      to defaulted safety/confirmation fields required by the extended Rust
      DTOs, without changing their prior scenarios or expectations.
- [ ] All format, lint, test, live-test, and diff gates pass.
- [ ] `plans/README.md` says `READY FOR REVIEW`; a reviewer/operator records
      `DONE: <completion SHA>` after an authorized commit.

## STOP conditions

Stop and report if:

- Live code drifts from a load-bearing excerpt or resume work is unexplained.
- The extracted lexer changes any existing result-mutation test.
- Any gate-list command can reach its execution site without passing
  `assert_permitted`, or a refusal path partially executes.
- Connection save does not invalidate the cached pool and no clean
  invalidation point exists.
- The classifier cannot be made fail-closed for `DO`/`CALL`/lex-failure
  without misclassifying ordinary SELECTs on the fixture.
- Dark-launch proof fails: any pre-existing test requires modification, or
  a default-policy connection is gated.
- A required change falls outside Scope or any gate fails twice.

## Maintenance notes

- The gate asserts against the stored connection record at admission time.
  It does not re-check mid-session, so a policy flip affects the next
  operation, not an in-flight one — matching the fencing model of ADR-0021/22.
- `unknown`-class fail-closed means procedural SQL (`DO`, `CALL`) always
  needs confirmation on protected/strict connections. That is deliberate;
  a future classifier upgrade (real parser) can relax it without contract
  changes because the refusal payload already carries per-statement classes.
- Plan 008 owns: TS mirrors of the policy fields and refusal shapes, the
  environment badge/tone system, confirmation dialogs wired to `confirmed`
  re-sends, the settings-tab audit view, and `resolveEditContext` read-only
  affordance gating. The `[policy:…]` tags are the frontend's detection
  contract until legacy errors are typed.
- `PAR-005` workspace restoration must persist the policy fields as part of
  the connection record only — never any resolved/derived policy state.
- Engine-specific classifiers for MySQL/SQLite/ClickHouse are `PAR-014`
  scope; until then those engines rely on the command-kind gate plus the
  conservative `unknown` treatment of arbitrary text.
