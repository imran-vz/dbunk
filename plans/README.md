# dbunk parity plans

Generated from the DBeaver and TablePlus parity audit on 2026-08-18 at commit
`24432fb`.

This directory is the execution handoff for closing the documented parity gaps.
The canonical inventory is [parity-gap-register.md](./parity-gap-register.md).
Plans should be executed in dependency order unless a plan explicitly says
otherwise. Executors must read a plan completely, honor its STOP conditions,
and update its status here when work finishes.

## Execution order and status

| Plan                                             | Title                                       | Priority | Effort | Depends on | Status                                         |
| ------------------------------------------------ | ------------------------------------------- | -------: | -----: | ---------- | ---------------------------------------------- |
| [001](./001-query-session-foundation.md)         | PostgreSQL Query Session backend foundation |       P0 |      L | None       | DONE: 657553d                                  |
| [002](./002-query-session-editor-integration.md) | PostgreSQL Query Session editor integration |       P0 |      L | 001        | DONE: 26268ca (selected mock: B)               |
| [003](./003-table-browse-backend.md)             | PostgreSQL Table Browse backend             |       P0 |      L | 001, 002   | DONE: 202f756                                  |
| [004](./004-table-browse-grid-integration.md)    | Server-backed browsing in table tabs        |       P0 |      L | 003        | DONE: ecefce8 (selected mock: B)               |
| [005](./005-result-mutation-backend.md)          | PostgreSQL Result Mutation backend          |       P0 |      L | 003, 004   | DONE: d98f8a1                                  |
| [006](./006-staged-mutation-review-integration.md) | Staged mutation review in table and query results | P0 |      L | 005        | DONE: 4e52c8a (selected mock: A)               |
| [007](./007-safety-policy-backend.md)            | Backend-enforced production safety policy   |       P0 |      L | 005, 006   | DONE: bd9f7ef                                  |
| [008](./008-safety-policy-activation.md)         | Safety policy activation and production identity | P0 |      L | 007        | DONE: 5409d66 (selected mock: C)               |
| [009](./009-workspace-navigation-foundation.md)  | Workspace navigation foundation (dark)      |       P0 |      L | 001–008    | DONE: f66abaa                                  |
| [010](./010-open-anything-activation.md)         | Open Anything activation and connection organization | P0 | L | 009        | DONE: 4facea1 (selected mock: A)               |
| [011](./011-connection-security-backend.md)      | PostgreSQL connection security backend (dark) |       P1 |      L | 001–010    | READY FOR REVIEW                               |
| [012](./012-connection-security-activation.md)   | TLS controls, staged connection diagnosis, and truth pass | P1 | L | 011        | TODO                                           |

Status values: `TODO`, `IN PROGRESS: through Step N`, `READY FOR REVIEW`,
`DONE: <completion SHA>`, `BLOCKED: <reason>`, or `REJECTED: <reason>`.

Executors update their own status row after each completed step and mark
`READY FOR REVIEW` after all gates. The reviewer or operator records
`DONE: <completion SHA>` after the work is committed. This makes resume state
useful without authorizing commits implicitly.

## Current selection

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
  asserted at all fifteen write-capable surfaces (typed
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
- **Selected mock (010):** A — folder section headers in the connections
  list, compact single-line palette rows with leading kind badges, and
  URI import as the first field of the connection form.
- **Selected (2026-08-24):** Plans 011 and 012 are the `PAR-006`
  execution path, authored at `4facea1`. Scope was reconciled against
  Plans 009/010, which already delivered the *organization* half of
  `PAR-006` (folders, favorites, colors, recency, Duplicate, secret-free
  Copy URI, Import-from-URI); the pair covers the *security* half for
  PostgreSQL only. Plan 011 is a dark backend: migration 18 adds a
  `tls_options` JSON blob (`mode` in libpq vocabulary `disable |
  prefer | require | verify-ca | verify-full`, CA / client cert /
  client key *paths*, optional `serverName`), legacy rows keep resolving
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
   diagnosis-panel activation. The next candidate after them is
   `PAR-007` object lifecycle.
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
