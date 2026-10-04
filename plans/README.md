# dbunk parity plans

Generated from the DBeaver and TablePlus parity audit on 2026-08-18 at commit
`24432fb`. The canonical gap inventory is
[parity-gap-register.md](./parity-gap-register.md).

Executors must read a plan completely, honor its STOP conditions, and update
its status here when work finishes. Completed plan bodies are deleted once
recorded `DONE` — the completion SHA below is the pointer into git history.

## Desktop migration proposal

Separate from the parity audit. This is a decision document, not an executable
plan: it has no plan number, effort or steps, and its status is outside the
status values below. Each stage becomes a numbered plan in the table that
follows before it is executed.

| Proposal | Scope | Status |
| --- | --- | --- |
| [GPUI migration](./gpui-migration.html) | macOS first; current feature parity before replacing Tauri | APPROVED TO START by Imran on 2026-10-02. Stage 00 decided the same day: reason is architecture with no performance gain required, margins 30% and 10%, licence GPL-3.0-or-later (path Z, Zed's editor), feature rule backend-first with baseline `102568b`. Stage 01 is Plan 024; the first slice of stage 02 is Plan 025. Both are READY FOR REVIEW pending completion commits. **Stage 01 gate CLOSED: PASS / continue, 2026-10-02**, under Imran's request to finish and close it. All four criteria are met: text geometry and editor/results/cell-editor keyboard focus are verified; remaining full-application work is costed in the [gate review](./evidence/024/stage01-gate.md). Imran confirmed VoiceOver complete on 2026-10-02. On 2026-10-03 Imran deferred VoiceOver from the blocking verification gate; keyboard/AX and real IME remain required. No daily-driver cutover is approved. Stage 03 is [Plan 026](https://github.com/imran-vz/dbunk/blob/3f987c96640d6738b48ae1113ceac3ebbdc8563f/plans/026-native-postgres-workflow.md), DONE at `3f987c9` after evidence review on 2026-10-02, with A + B + C and a user-facing layout switcher. Stage 04 is drafted as [Plan 027](./027-native-workspace-shell.md), with A, Persistent Navigator, selected by Imran on 2026-10-02; implementation in progress; [three static options](./mocks/native-workspace/index.html). Stage 05 is [Plan 028](./028-native-postgres-data-workflow.md), with backend and selected native UI implementation in progress; acceptance remains dependent on Plan 027 gates; table-review A selected. The PostgreSQL stage 06 tools are [Plan 029](./029-native-postgres-tools.md); stage 07 parity/profile/package acceptance is [Plan 030](./030-native-postgres-parity-acceptance.md). Tool layout A, Tool tabs, is selected. Imran clarified on 2026-10-02 that the target is **complete PostgreSQL parity with Tauri**, not only the fixture query workflow; see the [scope and design review](./mocks/native-postgres-parity/index.html) and [native parity inventory](./native-postgres-parity-inventory.md). |

The stage 01 closure activates the stage 00 backend-first rule: React receives
correctness, safety and data-loss fixes only; new capability lands dark in the
core and its UI is built natively. The parity baseline remains `102568b`.
Plan 024 stays `READY FOR REVIEW` until a separately authorized completion
commit supplies its SHA. The gate itself is closed; Plan 025 remains a separate
review.

## Execution order and status

| Plan                                                  | Title                                                            | Priority | Effort | Depends on | Status                                         |
| ----------------------------------------------------- | ---------------------------------------------------------------- | -------: | -----: | ---------- | ---------------------------------------------- |
| 001                                                   | PostgreSQL Query Session backend foundation                      |       P0 |      L | None       | DONE: 657553d                                  |
| 002                                                   | PostgreSQL Query Session editor integration                      |       P0 |      L | 001        | DONE: 26268ca (selected mock: B)               |
| 003                                                   | PostgreSQL Table Browse backend                                  |       P0 |      L | 001, 002   | DONE: 202f756                                  |
| 004                                                   | Server-backed browsing in table tabs                             |       P0 |      L | 003        | DONE: ecefce8 (selected mock: B)               |
| 005                                                   | PostgreSQL Result Mutation backend                               |       P0 |      L | 003, 004   | DONE: d98f8a1                                  |
| 006                                                   | Staged mutation review in table and query results                |       P0 |      L | 005        | DONE: 4e52c8a (selected mock: A)               |
| 007                                                   | Backend-enforced production safety policy                        |       P0 |      L | 005, 006   | DONE: bd9f7ef                                  |
| 008                                                   | Safety policy activation and production identity                 |       P0 |      L | 007        | DONE: 5409d66 (selected mock: C)               |
| 009                                                   | Workspace navigation foundation (dark)                           |       P0 |      L | 001–008    | DONE: f66abaa                                  |
| 010                                                   | Open Anything activation and connection organization             |       P0 |      L | 009        | DONE: 4facea1 (selected mock: A)               |
| 011                                                   | PostgreSQL connection security backend (dark)                    |       P1 |      L | 001–010    | DONE: b134766                                  |
| 012                                                   | TLS controls, staged connection diagnosis, and truth pass        |       P1 |      L | 011        | DONE: b45e294 (selected mock: A)               |
| 013                                                   | PostgreSQL object catalog and DDL workflow backend (dark)        |       P1 |      L | 001–012    | DONE: 4833a42                                  |
| 014                                                   | Object explorer, viewers, and lifecycle activation               |       P1 |      L | 013        | DONE: 2e843a6 (selected mock: C)               |
| 015                                                   | PostgreSQL structure editor switchover to the typed DDL workflow |       P1 |      M | 013, 014   | DONE: 84112dc                                  |
| 016                                                   | PostgreSQL table designer, routine, trigger, policy, and privilege DDL backend (dark) |       P1 |      L | 013–015    | DONE: 6b573f1                                  |
| 017                                                   | Table designer, routine editor, and table security activation                  |       P1 |      L | 016        | DONE: 25d36f1 (selected mock: A)               |
| 018 | File-backed PostgreSQL backup and restore foundation (dark)                    |       P1 |      L | 017        | DONE: de3272b                     |
| 019 | PostgreSQL backup and restore activation | P1 | L | 018 | DONE: ab33968 (selected mocks: A + C) |
| 020 | Bounded PostgreSQL CSV import and export | P1 | L | 018, 019 | DONE: 7745946 (selected mock: A) |
| 021 | Bounded PostgreSQL schema comparison foundation (dark) | P1 | L | 013–017, 020 | DONE: 9312b41 |
| 022 | PostgreSQL schema comparison activation | P1 | L | 021 | DONE: db2dae2 (selected mock: A) |
| [023](./023-query-session-bound-parameters-and-row-limit.md) | Bound parameters and row-limited reads in PostgreSQL Query Sessions (dark) | P0 | L | 001, 002, 007 | READY FOR REVIEW |
| [024](./024-gpui-baseline-and-spike.md) | Tauri baseline, external measurement harness and GPUI spike (migration stage 01) | P1 | L | Migration stage 00 | READY FOR REVIEW: stage 01 gate CLOSED, PASS / continue on 2026-10-02; all four criteria met ([review](./evidence/024/stage01-gate.md)); completion SHA pending an authorized commit |
| [025](./025-shared-core-query-session-extraction.md) | Host-neutral core seam and Query Session service extraction (migration stage 02, first slice) | P1 | L | Migration stage 00 | READY FOR REVIEW |
| 026 | First working native PostgreSQL workflow (migration stage 03) | P1 | L | Stage 01 gate closed; 025 implementation | DONE: 3f987c9 (selected mocks: A + B + C; [completion review](./evidence/026/completion-review.md)) |
| [027](./027-native-workspace-shell.md) | Native PostgreSQL workspace shell (migration stage 04) | P1 | XL | 026; stage 01 gate closed | IN PROGRESS: Step 1 typed connection/draft services, native admission and durable credential recovery ([progress](./evidence/027/step01-services-progress.md)); injected/headless and scoped OS Keychain CLI checks pass; native workspace/forms/writer implemented; release TLS/query/reopen, multi-session runtime and performance captures pass; recovery and packaged credentials pass; scoped real Pinyin SQL/form/cell checks pass; remaining actual-window acceptance pending; VoiceOver deferred by Imran on 2026-10-03 ([workspace progress](./evidence/027/step02-workspace-progress.md)); selected mock A, Persistent Navigator |
| [028](./028-native-postgres-data-workflow.md) | Native PostgreSQL transactions, Table Browse and mutations (migration stage 05) | P1 | XL | 027 | IN PROGRESS: typed data facade, owned task cleanup and confirmation services implemented; focused live data/transaction checks and corrected query harness rerun pass ([progress](./evidence/028/backend-progress.md)); selected mock A, Bottom review; native query controls, table paging, staged writes, exact-save apply barrier and v2 recovery implemented in source; column preferences, scoped query/table/review/apply/reopen and real Pinyin checks pass ([window evidence](./evidence/028/table-window-verification-20261003.md)); typed browse/presets, value inspection, specialized literal/array-element editors and virtual keys now have [source checks](./evidence/028/browse-inspector-source-checks/README.md); composite FK navigation also has [source/live checks](./evidence/029/metadata-fk-source-checks/README.md); new-control window acceptance and further data features remain pending; VoiceOver is deferred |
| [029](./029-native-postgres-tools.md) | Native PostgreSQL catalog, SQL tools, jobs and administration (migration stage 06 subset) | P1 | XL | 027, 028 | IN PROGRESS: 2026-10-03 increment ([evidence](./evidence/029/navigator-workspace-increments-20261003/README.md)): schema/object Navigator, Open Anything, console Dock, SQL find, Window menu/geometry, release log, health tick, overview recents, relationship detail, sequences, existing-schema alter (workspace v14), widened query-result editing context and WKT preview have source/focused/owned-live checks; no window, keyboard/AX or IME acceptance (CUA unavailable, AX untrusted in this harness). history/saved-query Tool tabs and execution capture implemented; bounded copy formats integrated; frozen source checks pass; EXPLAIN drafts/tree and owned catalog reads have source/live checks; Objects Tool tab and [all twelve baseline description kinds](./evidence/029/metadata-fk-source-checks/README.md) implemented with scoped live probes passed; retained file exports and downstream drop impact now have [focused source/live evidence](./evidence/029/export-impact-source-checks/README.md); [whole-table exports and saved configurations](./evidence/029/whole-table-export-source-checks/README.md) now have exact scoped file/window evidence, with corrected AX/focus/status and recipe reopen checks passed; the broader duplicate button keyboard and dynamic-toggle focus corrections pass scoped window checks; existing-table comment/rename now has native recovery/review source under verification; full tools acceptance and remaining object-service lifecycle work remain pending; selected tools mock A, Tool tabs; other per-family gates remain open |
| [030](./030-native-postgres-parity-acceptance.md) | Native PostgreSQL parity, profile compatibility and package acceptance (stage 07 subset) | P1 | XL | 027–029 | IN PROGRESS: 2026-10-03 increment ([evidence](./evidence/029/navigator-workspace-increments-20261003/README.md)): schema/object Navigator, Open Anything, console Dock, SQL find, Window menu/geometry, release log, health tick, overview recents, relationship detail, sequences, existing-schema alter (workspace v14), widened query-result editing context and WKT preview have source/focused/owned-live checks; no window, keyboard/AX or IME acceptance (CUA unavailable, AX untrusted in this harness). [action-level capability ledger](./evidence/030/capability-ledger.md) prepared; explicit general native profiles have source/live checks; disposable migration and complete package/window acceptance remain pending; VoiceOver deferred |
| [031](./031-native-redesign-and-engines.md) | Native redesign and multi-engine workspace (after the hard migration, ADR-0033) | P1 | XL | 027–030 | IN PROGRESS: through Step 3. `2554eb4` made the hard migration (Tauri/React app removed, backend moved to `backend/`) and added the compact shell (sidebar with project/environment switcher, tab bar, collapsible status bar, environment frame + tint) and the connection `project` field. `a39e7ae` restyled the documents (tool launchers in the tab bar Tools menu) and added native connection records and forms for MySQL, SQLite, ClickHouse and Redis; those engines still had no sessions, object trees or documents. `5215a39` made the redesigned workspace the default launch with a persistent profile. `aeabaa1` connects a connection on selection, with connect/disconnect, connecting and failed states on the sidebar row and the failure in the status bar. `ecbad98` applied the whole-app theme and interaction pass ([`DESIGN.md`](../DESIGN.md)): kit-based forms with inline validation, restored three-option credential storage with full-window setup/unlock/recovery gates, tab close fixes, overlay input blocking and Reduce-motion-aware motion. Step 4 MySQL (branch `plan-031-step4-mysql`): selecting a MySQL connection opens one owned backend session (dedicated sqlx connection, SSH route held by the worker, no automatic retry; disconnect, connection/credential edits and shutdown retire it); the sidebar tree lists databases → tables, views, routines, events, triggers (lazy per database, 5000 names per kind); tabs for query (safety policy and read-only enforced, ⌘↵, Stop = `KILL QUERY`), table data (200-row pages, primary-key order) and structure/definition (`SHOW CREATE`); results bounded to 1000 rows/16 MiB/64 KiB per cell. Verified by unit tests and a live check against a disposable `mysql:8.4`; MySQL tabs are not persisted. SQLite, ClickHouse and Redis remain for Step 4. Source checks only; no window, keyboard/AX or IME acceptance; [mock](./mocks/native-redesign/index.html) |

Status values: `TODO`, `IN PROGRESS: through Step N`, `READY FOR REVIEW`,
`DONE: <completion SHA>`, `BLOCKED: <reason>`, or `REJECTED: <reason>`.

Executors update their own status row after each completed step and mark
`READY FOR REVIEW` after all gates. The reviewer or operator records
`DONE: <completion SHA>` after the work is committed.

Plan 026 is DONE at `3f987c9`, reviewed on 2026-10-02 under Imran's request.
The [completion review](./evidence/026/completion-review.md) confirms that all
220 recorded source/lockfile hashes match the committed implementation, the
27 actual-window race runs and release AX workflow pass with clean teardown,
and the guarded performance captures reproduce the reported results. The
separate human VoiceOver correction recheck passed. Required repository,
backend and native check logs were reviewed; `pnpm format`, `pnpm lint` and
`pnpm typecheck` also pass freshly during closure. Runtime tests were not rerun.
The completed plan body is retired; its [historical execution record](https://github.com/imran-vz/dbunk/blob/3f987c96640d6738b48ae1113ceac3ebbdc8563f/plans/026-native-postgres-workflow.md)
and [verification evidence](./evidence/026/implementation-review.md) remain.
Plans 024 and 025 retain their separate review status. This completes the
isolated macOS PostgreSQL slice, without authorizing daily-driver cutover.

**Plan 023 is READY FOR REVIEW** (PAR-001 follow-ons, chosen by Imran on
2026-10-01, authored against `49c50e8`, now `677a7e8` on `main` with an
identical tree). All seven steps are complete and uncommitted. The backend is
dark: no frontend caller sends `parameters` or `rowLimit`.

- Delivered: named-parameter scan and `$k` rewrite with a position map, the
  shape planner, the cursor read with a row limit, the bound command, the
  `cancelled` outcome, the credit-loop repair, `describe_query_parameters`,
  ADR-0031.
- Evidence, 2026-10-01, disposable fixtures, macOS only: `just fmt`, `just
  lint`, `just test` (651 passed, 79 ignored), `pnpm format`, `pnpm lint`,
  `pnpm typecheck`, `pnpm test` (1,488 passed); 29 live tests on PostgreSQL
  16.14 and the TLS fixture; 19 of 19 fixture-port tests on PostgreSQL
  17.10. The Script shape's event sequence is unchanged apart from two new
  null fields.
- Two decisions made by Imran during validation: the `FETCH` is read eagerly
  so frontend credit never holds the wrapper transaction open, and the
  server's type-inference limit (`:x IS NULL` needs a cast) is a known limit
  for the activation plan, not a STOP.
- Not run: an SSH-tunnel route (no fixture), PostgreSQL 18, platforms other
  than macOS, the `ackTimeout` expiry end to end.

The [plan's execution record](./023-query-session-bound-parameters-and-row-limit.md)
lists every departure from the plan as written, and ADR-0031 holds the
measurements. Other candidates are in
[parity-gap-register.md](./parity-gap-register.md).

Plan 022 is DONE at `db2dae2`, confirmed by Imran on 2026-10-01. It brought Plan
021's read-only comparison into the workbench as the Object inspector (mock A,
selected 2026-09-14). The native/WebView fixture and memory gate ran on
2026-10-01 against owned PostgreSQL 16.15, 16.14 and 17.11 fixtures and an SSH
bastion: 117 scripted checks pass on a production frontend bundle in the real
desktop WebView. The completed plan body is retired; the
[historical execution record](https://github.com/imran-vz/dbunk/blob/db2dae24c504248d62f15f772bf82e4c8d1f5ff2/plans/022-postgres-schema-comparison-activation.md) retains the scenarios, measurements, the
one message repaired and the limits of that evidence (one platform, a debug
native build, no screen capture or physical key input). This bookkeeping
update records that commit and does not claim new runtime tests.
[Published brief and mocks](https://dbunk-schema-compare-plan-022.imran-vz.chatgpt.site) ·
[Local artifact](./mocks/schema-compare/index.html).

Plan 021 is DONE at `9312b41`. Its completion record and reviewed fixes were
committed on 2026-09-14. The completed plan body is retired; the
[historical execution record](https://github.com/imran-vz/dbunk/blob/9312b41ab2d2c92f48b54d2b3229332bf74641a2/plans/021-bounded-postgres-schema-comparison.md) retains the checks,
fixture matrix, performance measurements and remaining validation limits.
This bookkeeping update records that commit and does not claim new runtime tests.
The delivered backend covers ordinary-table definitions on PostgreSQL 16,
with bounded capture, structural differences, typed jobs, cancellation and
explicit coverage. UI activation and WebView memory validation were Plan 022;
wider object coverage, migration SQL and data comparison remain later slices.

Plan 020 is DONE at `7745946`, confirmed by Imran on 2026-09-05.
Its historical execution record retains the automated/live results and native
validation limitations known at completion.

## Planning rules

- PostgreSQL is the reference engine per `docs/adr/0001-postgres-first-engine-coverage.md`.
- Correctness, bounded resource use, cleanup under failure, and predictable
  reconnect behavior take priority over feature breadth.
- A plan must be self-contained and stamped with the commit it was written
  against.
- Plans may not silently broaden from PostgreSQL into every relational engine.
- Backend changes must pass `just fmt`, `just lint`, and `just test`; native
  changes must pass `just fmt-native`, `just lint-native`, and
  `just test-native`. The pnpm checks were removed with the React app
  (ADR-0033).
- Publishing, production changes, commits, pushes, and PR creation require
  separate authorization.

Latest native data-workflow increment: [duplicate/bulk rows, snippets and snapshot ownership](./evidence/028/duplicate-bulk-source-checks/README.md).
Plans 027–030 remain IN PROGRESS; the new controls still require actual-window acceptance.

Latest query-edit increment: [executed source, version 3 recovery and guarded query UPDATEs](./evidence/028/query-mutation-source-checks/README.md).
This is a restricted implementation, not complete query-result parity. Unqualified
sources, context-dependent types and non-ASCII row guards remain unsupported;
new native-window acceptance is pending. Plans 027–030 remain IN PROGRESS.

Latest tools increment: [read-only Administration Tool tab](./evidence/029/admin-source-checks/README.md).
Owned sessions/locks/pending-transaction reads, required source checks and the
separate package pass. CUA window discovery failed for that exact package. Session control, maintenance and actual-window acceptance
remain open. Plans 027–030 remain IN PROGRESS.

Latest SQL tools increment: [native completion](./evidence/029/completion-source-checks/README.md).
Bounded suggestions and exact owned column metadata are integrated. Focused
source/live probes, debug/release checks and the isolated package pass. Actual-window acceptance remains open.
A [conservative format command](./evidence/029/formatter-source-checks/README.md) now preserves token spelling and applies bounded whitespace edits with separate undo boundaries. Required/native debug/release checks and the separate package pass. Keyword normalization and full formatter/window acceptance remain open. Plans 027–030 remain IN PROGRESS.

Latest Plan 030 increment: [explicit general PostgreSQL profiles](./evidence/030/general-profile-source-checks/README.md). Separate marker/SQLite identity, endpoint authority, explicit create/open launch modes and owned verification receipts are implemented. Required checks, owned service probe and a separate frozen package pass; window discovery failed, so actual-window acceptance remains pending. Fixture guards remain in place; profile import, production identity and full package/window acceptance remain open.

Latest Plan 029 connection increment: [URI import and secret-free copy](./evidence/029/connection-uri-source-checks/README.md). Required source checks, both pinned URL-parser corpora and the separate frozen package pass; actual-window acceptance remains pending. General-form TLS now defaults to Prefer; fixture defaults remain unchanged. Full staged diagnosis, SSH and managed PostgreSQL remain open.

Latest Plan 029 inspection increment: [server facts, settings and extensions](./evidence/029/server-details-source-checks/README.md). Scoped live and frozen repository/native checks pass; the separate frozen package passes, but CUA discovery failed before interaction and actual-window acceptance remains pending. Readings disclose reader-session scope and inspection overrides. Statistics, audit, Settings mirror/Edit and administration write actions remain open.

Latest Plan 029 DDL increment: [atomic Create Schema with optional comment](./evidence/029/schema-create-source-checks/README.md). The dedicated write fence, exact-revision recovery and native bottom review are implemented; focused/frozen checks, owned live verification and packaging pass; CUA discovery failed and window acceptance remains pending. This does not complete object lifecycle or designer parity.

Latest local administration increment: [retained safety overrides](./evidence/029/safety-audit-source-checks/README.md), with bounded profile-local pages and disconnected inspection. Focused/frozen checks and the separate package pass; actual-window acceptance remains pending. PostgreSQL control and maintenance actions remain open.

Current file-job increment: [native backup/restore progress](./evidence/029/tool-jobs-progress.md). Stable attempt registration, owned source/process cleanup, restore data fences, transient Tool-tab setup and the shared observer are under implementation and verification. Final source/package, real-tool and window evidence remain pending; this does not close any plan.

Backup/Restore now has [frozen source, package and owned real-client evidence](./evidence/029/tool-jobs-source-checks/README.md). Plain/custom backup and restore pass the scoped fixture probe. Native window discovery failed before interaction, so keyboard/AX, real IME and complete job acceptance remain pending. CSV transfer ownership work follows; full PostgreSQL parity remains IN PROGRESS.

CSV transfer now has [frozen source, package and owned import/export evidence](./evidence/029/csv-transfer-source-checks/README.md). Indexed mapping, dialect/NULL controls, exact review, app-owned observation, joined cancellation and import-source invalidation are implemented. The real probe passed exact values, source replacement refusal and late-error rollback. Required/native checks and the separate package pass; CUA discovery failed before interaction, so keyboard/AX, real IME and full transfer acceptance remain pending. Workspace format 6 persists only CSV-tab identity/binding. Full PostgreSQL parity remains IN PROGRESS.

Current comparison increment: [native PostgreSQL 16 comparison progress](./evidence/029/schema-comparison-progress.md). App-owned jobs, exact request reconciliation, dedicated result-reader generations, bounded typed pages and UTF-8 value chunks are implemented. Focused checks and the owned PG16 facade probe pass; the corrected backend and refined native checks pass, and the separate package matches 435 source hashes. CUA window discovery failed; actual-window acceptance remains pending. Cached schema suggestions and presentation/copy/keyboard parity refinements are now source-implemented. This does not close any plan.

Current formatter increment: [keyword normalization progress](./evidence/029/keyword-format-progress.md). Known PostgreSQL keywords and phrases now normalize through Format SQL; exact bound parameters and protected text remain unchanged. Focused tests and the pinned baseline vocabulary probe pass. Required checks and the separate package pass against 439 source hashes. Actual-window acceptance remains pending.

Current grid sizing increment: [content-derived widths and auto-fit](./evidence/028/auto-fit-progress.md). Query/table defaults sample retained rows; explicit auto-fit preserves per-result query geometry and the table preference acknowledgement barrier. Required/native checks and separate packaging pass against 440 source hashes. Actual-window acceptance remains pending.

Latest unlocked-desktop attempt: [scoped auto-fit window and reopen evidence](./evidence/028/auto-fit-window-20261003/README.md). Formatting/undo, grid sizing, persisted table geometry and selected library/administration workflows were exercised; both normal quits returned fixture activity to zero. Full keyboard/AX and newer-control IME acceptance remain open. Restoring both library tabs exposed a delivery-budget refusal; the [deferred library activation fix](./evidence/029/library-activation-source-checks/README.md) passes required/native checks and the same-profile window retry. Plans 027–030 remain IN PROGRESS.

Native administration cancel/terminate now has an immutable captured-target
review, stored-policy confirmation, exact-save dispatch barrier and version-8
read-only recovery. Its owned stage03 backend probe passed with activity 0 → 0;
[source and verification scope](./evidence/029/admin-control-source-checks/README.md)
records the PostgreSQL signal-identity race. Required/native/backend checks and
the isolated package pass; [scoped window checks](./evidence/029/admin-control-window-20261003/README.md)
pass cancel, policy confirmation/termination, staged reopen and explicitly
injected unknown recovery, with normal quit and activity 0 → 0.
Full parity remains IN PROGRESS; VoiceOver stays deferred.

Current grid navigation increment: [Go to retained row and whole-row rectangles](./evidence/028/grid-navigation-progress.md). Required/native checks and packaging pass; scoped window navigation/copy passes with clean teardown. Missing AX range/error labels were corrected and verified in the [maintenance package](./evidence/029/maintenance-reopen-20261003/README.md). Independent checkbox row selection and complete grid parity remain open.

Current maintenance increment: [owned PostgreSQL maintenance and refresh](./evidence/029/maintenance-progress.md). Required/backend/native checks, owned live probe, packaging and scoped window/recovery/cancellation checks pass; the second launcher gate and guarded helper cleanup are recorded explicitly. Receipt/disclosure corrections also pass separate checks and a clean window recheck; full acceptance remains pending. Residual concurrent-DDL targeting limits are explicit. Full PostgreSQL parity remains IN PROGRESS.

Current grid pinning increment: [ordered left pins](./evidence/028/pinning-progress.md).
Required/native debug and release checks, independent review and the isolated
package pass. Scoped AX projection/copy, exact cell editor and table-persistence
reopen checks pass with clean teardown. Visual scrolling/resize and broader IME
acceptance remain open; missing-content capture also reproduced on the preceding
package. Full PostgreSQL parity remains IN PROGRESS.

Current keyboard increment: [retained-grid commands](./evidence/028/grid-keyboard-source-checks/README.md).
Home/End, Page Up/Down, Shift extension and Escape pass source/native/package
checks; actual keyboard/AX verification is pending native desktop tool access.

Current connection increment: [direct staged diagnosis](./evidence/029/connection-diagnosis-progress.md).
The native form now uses a bounded six-stage report with cancellation, credential
destination checks and shared payload accounting. Required/backend/native
checks, the owned TLS matrix and separate package pass; scoped direct-diagnosis
window checks and rebuilt warning AX labels also pass. Full connection and IME
acceptance remain open. Full PostgreSQL parity remains IN PROGRESS.

Native CUA access returned for the [diagnosis/grid keyboard window run](./evidence/029/diagnosis-window-20261003/README.md). Scoped diagnosis, retained-grid commands and clean shutdown passed. Missing warning AX names were source-corrected and verified in the rebuilt table-copy package. Intermittent capture/AX activation limits and unobserved IME composition remain explicit. VoiceOver stays deferred.

Current transfer increment: [table copy and durable app-owned recovery](./evidence/029/table-copy-progress.md) is under implementation and verification. It uses the approved Tool tab and Bottom review. Required backend checks, bounded native source checks, owned live copy and scoped actual-window review/apply/tab-closure checks pass. Final-package interrupted-apply/reopen/refusal/reconciliation checks also pass. Broader AX/IME and full parity acceptance remain open; see [evidence](./evidence/029/table-copy-source-checks/README.md). Full parity stays IN PROGRESS.

Scoped [CSV actual-window export/import](./evidence/029/csv-window-20261003/README.md) now passes on the frozen table-copy receipt package: 60 rows, exact two-way equality, native file dialogs, immutable indexed mapping and guarded cleanup. Complete keyboard/AX/IME acceptance remains open. XLSX import preparation follows the [reviewed existing dependency edges](./evidence/029/xlsx-import-source-checks/dependency-review.md); scoped import evidence is recorded separately.

Current XLSX increment: [bounded workbook and sheet import](./evidence/029/xlsx-import-progress.md) reuses the approved transfer layout. Required/source checks and scoped actual-window import pass. The two window corrections also pass their separate recheck; full keyboard/AX/IME and parity acceptance remain open.

Current Structure increment: [bounded typed reader and native inspector](./evidence/029/table-structure-source-checks/README.md) passes backend/live and scoped release-window checks, including stale-capture and changed-identity refusal. Two observed catalog/partition-label issues were corrected and rechecked. DDL editing and complete parity remain open.

Current seed increment: [column-aware setup, owned execution and format-11 recovery](./evidence/029/seed-progress.md) has focused source and owned PostgreSQL probe coverage. Required checks, corrected release keyboard/receipt checks and scoped interrupted-write recovery pass. Complete keyboard/AX/IME and full parity remain open.

Current overview increment: [bounded database/schema/relation statistics](./evidence/029/overview-source-checks/README.md) adds Administration inspection with explicit estimates and document-bound pages. Required/backend/native checks and scoped window paging, estimates, cancellation, stale identity and corrected row rendering pass. Complete keyboard and Tool-tab IME acceptance remain open. Full PostgreSQL parity remains IN PROGRESS.

Current metadata-artifact increment: [DDL export and real schema maps](./evidence/029/metadata-artifacts-source-checks/README.md) integrate owned read-only captures, scoped preferences, native Tool tabs and bounded file exports. Source/live checks and integrated release packaging pass; scoped corrected-window evidence covers DDL files, map rendering, movement, persistence/reopen and exports. Full acceptance remains in progress. Unicode PNG, legacy preference import and full keyboard/IME acceptance remain open. Plans 027–030 stay IN PROGRESS.

Existing-table comment/rename now has [native review and version-13 recovery](./evidence/029/table-ddl-activation-source-checks/README.md). Corrected debug/release checks and packaging pass (383 native tests, 13 ignored). Native automation failed at pipe startup before any DDL window interaction; scoped recovery/keyboard/IME acceptance remains pending. The owned process and unused fixture were cleaned up. Full PostgreSQL parity remains IN PROGRESS.
