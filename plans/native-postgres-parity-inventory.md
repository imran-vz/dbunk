# Native PostgreSQL parity inventory

Target: PostgreSQL behavior in Tauri at **`102568b`**, as requested by Imran on
2026-10-02. This is a source inventory and proposed implementation sequence,
not runtime acceptance evidence or a change to plan status.

[Plan 027](./027-native-workspace-shell.md) covers the native workspace,
credentials, direct connections and draft recovery; its acceptance remains in
progress. [Plan 028](./028-native-postgres-data-workflow.md) covers transactions,
Table Browse and mutations after those gates. The remaining families below
complete the requested PostgreSQL scope; they need executable plans and
verification before being called complete.

Sources: [baseline checklist](./evidence/024/parity-checklist.md),
[gap register](./parity-gap-register.md), [roadmap](../ROADMAP.md), baseline
command registration and React components/tests. At `102568b`, registration is
in `src-tauri/src/lib.rs`; the extracted current path is `tauri_host.rs`.
The React frontend is unchanged against that baseline at this inventory's
2026-10-02 review.

## Current boundary

The native app implements query execution and the Plan 027 workspace,
connection, credential and draft surfaces. Implementation is distinct from
passing all real-window, failure, accessibility and compatibility gates.
The [2026-10-03 scoped table window checks](./evidence/028/table-window-verification-20261003.md)
cover paging/filter/count, staged review/apply, column sizing/order/visibility
persistence, SQL-review AX, and actual Pinyin composition in SQL/form/cell
editors. Their exact builds are recorded separately from subsequent tools work.
A later [source/package variant](./evidence/028/browse-inspector-source-checks/README.md)
adds browse presets/history, value inspection, typed literal editors, virtual keys,
EXPLAIN drafts/tree and owned catalog reads. Source/live checks do not establish
actual-window acceptance for these controls. A subsequent [catalog/array slice](./evidence/029/catalog-array-source-checks/README.md) adds structured array elements, the Objects Tool tab and eight-kind bounded descriptions; its live probe passes while window acceptance remains pending. The [metadata/FK slice](./evidence/029/metadata-fk-source-checks/README.md) completes the twelve baseline description kinds and adds bounded composite FK navigation with exact live probes; reconstruction omissions are disclosed and new-control window acceptance remains pending. The [file/impact slice](./evidence/029/export-impact-source-checks/README.md) adds bounded retained-result file formats/settings/gzip/XLSX and read-only downstream drop impact; focused source and dependency live probes pass, with window acceptance still pending.
Most remaining PostgreSQL operations have reusable Rust implementations but
are reached through Tauri commands rather than the native `Backend` facade.
The Plan 024 cell-editor and schema-map demonstrations used synthetic data;
they do not establish those features in the native workspace.

All development continues on owned fixtures and isolated profiles. This
inventory does not authorize daily-driver migration, production connections,
release publication or dependency upgrades.

## Implementation sequence

| Family | Native work remaining | Existing seam and behavior reference | Verification to carry forward |
| --- | --- | --- | --- |
| **1. Transactions and table data: Plan 028** | Transaction mode/isolation/recheck/commit/rollback; parameters and row-limit controls; server paging/filter/sort/count; grid preferences; value inspection; staged insert/update/delete; virtual keys; FK drilldown and review/apply | `query_session/service.rs` already contains transaction and parameter operations. Extract orchestration from `commands/table_browse.rs` and `commands/result_mutation.rs`, retaining private managers. React references: `table-editor`, `data-grid`, `mutation-review`, `query-editor/transaction-controls.tsx`. | Existing query-session, table-browse and result-mutation suites. Add native independent transactions, stale pages, composite/virtual keys, conflicting rows, confirmation binding, uncertain writes and close-during-apply. |
| **2. SQL tools and saved work** | Schema-aware completion, formatting, snippets, history, saved queries, EXPLAIN visualization and result exports | Six history/saved-query operations in `commands/relational.rs` can become profile-storage services. Completion is `src/lib/sql-completions.ts`; formatting uses JS `sql-formatter`, requiring a native implementation decision. EXPLAIN reuses query sessions. Most exporters live in `src/lib/export.ts`; XLSX has reusable Rust implementation. | `sql-completions.test.ts`, `sql-format.test.ts`, query-history and query-editor component tests, EXPLAIN `plan-analysis.test.ts`, `export.test.ts`. Preserve Unicode, escaping, NULL/empty distinctions, retention limits, failed exports and incomplete-plan handling. |
| **3. Catalog, structure and DDL** | Schema/object navigator, descriptions, table structure/designer, routines, triggers, policies, grants, sequence actions, drop impact and DDL export | `commands/pg_objects.rs` catalog/describe/impact/preview/apply; several `_inner` functions already approximate a service seam but remain inside Tauri commands. Also extract relational schema/structure/export/lifecycle operations. References: `object-viewer`, `object-ddl`, `table-designer`, `table-structure`. | `postgres/objects.rs`, `postgres/object_ddl/tests/*`, `commands/pg_objects_live_tests/*`, frontend designer/review/impact tests. Preserve pure preview, regenerated typed operations on apply, partial-group commit/residue reporting, stale references and policy refusal. |
| **4. Schema map** | Actual metadata, cardinality/column anchors, layout, saved positions, attribute/routing preferences and PNG/SVG export | Relational relationship and map position/preferences commands; `schema-relationship-map*` and `src/lib/schema-graph.ts`. Native spike rendering is only a starting point. | Existing graph/map/toolbar tests; real large-schema rendering, bounded layout, persisted coordinates, keyboard/AX navigation and exported content. |
| **5. Jobs, transfers and comparison** | Backup/restore; bounded CSV mapping/import/export; existing XLSX import and export formats; table copy/seeding; schema comparison; progress/cancel/release and file selection | Extract orchestration from `commands/pg_backup.rs`, `pg_transfer.rs`, `pg_schema_compare.rs`; reuse managers/runners. Replace file-picker adapters. Comparison needs a bounded native reader preserving its response leases/ACKs. Also expose `import_rows`, `copy_table_rows`, `seed_table` as appropriate. | Backup lifecycle/live tests; transfer manager/CSV/runner tests; comparison capture/diff/manager and frontend protocol/reader tests. Preserve uncertain-start reconciliation, stale inspection refusal, cancellation, safe file publication, bounded result reads and joined child processes. |
| **6. Overview and administration** | Relation/schema statistics; server settings/extensions; sessions, locks and pending transactions; cancel/terminate; maintenance; matview refresh; safety audit | Relational `load_database_overview_stats`, `load_relation_stats`, `load_server_details`, `load_pg_admin_snapshot`, `cancel_pg_backend`, `terminate_pg_backend`, `run_pg_maintenance`, `refresh_materialized_view`; safety `load_safety_overrides`. Reuse `postgres/admin.rs` and existing policy enforcement. | Native refresh/cancellation, correct target PID, stale connection refusal and confirmation tests. Existing administration coverage is thinner than DDL/job coverage; do not infer acceptance from the shared queries. |
| **7. SSH, bastions and managed PostgreSQL** | Bastion CRUD/testing/fingerprint reset; SSH/proxy configuration; managed Docker lifecycle; full staged diagnosis; URI import | `commands/bastions.rs`, `managed.rs`, `diagnosis.rs` and existing tunnel/managed implementations. Native currently preserves but rejects unsupported SSH/managed records. Add profile-scoped atomic bastion secret changes and owned tunnel/container admission before activation. | Existing tunnel, diagnosis, bastion/store and managed tests. New owned SSH/proxy/Docker fixtures: changed host key, denied secrets, credential transitions, cross-profile refusal, reconnect and joined cleanup. Never adopt an arbitrary container as a fixture. |
| **8. Compatibility and acceptance** | Preserve disposable old-format connections/secrets/history/saved queries/drafts; finish shell navigation, command palette/Open Anything, preferences, geometry/logging and isolated package behavior | Plan 024 shell checklist, existing React persistence/navigation behavior, Plan 027 native storage and package work. Feature presence must be checked individually. | Release-window workflows, interrupted operations, exact acknowledged draft recovery, keyboard/AX and real IME (VoiceOver deferred by Imran on 2026-10-03), bounded memory/tasks, profile compatibility and packaged launch outside the repository. |

For every network family, keep stored policy, connection generation and
ownership checks behind typed services. Native fixture admission must happen
before sockets or secret hydration. Closing a document or window must join its
work under the shared cleanup deadline; exposing raw managers is not a service
extraction.

## Details that are easy to miss

- Baseline EXPLAIN runs `EXPLAIN (ANALYZE, BUFFERS, FORMAT JSON)` through guarded
  query execution. ANALYZE can execute writes; it must not become an unguarded
  metadata-pool call.
- Native Test Connection currently reports reachable/failure. Baseline diagnosis
  exposes tunnel, DNS, TCP, TLS, authentication and database stages, including
  observed encryption state. Direct TLS controls alone do not establish parity.
- Formatting, snippets, completion, exports and map layout include frontend-only
  behavior. They require ports, not merely additional command adapters.
- Driver-bound parameters and row limits were dark at the baseline. Their
  native activation is explicitly included in Plan 028; do not silently fall
  back to literal interpolation where a bound execution is unsupported.

## Design coverage

Existing selections persist. The [plan index](./README.md) records them:

- Native query layouts A+B+C; workspace A, Persistent Navigator; table review A,
  Bottom review. These cover the corresponding Plan 026–028 surfaces.
- [Native PostgreSQL tools A, Tool tabs](./mocks/native-postgres-tools/index.html),
  selected by Imran on 2026-10-02: history, saved queries, EXPLAIN,
  administration and advanced connection setup use named documents within shell
  A. Result export uses native file dialogs and schema maps retain a full-width
  document. B and C remain unselected alternatives.
- Earlier selected Tauri designs: object explorer C, table designer A,
  backup/restore A+C, CSV table-transfer A and schema comparison A. Reuse their
  behavior and selected organization where applicable. Surviving artifacts are
  under [table designer](./mocks/table-designer/mock-a.html),
  [backup/restore](./mocks/backup-restore/index.html),
  [CSV transfer](./mocks/csv-transfer/index.html) and
  [schema comparison](./mocks/schema-compare/index.html).

The broad [native parity scope](./mocks/native-postgres-parity/index.html#scope)
is not a detailed design for every family. Substantial departures from the
selected arrangements still need static alternatives and a selection before
real UI edits under the repository's design rule. Backend extraction and
focused service tests can continue independently of new design choices.

## Baseline limits, not migration promises

Full PostgreSQL **Tauri parity** does not mean completing every DBeaver gap.
The following stay outside this migration unless separately requested:

- PL/pgSQL debugger, visual query builder, data comparison and generated schema
  migration SQL; broader comparison object coverage beyond the existing bounded
  PostgreSQL 16 ordinary-table comparison.
- XML/Parquet, split exports and richer transfer formats/locale controls;
  replacing the baseline buffered XLSX path with a new streaming implementation.
- Diagram annotations, virtual relationships, custom subset design and other
  absent diagram editing features.
- Advanced monitoring, replication/WAL dashboards, slow-query digests,
  scheduling, enterprise identity and plugin ecosystems.
- New engines, Windows/Linux migration and daily-driver cutover.

Preserve truthful disclosure of inherited limits: SQLx metadata connections over
SSH verify the CA rather than the original hostname, SQLx has no applied TCP
keepalive option, query-script error policy/savepoints/command tags remain
limited, and comparison remains read-only. Known defects identified during a
port should be reported and corrected deliberately, not copied or presented as
newly verified behavior.

The [duplicate/bulk/snippet increment](./evidence/028/duplicate-bulk-source-checks/README.md)
implements the baseline row-copy and selected-row literal/NULL actions, plus
Query menu templates. It also admits draft/editor/recovery and workspace snapshot
working copies. Source checks do not replace actual-window, keyboard/AX or real
IME acceptance. Full PostgreSQL parity remains incomplete.

The [query-result mutation increment](./evidence/028/query-mutation-source-checks/README.md)
adds immutable execution source, UPDATE-only version 3 recovery and the existing
Bottom review/apply barrier to query documents. It remains restricted to qualified
non-temporary sources, a small exact builtin type set and ASCII captured guards.
Execution-origin/rendering-context metadata and native-window acceptance remain
open, so this does not complete the query-result editing inventory item.

The [read-only Administration increment](./evidence/029/admin-source-checks/README.md)
adds owned bounded snapshot reads and an approved Tool tab with manual refresh,
retained inspection and cancellation. Its scoped live probe does not establish
window parity, server-session control, maintenance or overview/settings parity.

[Native SQL completion](./evidence/029/completion-source-checks/README.md) now
provides connection-scoped catalog names and lazy column suggestions through the
owned data worker. Heuristic context, bounds and fallback-schema semantics are
explicit; actual-window completion acceptance remains pending. A [bounded SQL layout formatter](./evidence/029/formatter-source-checks/README.md) now edits only whitespace gaps and refuses uncertain input without changing the draft. Baseline keyword normalization and formatter/window acceptance remain open.

[Explicit general native profiles](./evidence/030/general-profile-source-checks/README.md) now have separate constructors, marker/SQLite identity and endpoint authority. Required checks, an owned service probe and separate package pass. Native-window discovery failed; keyboard/AX and IME remain unverified. Fixture profiles are not converted. This does not finish profile import, advanced transports, production identity or native-window acceptance.

[URI import and copy](./evidence/029/connection-uri-source-checks/README.md) have passing source checks and a separate frozen package; actual-window acceptance remains pending. URI import is explicit clipboard prefill; copying omits secrets and connection policy. This does not finish the remaining advanced connection workflows.

Latest read-only administration increment: [server facts, captured settings search/source filter and extensions](./evidence/029/server-details-source-checks/README.md). Frozen source checks and the corrected owned live probe pass; the separate package passes, but CUA discovery failed and actual-window acceptance remains pending. The connection Settings mirror, overview statistics, audit and administration write actions remain open.

Latest DDL increment: [Create Schema and exact-attempt recovery](./evidence/029/schema-create-source-checks/README.md). Focused/frozen source checks, owned live verification and packaging pass; CUA discovery failed and actual-window acceptance remains pending. Workspace format 4 preserves older supported records and adds bounded Objects journals. Existing-object lifecycle, standalone groups and designers remain open.

Latest local administration increment: [retained safety overrides](./evidence/029/safety-audit-source-checks/README.md), with bounded profile-local pages and disconnected inspection. Focused/frozen checks and the separate package pass; actual-window acceptance remains pending. PostgreSQL control and maintenance actions remain open.

Current file-job increment: [native backup/restore progress](./evidence/029/tool-jobs-progress.md). Stable attempt registration, owned source/process cleanup, restore data fences, transient Tool-tab setup and the shared observer are under implementation and verification. Final source/package, real-tool and window evidence remain pending; this does not close any plan.

Backup/Restore now has [frozen source, package and owned real-client evidence](./evidence/029/tool-jobs-source-checks/README.md). Plain/custom backup and restore pass the scoped fixture probe. Native window discovery failed before interaction, so keyboard/AX, real IME and complete job acceptance remain pending. CSV transfer ownership work follows; full PostgreSQL parity remains IN PROGRESS.

CSV transfer now has [frozen source, package and owned import/export evidence](./evidence/029/csv-transfer-source-checks/README.md). Indexed mapping, dialect/NULL controls, exact review, app-owned observation, joined cancellation and import-source invalidation are implemented. The real probe passed exact values, source replacement refusal and late-error rollback. Required/native checks and the separate package pass; CUA discovery failed before interaction, so keyboard/AX, real IME and full transfer acceptance remain pending. Workspace format 6 persists only CSV-tab identity/binding. Full PostgreSQL parity remains IN PROGRESS.

Current comparison increment: [native PostgreSQL 16 comparison progress](./evidence/029/schema-comparison-progress.md). App-owned jobs, exact request reconciliation, dedicated result-reader generations, bounded typed pages and UTF-8 value chunks are implemented. Focused checks and the owned PG16 facade probe pass; the corrected backend and refined native checks pass, and the separate package matches 435 source hashes. CUA window discovery failed; actual-window acceptance remains pending. Cached schema suggestions and presentation/copy/keyboard parity refinements are now source-implemented. This does not close any plan.

Current formatter increment: [keyword normalization progress](./evidence/029/keyword-format-progress.md). Known PostgreSQL keywords and phrases now normalize through Format SQL; exact bound parameters and protected text remain unchanged. Focused tests and the pinned baseline vocabulary probe pass. Required checks and the separate package pass against 439 source hashes. Actual-window acceptance remains pending.

Current grid sizing increment: [content-derived widths and auto-fit](./evidence/028/auto-fit-progress.md). Query/table defaults sample retained rows; explicit auto-fit preserves per-result query geometry and the table preference acknowledgement barrier. Required/native checks and separate packaging pass against 440 source hashes. Actual-window acceptance remains pending.

Latest actual-window evidence: [auto-fit package and disconnected reopen](./evidence/028/auto-fit-window-20261003/README.md). Scoped formatting, table geometry and library/administration checks ran; full acceptance remains open. The restored-library budget failure is tracked in the [activation fix](./evidence/029/library-activation-source-checks/README.md). VoiceOver remains deferred.

Native administration cancel/terminate now has an immutable captured-target
review, stored-policy confirmation, exact-save dispatch barrier and version-8
read-only recovery. Its owned stage03 backend probe passed with activity 0 → 0;
[source and verification scope](./evidence/029/admin-control-source-checks/README.md)
records the PostgreSQL signal-identity race. Required/native/backend checks and
the isolated package pass; [scoped window checks](./evidence/029/admin-control-window-20261003/README.md)
pass cancel, policy confirmation/termination, staged reopen and explicitly
injected unknown recovery, with normal quit and activity 0 → 0.
Full parity remains IN PROGRESS; VoiceOver stays deferred.

Current whole-table export and Connection settings increment: [source and scoped window evidence](./evidence/029/whole-table-export-source-checks/README.md). JSON, compressed UTF-16LE SQL, XLSX and routed CSV files match the owned 13-row fixture while the grid is filtered to one row. Saved recipes and the local settings mirror are implemented. Corrected export AX/focus/complete-and-cancel status and disconnected saved-configuration process reopen pass scoped checks. A [broader duplicate keyboard activation correction](./evidence/029/button-activation-source-checks/README.md) passes scoped release-window checks, including a subsequent form-toggle focus correction; real Tool-tab IME remains pending. [Existing-table comment/rename](./evidence/029/table-ddl-activation-source-checks/README.md) now has native observed-identity review and version-13 recovery source; corrected debug/release/package checks pass, while native-pipe startup failure leaves window acceptance pending. The owned process and unused fixture are cleaned up. Broader table alterations and designers remain unimplemented.
