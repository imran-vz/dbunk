# dbunk parity plans

Generated from the DBeaver and TablePlus parity audit on 2026-08-18 at commit
`24432fb`.

This directory is the execution handoff for closing the documented parity gaps.
The canonical inventory is [parity-gap-register.md](./parity-gap-register.md).
Plans should be executed in dependency order unless a plan explicitly says
otherwise. Executors must read a plan completely, honor its STOP conditions,
and update its status here when work finishes.

## Execution order and status

| Plan                                                  | Title                                                            | Priority | Effort | Depends on | Status                                         |
| ----------------------------------------------------- | ---------------------------------------------------------------- | -------: | -----: | ---------- | ---------------------------------------------- |
| [001](./001-query-session-foundation.md)              | PostgreSQL Query Session backend foundation                      |       P0 |      L | None       | DONE: 657553d                                  |
| [002](./002-query-session-editor-integration.md)      | PostgreSQL Query Session editor integration                      |       P0 |      L | 001        | DONE: 26268ca (selected mock: B)               |
| [003](./003-table-browse-backend.md)                  | PostgreSQL Table Browse backend                                  |       P0 |      L | 001, 002   | DONE: 202f756                                  |
| [004](./004-table-browse-grid-integration.md)         | Server-backed browsing in table tabs                             |       P0 |      L | 003        | DONE: ecefce8 (selected mock: B)               |
| [005](./005-result-mutation-backend.md)               | PostgreSQL Result Mutation backend                               |       P0 |      L | 003, 004   | DONE: d98f8a1                                  |
| [006](./006-staged-mutation-review-integration.md)    | Staged mutation review in table and query results                |       P0 |      L | 005        | DONE: 4e52c8a (selected mock: A)               |
| [007](./007-safety-policy-backend.md)                 | Backend-enforced production safety policy                        |       P0 |      L | 005, 006   | DONE: bd9f7ef                                  |
| [008](./008-safety-policy-activation.md)              | Safety policy activation and production identity                 |       P0 |      L | 007        | DONE: 5409d66 (selected mock: C)               |
| [009](./009-workspace-navigation-foundation.md)       | Workspace navigation foundation (dark)                           |       P0 |      L | 001–008    | DONE: f66abaa                                  |
| [010](./010-open-anything-activation.md)              | Open Anything activation and connection organization             |       P0 |      L | 009        | DONE: 4facea1 (selected mock: A)               |
| [011](./011-connection-security-backend.md)           | PostgreSQL connection security backend (dark)                    |       P1 |      L | 001–010    | DONE: b134766                                  |
| [012](./012-connection-security-activation.md)        | TLS controls, staged connection diagnosis, and truth pass        |       P1 |      L | 011        | DONE: b45e294 (selected mock: A)               |
| [013](./013-object-catalog-ddl-backend.md)            | PostgreSQL object catalog and DDL workflow backend (dark)        |       P1 |      L | 001–012    | DONE: 4833a42                                  |
| [014](./014-object-explorer-lifecycle-activation.md)  | Object explorer, viewers, and lifecycle activation               |       P1 |      L | 013        | DONE: 2e843a6 (selected mock: C)               |
| [015](./015-structure-editor-typed-ddl-switchover.md) | PostgreSQL structure editor switchover to the typed DDL workflow |       P1 |      M | 013, 014   | IN PROGRESS: Steps 1–3 + Step 4 docs/gates done; Step 4 manual dev-app pass pending |

Status values: `TODO`, `IN PROGRESS: through Step N`, `READY FOR REVIEW`,
`DONE: <completion SHA>`, `BLOCKED: <reason>`, or `REJECTED: <reason>`.

Executors update their own status row after each completed step and mark
`READY FOR REVIEW` after all gates. The reviewer or operator records
`DONE: <completion SHA>` after the work is committed. This makes resume state
useful without authorizing commits implicitly.

## Current selection

- **Plan 015 delivery note (2026-09-01):** Steps 2–4 implemented in the
  working tree on top of the Step 1 commit `b468108`. PostgreSQL column
  forms produce typed ops directly (tagged literal/expression defaults,
  `USING` on type changes); the pending-changes section loads the
  backend's grouped per-statement preview (shared
  `DdlPlanPreviewGroups`, extracted from the Plan 014 dialog) with a
  reviewed gate — any pending edit reloads it and disables Commit; the
  destructive confirm sources its summaries from the preview; the store
  routes `pg-op` batches through `apply_object_ddl` with the typed
  confirmation retry, formats failures against the reviewed statement
  summaries, and on full or partial apply bumps `pgObjectDdlVersion`,
  runs the structure/data refresh fan-out, and revalidates loaded
  object descriptions the batch touched (partial applies keep the
  batch queued for edit-and-retry). The PK/FK/index/constraint sections
  gained add/drop affordances (index drops offer `CONCURRENTLY`; Add
  primary key hides when a PK exists), and the specialized index and
  cross-table FK panels queue the same typed ops into the shared
  pending list (`quoteIdent` retired in favour of `pgQuoteIdent` from
  the ddl lib for the still-generate-only GRANT/RLS/trigger panels).
  ClickHouse is behaviourally unchanged (its tests untouched). A
  2026-09-01 review pass hardened the first delivery: the loaded
  preview carries its execution identity (ops JSON + `pgObjectDdlVersion`
  + connection epoch) and is derived stale-proof, with commit
  re-verifying against live store state after every await; the apply
  acquires the shared per-connection `pgObjectDdlApplying` lock and the
  dialog's `isConnectionCurrent` fencing; success cleanup removes only
  the committed entry ids so edits queued mid-apply survive; the
  specialized index/FK panels queue typed ops only on PostgreSQL and
  keep the original generate-SQL fallback on other engines; and the
  description revalidation always includes the committing table's ref
  (covering `dropIndex`, which `objectDdlRefreshScope` cannot
  attribute). Referential-action vocabulary and index/FK op
  construction live once in `src/lib/structure-changes.ts`. Gates
  green: `pnpm format`/`lint`/`typecheck`, vitest, `check:ui-gates`,
  `check:slice-isolation`. The plan's Step 4 **interactive dev-app pass
  has not been run in the implementing session** (no display) — walk it
  in `pnpm tauri dev` against `lifecycle.orders`, then mark READY FOR
  REVIEW / DONE; the plan file carries the full execution deviation
  record.
- **Selected mock (014, 2026-08-29):** C — compact object viewer with
  Definition / Facts tabs and the DDL review rendered inline immediately
  above the object action row. Database-scoped list-only objects stay below
  schema-scoped groups in the navigator.
- **Selected (2026-08-29):** Plans 013, 014, and 015 are the `PAR-007`
  execution path, authored at `b45e294` and amended the same day after
  a pre-execution review (each plan carries a `Review correction
  record`). `PAR-007` is `XL`; the trio deliberately delivers its core
  and records the rest (Plan 013's Reconciliation section has the full
  deferral list with rationale).
  Plan 013 is a dark backend: a typed `PgObjectKind`/`PgObjectRef`
  model with overload-safe routine identity (replacing fourteen legacy
  `Vec<String>` name lists; event triggers/roles/tablespaces stay list-only
  entries, not kinds), a batched, capped, extension-member-filtered
  `load_pg_object_catalog` on the native PG pool with comments,
  `describe_pg_object` (owner, comment, tagged facts, reconstructed DDL
  per kind), `load_pg_drop_impact` as a bounded transitive closure over
  `pg_depend` with `pg_rewrite` resolution, and a backend-owned DDL
  workflow mirroring the Result Mutation shape: typed operations
  (schemas, table rename/drop/comment, the six column ops with tagged
  literal-vs-expression defaults and `USING`, PK/unique/FK/check and
  constraint drop, index create/drop including `CONCURRENTLY`,
  views/matviews from SQL bodies, sequences, enums) → pure server-side
  generation with every rendered statement re-verified by
  `classify_script` as exactly one statement → per-statement preview
  with destructive and transactional flags and atomic/standalone
  grouping → gated `apply_object_ddl` (regenerates from ops, never
  accepts a whole statement over IPC; the sixteenth gated write
  surface) on a detached connection that preserves the configured
  `statement_timeout` and adds `lock_timeout = 10s`, with a typed
  `PgObjectError` union carrying
  SQLSTATE/statement-index/applied-count/lock-timeout/invalid-index
  residue, plus a `lifecycle` fixture schema and ADR-0026. No migration
  needed.
  Plan 014 activates the schema-level half: navigator breadth with
  grouped, per-group-capped object sections (catalog replaces the PG
  explorer fetch behind a derive adapter with a generation guard
  against the connect/disconnect race; the dead sqlx-Any PG explorer
  arm is deleted in its truth pass), a persisted `object` workspace-tab
  kind with per-kind viewers that honour the Plan 010 disconnected-
  restore contract, palette reach into non-relation kinds (closing the
  Plan 009 deferral), one shared DDL review dialog (per-statement
  preview, reviewed gate, a new `ddl` safety-confirmation subject that
  shows statement summaries, drop-impact dialog with explicit CASCADE
  opt-in, refresh on partial failure), create dialogs, and the
  `PAR-017` truth pass. Matview edit is drop + create; matview refresh
  is the one declared legacy-path exception.
  Plan 015 switches the PostgreSQL structure editor onto the typed
  workflow (`StructureChange` union, PG forms produce ops directly,
  tagged defaults and `USING`, constraint/index affordances, index and
  cross-table FK panels queue typed ops; ClickHouse keeps `execute_ddl`).
  Deferred, staying in the register: databases, roles/grants/default
  privileges, RLS, triggers/event triggers, rules, partitions,
  tablespaces, extension install/remove, structured routine
  create/alter, the create-table designer, matview refresh policy, DDL
  cancellation, and the non-DDL legacy typed-error migration
  (`PAR-014`).

- **Completed:** Plans 001 and 002 delivered the PostgreSQL query-session
  foundation of `PAR-001` through commit `26268ca`. Plans 003 and 004
  delivered and activated PostgreSQL Table Browse for `PAR-002` through
  commit `ecefce8` on 2026-08-21.
- **Completed:** Plans 005 and 006 delivered the `PAR-003`
  staged mutation review core through `4e52c8a`.
  Batch paste, deep value editors, Quick Look, and configurable copy
  formats are the `PAR-003` register items deliberately left for a
  follow-on plan.
- **Completed:** Plans 007 and 008 delivered the `PAR-004`
  backend-enforced production safety policy and UI activation. Both plans
  are implemented and merged to `main`; Plan 008's implementation completed
  at `5409d66` and landed on `main` as `5991dbf`, with selected mock C. Plan
  007 landed the dark backend: per-connection environment / Safe Mode /
  relational read-only fields (migration 15), a fail-closed PostgreSQL
  statement classifier extracted from the Plan 005 lexer, one shared policy gate
  asserted at all sixteen write-capable surfaces (typed
  `policyBlocked`/`policyNeedsConfirmation` on the query-session and
  result-mutation actors; tagged refusal strings on legacy commands), a
  belt-and-braces `default_transaction_read_only` session GUC, and a
  persisted confirmed-override audit. Plan 008 activated it with form
  controls, environment badges and production identity across sidebar /
  header / tabs / banner / status bar, one shared confirmation dialog that
  re-sends with `confirmed: true`, and the audit view in Settings.
  Smart-commit environment defaults and user-picked connection colors are
  deliberately deferred (`PAR-001` follow-ons / `PAR-005`–`PAR-006`).
- **Selected mock (004, completed):** B — stacked command bar, inspection
  popover, keyset next as the primary pager.
- **Selected mock (006):** A — persistent right-side mutation review
  inspector that keeps the result grid primary while showing grouped changes
  and exact generated DML.
- **Selected (2026-08-24):** Plans 009 and 010 are the `PAR-005` execution
  path, authored at `9570f11`. Scope was reconciled against the UI-refresh
  series, which landed after the register's audit: UI refresh Phase 8
  already delivered the persistence half of `PAR-005` (SQLite `ui.v1.*`
  store, session restore of query/table tabs + hot-exit SQL + pins +
  active tab + expanded nodes, corrupt-blob fallback, connection GC), and
  the workbench consolidation (`8411dbf`) deleted the old sidebar and
  several overview tabs, so some register evidence paths are stale. Plan
  009 is a dark foundation: connection organization fields (migration 17:
  folder / favorite / color), a backend `duplicate_connection`
  (sequenced-with-rollback credential copy through the credential
  backend; the secret never crosses IPC; credential read-modify-writes
  now serialized by a mutex), a pure secret-free connection-URI
  build/parse library, the pure Open Anything index/ranking library
  (caps after ranking, capped frecency boost, typed targets,
  saved-query target-resolution fix), the session-blob caret field, and
  restore-orchestration tests. Plan 010 activates it: the palette
  becomes Open Anything (flat ranked results, kind badges), connections
  gain folders/favorites/colors/recency + Duplicate / Copy URI /
  Import-from-URI, restored tabs stop erroring against disconnected
  connections, and the caret round-trips. Both plans passed an
  independent adversarial review on 2026-08-24 and were amended for its
  findings (Plan 009's Review correction record has the list; Plan
  010's palette dispatch, reveal-schema plumbing, and duplicate flow
  were the substantive corrections). Deliberately deferred (recorded in
  Plan 009's Reconciliation section): SQL files/recent files, split
  editors, multiple windows, OS deep links, encrypted profile exchange,
  offline metadata/data search, keyvalue tab restore, per-tab
  browse-state persistence, and non-table object viewers (`PAR-007`).
- **Plan 011 delivery note (2026-08-24):** Steps 1–7 implemented in the
  working tree on top of `4facea1`: migration 18 + `PgTlsOptions` /
  `PgTlsMode` / `TlsFailureKind` types, `postgres/tls.rs` (resolver +
  four renderers, native ∪ file roots, `CaOnlyVerifier`, client auth,
  typed material errors, fixture PEMs), `postgres/connect_error.rs`
  (stage classifier over an extracted view), the four connect sites
  converged (`tls_prefer` is gone), tunnel server-name carry-over,
  keepalive applied on the dedicated driver (test inverted), `TlsFailed`
  arms on the three actor unions with TS mirrors + minimal formatter /
  decoder arms (see the plan's correction record), `diagnose_connection`
  with typed report + TS mirrors, a rebuilt `postgres-tls` fixture with
  a real CA and `dbunk_cert` client-cert role, ADR-0025 plus pointers in
  ADR-0013/0021. Gates: `just fmt/lint/test` (387 passed, 29 ignored),
  `pnpm format/lint/typecheck`, vitest (1078 passed), and the live
  suites against both fixtures (15 TLS/diagnosis tests plus the
  pre-existing query-session / table-browse / result-mutation / safety
  live suites) all green. Final review changed the legacy PostgreSQL
  `test_connection` implementation to a one-shot connect so unsaved values
  and ephemeral tunnel endpoints cannot reuse a cached socket; its wire
  contract is unchanged, and Plan 012 removes it.
- **Plan 012 delivery note (2026-08-28):** Steps 1–5 implemented in the
  working tree on top of `cf5ad07` (selected mock A). Step 2:
  `tlsControls: "postgres-modes" | "toggle"` replaces `showSslToggle`
  on the `host-auth` policy, `<TlsFields>` (mode select, conditional
  CA / client cert / client key / certificate host-name fields,
  production advisory), per-field TLS form state folded back into the
  blob by `tlsOptionsFromForm` (only the paths the mode reads are
  persisted; a legacy `ssl: false` record round-trips as `disable`),
  the keepalive control with its pool disclosure, and both-or-neither /
  keepalive-range validation. Step 3: `diagnoseConnection` store action
  (dev stub marks `tls` skipped, never passed), `<DiagnosisPanel>`
  replacing the footer banners, Test Connection in edit mode with
  backend credential hydration, and `test_connection` removed together
  with its now-orphaned one-shot probe helpers (recorded in the plan's
  correction record). Step 4: `formatTlsFailure` renders every
  `tlsFailed` arm as headline — detail (the backend prefix is stripped,
  not repeated), the table-session copy gained the arm, and `sslmode`
  round-trips through Copy URI / Import-from-URI (invalid values and
  certificate-path parameters stay disclosed as ignored). Step 5: the
  truth pass over ROADMAP / PENDING_TASKS / CONTEXT / ADR-0013 /
  ADR-0025 / `types.ts` / the register. Gates green: `pnpm format`,
  `lint`, `typecheck`, vitest (1140), `check:ui-gates`, and the `just`
  trio (407 passed, 34 ignored). Step 6 was executed on 2026-08-28 as
  an automated pass, because the implementing session cannot see the
  native window: with both fixtures up, the 28 live suites
  (`cargo test -- --ignored …`, 18 on 15432 + 10 on 15433) covered every
  mode with and without the fixture CA, the `dbunk_cert` client
  certificate, the wrong-server-name mismatch, the `prefer` plaintext
  downgrade, wrong password → authentication, missing database →
  database, and query-session / table-browse over TLS; the real form,
  served by the Vite dev server and driven through Chrome's DevTools
  protocol against the dev-mode store, confirmed the TLS block's
  per-mode fields and advisory, the keepalive control with its
  disclosure and range error, the diagnosis panel in the footer, Copy
  URI emitting `?sslmode=verify-full`, edit mode hydrating the TLS
  fields with a blank password and a working Test Connection, and
  Import-from-URI prefilling the mode; a Rust test pins the IPC payload
  shape the store sends. The mocks were deleted. What that pass could
  **not** exercise is the native app's IPC round-trip end to end (a real
  `diagnose_connection` from the form, and a `verify-full` query tab
  with a broken CA path showing the TLS message); the app is left
  running in `pnpm tauri dev` for the reviewer to click through before
  recording DONE.
- **Selected mock (012, 2026-08-28):** A — the "Transport security"
  card inline under the credentials (always visible, not behind
  Advanced), a checklist-style diagnosis panel replacing the footer
  banners (one row per stage in fixed order, passed detail in a muted
  column, elapsed right-aligned, the failing stage expanded in place
  with the kind headline and monospace message, skipped stages muted
  with their reason), the TLS summary line and the token-toned warning
  lines below the rows, and the keepalive control staying in Session
  defaults next to connect timeout. Plan 011 recorded `DONE: b134766`
  at the same time (the 2026-08-25 hardening commits were its review).
- **Selected mock (010):** A — folder section headers in the connections
  list, compact single-line palette rows with leading kind badges, and
  URI import as the first field of the connection form.
- **Selected (2026-08-24):** Plans 011 and 012 are the `PAR-006`
  execution path, authored at `4facea1`. Scope was reconciled against
  Plans 009/010, which already delivered the _organization_ half of
  `PAR-006` (folders, favorites, colors, recency, Duplicate, secret-free
  Copy URI, Import-from-URI); the pair covers the _security_ half for
  PostgreSQL only. Plan 011 is a dark backend: migration 18 adds a
  `tls_options` JSON blob (`mode` in libpq vocabulary `disable |
  prefer | require | verify-ca | verify-full`, CA / client cert /
  client key _paths_, optional `serverName`), legacy rows keep resolving
  through `ssl` unchanged; one `ResolvedTls` resolver with four
  renderers converges the dedicated tokio-postgres driver, the sqlx
  pool, the PG DSN, and `pg_dump`/`pg_restore` (today four independent
  `prefer|disable` decisions and an accept-all verifier); real chain
  and hostname verification with native ∪ user-CA roots; client
  certificates; the SSH tunnel carries the original hostname so
  `verify-full` survives tunnelling (`host` + `hostaddr` on the
  dedicated driver, `PGHOSTADDR` on libpq; the sqlx pool verifies CA
  only over a tunnel and discloses it); `keepalive_seconds` applied on
  the dedicated driver; typed `tlsFailed` arms on the three actor
  unions instead of `connectionLost`; a staged `diagnose_connection`
  command (tunnel → DNS → TCP → TLS → authentication → database, each
  passed / failed-with-kind / skipped, plus real encryption state from
  `pg_stat_ssl`); a live fixture with a real CA, CA-signed server cert,
  and a client-cert role; and ADR-0025. Plan 012 activates it: a TLS
  mode select with conditional certificate-path fields and server-name
  override for PostgreSQL (MySQL keeps its toggle), a keepalive control
  with the pool-path disclosure, Test Connection as a per-stage
  checklist available in edit mode too (credential hydrated backend
  side), `sslmode` round-tripping through Copy URI / Import-from-URI,
  `tlsFailed` rendered where `connectionLost` is today, removal of the
  old `test_connection` command, and the `PAR-017` truth pass over
  ROADMAP / ADR-0013 / CONTEXT / PENDING_TASKS / the stale keepalive
  comments. Deliberately deferred (recorded in Plan 011's
  Reconciliation section): MySQL TLS modes, connection tags, encrypted
  client keys, certificate contents in the credential store / secure
  bundles, IAM and external-secret adapters, keepalive on the sqlx pool.
- **Plan 010 delivery note (2026-08-24, DONE: `4facea1`):** Steps 2–6
  implemented on top of Plan 009 (`DONE: f66abaa`) and merged to `main`
  as `e4a2fb8` + `4facea1`: flat-ranked Open
  Anything palette with controlled selection and disclosed truncation;
  folder/favorite/color organization (one Plan 009 amendment landed
  alongside: `update_connection_organization`, a credential-free
  column-only backend update, because `save_connection` treats an empty
  password as credential deletion); Duplicate / Copy URI /
  Import-from-URI; reconnect-gated table tabs with a connect-to-load
  shell; caret capture/restore with clamping. All automated gates green
  (frontend suite, Rust suite, `check:ui-gates`, `check:slice-isolation`,
  `check:redis-commands`). Step 6's **interactive dev-app pass has not
  been run in the implementing session** (no display available) — the
  reviewer/operator should walk the Step 6 checklist in `pnpm tauri dev`
  before recording DONE.
- **Selected mock (008):** C — restrained production banner with persistent
  target identity in the status bar and confirmation-forward safety flows.
- **Delivered boundary:** PostgreSQL table tabs now use typed server-side
  filters and sorting, bounded pagination/count behavior, cancellation,
  stale-response rejection, query inspection, durable grid preferences, and
  backend-authoritative row identity. Query-result mutation remains in
  `PAR-003`.

## Dependency order after Plans 001 through 010

Items 7 and later are candidate plans, not authored plans. Their identifiers
refer to the gap register rather than files in this directory.

1. Plans 001 and 002 delivered and activated the PostgreSQL query-session
   foundation. Non-blocking `PAR-001` follow-ons remain in the gap register.
2. Plans 003 and 004 delivered and activated `PAR-002` server-backed table
   browsing, reusing the query-session connect-spec, TLS, cancellation, and
   bounded-result semantics.
3. Plans 005 and 006 delivered the `PAR-003` execution path: the dark
   PostgreSQL Result Mutation backend, then one staged Mutation Draft model
   shared by browse-mode table tabs and query results with DML review
   before commit. Deep editors, Quick Look, batch paste, and copy formats
   remain follow-on register scope.
4. Plans 007 and 008 delivered the `PAR-004` execution path: a dark
   backend safety policy (environment classification, Safe Mode levels,
   enforced relational read-only, fail-closed statement classification,
   one gate at every write surface, override audit), then UI activation
   (form fields, production identity, confirmation flows). They build on
   `PAR-001` transaction state and the `PAR-003` mutation preview as
   planned.
5. Plans 009 and 010 delivered the `PAR-005` execution path: durable
   descriptors (never live handles), Open Anything, and connection
   organization. Plan 010 landed on `main` as `e4a2fb8` with review
   amendments in `4facea1`.
6. Plans 011 and 012 are the authored `PAR-006` execution path: the
   PostgreSQL connection-security backend (TLS verification modes,
   client certificates, tunnel-aware hostname verification, applied
   keepalive, staged diagnosis) as a dark foundation, then the form and
   diagnosis-panel activation. Plans 013, 014, and 015 are the authored
   `PAR-007` execution path that follows them: the object catalog and
   DDL workflow backend as a dark foundation, then the object explorer,
   viewers, and schema-level lifecycle activation, then the structure
   editor's switchover onto the same typed workflow.
7. Professional PostgreSQL features (`PAR-007` through `PAR-011`) should follow
   once execution, editing, safety, restoration, and transport security are
   trustworthy.
8. Cross-engine breadth (`PAR-014`) should reuse the PostgreSQL contracts only
   after those contracts have proven stable.

## Planning rules

- PostgreSQL is the reference engine per `docs/adr/0001-postgres-first-engine-coverage.md`.
- Correctness, bounded resource use, cleanup under failure, and predictable
  reconnect behavior take priority over feature breadth.
- A plan must be self-contained and stamped with the commit it was written
  against.
- Plans may not silently broaden from PostgreSQL into every relational engine.
- Every implementation must pass `pnpm format`, `pnpm lint`, and
  `pnpm typecheck`. Rust changes additionally require `just fmt`, `just lint`,
  and `just test`.
- Publishing, production changes, commits, pushes, and PR creation require
  separate authorization.

## Findings considered and rejected

None. Features outside the recommended PostgreSQL-first boundary are recorded
as deferred decisions in the gap register rather than rejected.
