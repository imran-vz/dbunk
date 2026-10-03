# Plan 029: Native PostgreSQL tools

- Verification scope updated by Imran on 2026-10-03: VoiceOver is deferred and is not a blocking gate. Keyboard/AX and real IME checks remain required; see [the scope decision](./evidence/027/accessibility-scope-20261003.md).

- Migration stage 06, PostgreSQL subset; macOS Apple Silicon only.
- Written on 2026-10-02 against `3f987c96640d6738b48ae1113ceac3ebbdc8563f`, with the uncommitted Plan 027 work identified separately in its evidence.
- Behavioral baseline: Tauri at `102568b`. New capability follows the approved backend-first rule.
- Current Structure increment: [bounded typed reader and native inspector](./evidence/029/table-structure-source-checks/README.md) has backend/live and scoped release-window evidence; two observed catalog/partition-label issues were corrected and rechecked. Table DDL editing remains open.
- Current overview increment: [bounded database/schema/relation statistics](./evidence/029/overview-source-checks/README.md) is integrated into Administration. Required/backend/native checks and scoped window paging, cancellation, stale identity and corrected row rendering pass; complete keyboard and Tool-tab IME acceptance remain open.
- Current seed increment: [column-aware recipes, owned execution and durable recovery](./evidence/029/seed-progress.md) passes focused source and scoped PostgreSQL probes. Required/native checks, corrected release keyboard/receipt checks and scoped interrupted-write recovery pass. Full keyboard/AX/IME acceptance remains open.
- Latest XLSX import: [bounded source, exact fixture and scoped native-window evidence](./evidence/029/xlsx-import-source-checks/README.md) passes worksheet selection/review invalidation and exact two-row import. Receipt keyboard scrolling and stale-message corrections pass their separate recheck; full keyboard/AX/IME acceptance remains open.
- Latest table copy: [source, live and scoped native-window evidence](./evidence/029/table-copy-source-checks/README.md) passes exact values, app-owned tab closure, persisted admission, interrupted Applying → Unknown, refusal before reconciliation and explicit new-attempt recovery. Shared payload bounds and final package hashes are recorded; broader AX/IME and full parity remain open.
- Latest formatter evidence: [bounded whitespace edits and editor command](./evidence/029/formatter-source-checks/README.md).
- Latest completion evidence: [owned column metadata and native suggestions](./evidence/029/completion-source-checks/README.md).
- Latest DDL increment: [Create Schema with durable exact-attempt recovery](./evidence/029/schema-create-source-checks/README.md); focused/frozen checks, owned live verification and packaging pass; CUA discovery failed, actual-window acceptance pending. Existing-object lifecycle and designer remain incomplete.
- Latest server inspection: [facts/settings/extensions and owned reader](./evidence/029/server-details-source-checks/README.md); frozen source and corrected live checks pass, separate package passes; CUA discovery failed, actual-window pending.
- Latest local audit increment: [bounded retained safety overrides](./evidence/029/safety-audit-source-checks/README.md); focused/frozen checks and separate package pass; actual-window acceptance pending.
- Latest administration evidence: [read-only Tool tab and owned reader](./evidence/029/admin-source-checks/README.md).
- Latest whole-table export and settings increment: [complete captures, saved configurations and scoped file/window checks](./evidence/029/whole-table-export-source-checks/README.md), plus [the local Connection settings mirror](./evidence/029/connection-settings-source-checks/README.md). Initial JSON/SQL/XLSX/CSV values pass; observed AX/focus/status corrections and process-reopen checks are under verification. [Existing-table comment/rename services](./evidence/029/table-ddl-source-checks/README.md) have focused/live checks but no native activation yet.
- Latest export/drop-impact evidence: [source and exact live probe](./evidence/029/export-impact-source-checks/README.md).
- Latest metadata/FK evidence: [source checks and exact live probes](./evidence/029/metadata-fk-source-checks/README.md).
- Latest connection work: [direct PostgreSQL staged diagnosis](./evidence/029/connection-diagnosis-source-checks/README.md) passes required/backend/native checks, the owned TLS matrix and separate packaging, including stored-credential destination checks and joined cancellation. [URI import and secret-free copy](./evidence/029/connection-uri-source-checks/README.md) have separate frozen evidence. New-control window acceptance, SSH and managed PostgreSQL remain incomplete.
- Status: IN PROGRESS. History/saved queries, EXPLAIN, Objects and Administration Tool tabs are implemented, with scoped [library window evidence](./evidence/029/library-activation-source-checks/README.md) and [session-control evidence](./evidence/029/admin-control-source-checks/README.md). Formatter keyword normalization, owned Create Schema, backup/restore, CSV transfer and comparison have separate source/live evidence; their complete window acceptance remains open. [Maintenance and materialized-view refresh](./evidence/029/maintenance-progress.md) are now integrated with exact-save recovery and are under verification. [DDL export and real schema maps](./evidence/029/metadata-artifacts-source-checks/README.md) now have integrated native implementations, source/live checks and scoped corrected-window evidence; full keyboard/IME acceptance and Unicode PNG remain open. Existing-object lifecycle, designers, remaining formats and advanced connections are incomplete. These increments do not establish full tool parity.
- Depends on [Plan 027](./027-native-workspace-shell.md) workspace/credential gates and [Plan 028](./028-native-postgres-data-workflow.md) data-service and lifecycle contracts.
- Scope ledger: [native PostgreSQL inventory](./native-postgres-parity-inventory.md), [baseline checklist](./evidence/024/parity-checklist.md), and [gap register](./parity-gap-register.md).
- UI decision: [A, Tool tabs](./mocks/native-postgres-tools/index.html#a), selected by Imran on 2026-10-02. B and C remain unselected alternatives.

## Outcome and boundary

Complete the PostgreSQL tools already available in the baseline: SQL tools and
saved work, catalog/DDL/designer, schema map, file jobs and formats, comparison,
administration, and advanced PostgreSQL connection setup. Each tool must work
through the native workspace with the same policy, ownership, result limits and
failure disclosures as its shared service. Plan 030 then verifies the combined
PostgreSQL product and disposable profile/package compatibility.

This is not full stage 06 across every engine. Other engines, daily-driver
cutover, release publication and dependency upgrades remain outside this plan.
Missing competitor features in the gap register do not become migration scope.
In particular, do not add a debugger, visual query builder, data comparison,
schema migration generation, broader comparison coverage, new transfer formats,
scheduling, or advanced monitoring merely to close a checklist label.

## Design gate

Workspace A and table Bottom review A remain selected. Preserve historical
selections: object explorer C, table designer A, backup/restore A+C, CSV Transfer
A, and schema-comparison Object inspector A. Their behavior is documented in
Plans 013–022 and ADR-0026 through ADR-0030; surviving artifacts are linked from
the parity inventory. Historical selections do not authorize a substantially
different native arrangement.

Imran selected A, Tool tabs, for history, saved queries, EXPLAIN, administration
and advanced connection setup on 2026-10-02. That design gate is satisfied;
activate those native surfaces after their service and dependency gates pass.
Schema-map controls, export settings or other substantial UI not adequately covered by the chosen artifact need focused
static alternatives and a selection before activation. Reuse the chosen shell,
black background, white primary text, density metrics and separators. No
continuous repaint animation.

Backend service extraction, pure model ports, owned fixture preparation and
focused headless tests can proceed independently. A backend-ready row remains
backend-ready until its active real-window, keyboard/AX and IME gates have evidence; a mock is not
an acceptance result.

## Complete capability and service ledger

The entries below are mandatory baseline behavior, not optional examples. Resolve
stale source paths against `git show 102568b:PATH` and record the current path in
the implementation evidence. Do not use a current stub or a roadmap checkmark as
proof that a baseline capability was ported.

| ID | Capability to deliver | Existing seam and native work |
| --- | --- | --- |
| T01 | Connection-scoped schema-aware SQL completion, quoting and snippets/templates; format command and shortcut preserving useful selection/undo behavior | Port behavior from `sql-completions.ts`, `sql-format.ts`, query toolbar/editor helpers and their tests. Feed completion through the catalog service. Decide a native formatter implementation after testing the baseline dialect corpus; JS `sql-formatter` cannot be assumed available in GPUI. Do not add a dependency or alter the pinned graph without the applicable review. |
| T02 | Query history with connection/search/status filters, duration and truthful row counts; saved-query create/edit/delete/open; opening saved work produces a draft, never execution | Extract the six history/saved-query functions in `commands/relational.rs` into profile-storage services. Port `query-sidebar`, history UI and storage semantics. Carry baseline limits and ordering. Resolve the documented omitted-row double-count bug deliberately; do not reproduce it as parity. Keep cancelled-history behavior explicit. |
| T03 | EXPLAIN and EXPLAIN ANALYZE tree, cost/actual-time/rows/buffers, source SQL, JSON/text inspection and incomplete-plan errors | Use Query Session execution and the existing policy gate, with the baseline `EXPLAIN (ANALYZE, BUFFERS, FORMAT JSON)` behavior where applicable. Port `query-editor/explain` and `plan-analysis` models. ANALYZE may execute writes; never route it through an unguarded metadata read. |
| T04 | Result/selection and whole-table CSV, JSON, SQL INSERT, HTML, Markdown, TXT and XLSX exports; UTF-8/UTF-16LE, gzip and NULL-token settings where the baseline supports them; saved re-runnable export configurations | Port `src/lib/export.ts`, `export-tasks.ts` and their tests. Reuse Rust `xlsx.rs`; provide host-neutral bytes/file adapters and native save dialogs. Distinguish retained-result export from whole-table retrieval and bounded CSV jobs. Preserve escaping, NULL versus empty, numeric text and partial-result disclosure. A saved export task is configuration, not a scheduler. |
| T05 | Filterable schema/object Navigator; database Tables/Schemas lists; object descriptions, definitions, dependency/reference inspection and table Structure | Extract `load_schema_explorer`, `load_table_structure`, and `pg_objects` catalog/describe/drop-impact orchestration. Preserve capped, overload-safe Object Refs. Include tables, views, materialized views, foreign tables, functions/procedures/aggregates, sequences, types/domains, event triggers, extensions, roles/users and tablespaces; per-table triggers/rules/policies/partitions/dependencies/references. Read-only inspection does not imply structured editing of every kind. |
| T06 | Reviewed schema/view/materialized-view/sequence/enum lifecycle, comments and supported-kind drops; sequence Inspect/Advance/Set/Restart; table designer and Structure edits; indexes/constraints/FKs; function/procedure bodies and attributes; triggers, RLS/policies and supported privileges | Extract `preview_object_ddl`/`apply_object_ddl` and required relational actions. Reuse typed `PgObjectOp` builders and the existing DDL lock/fence. Port selected designer, Object Viewer, Structure and Specialized forms. Keep tagged literal/expression defaults, generated/identity handling, typed-operation regeneration on apply, atomic versus standalone groups, committed prefixes/residue and qualified target display. Preserve drop-impact uncertainty. DDL preview never writes. |
| T07 | Table/schema/database DDL export at existing relation-oriented coverage; explicit materialized-view refresh | Expose `export_ddl` and `refresh_materialized_view` through services; retain policy and capability checks. Generated DDL is not a complete canonical database backup. Keep unsupported object categories disclosed. |
| T08 | Real schema relationship map, cardinality and column anchors, FK labels, pan/zoom/drag and layout/reset; saved positions; All/Keys-only/None, type/NULL/comment toggles, routing and PNG/SVG export | Extract relationship and map-position/preference operations in `commands/relational.rs`. Port `schema-graph.ts` and map behavior to bounded native models/layout. Preserve the baseline database-wide (all schemas, default), single-schema, and table/direct-neighbor scopes, each with its own saved preferences. This is confirmed in `102568b:src/components/workspace-overview/schema-map-tab.tsx`, `table-editor/schema-map-sub-tab.tsx` and `src/lib/schema-graph.ts`; stale roadmap descriptions are not authoritative. Synthetic spike nodes are not accepted evidence. |
| T09 | Global and table-context backup/restore, plain/custom archive choices, database-target restore review, client preflight, progress, cancel, release and session history | Extract `commands/pg_backup.rs` into a PostgreSQL Tool Job service over the existing manager/runner. Reuse ADR-0028's file publication, subprocess, redaction, safety and reaper contracts. Native file dialogs select paths; they do not authorize execution. No invented percentage where restore has no reliable denominator. |
| T10 | Bounded CSV inspect/map/import/export, dialect/header/NULL settings, defaults/generated-column restrictions, progress/cancel/release; XLSX sheet selection and mapping; table-to-table copy and column-aware seeding | Extract `commands/pg_transfer.rs` and the needed `import_rows`, `copy_table_rows`, `seed_table` orchestration. Reuse transfer managers and Rust XLSX parser. Keep indexed source-column mapping, inspection expiry, file/relation identity checks, one-transaction import, safe publication and `outcomeUnknown`. Label baseline buffered XLSX/copy/non-CSV paths honestly; do not silently fall back from rejected bounded CSV. |
| T11 | Read-only PostgreSQL Schema Comparison with explicit source/target endpoints, coverage, object/field pages, lazy values, progress/cancel/release and session jobs | Extract `commands/pg_schema_compare.rs` without exposing its manager. Replace the WebView transport adapter with a native document-bound read/ACK lease owner. Retain PG16 ordinary-table scope, exact endpoint identities, same-connection snapshot semantics, independent cross-server times, incomparable fields, expiry and all byte/admission limits. No migration SQL. |
| T12 | Overview statistics, relation/schema sizes/counts, recent queries/favorite tables/health and activity; server version/encoding/locale/timezone, settings catalogue/modified filter and extensions; read-only Settings mirror with Edit | Expose `load_database_overview_stats`, `load_relation_stats`, `load_server_details` and existing connection activity/health/storage operations. Port the actual baseline overview behavior, including empty/error/permission states. Avoid per-tab duplicate polling and misleading zeroes on failed reads. |
| T13 | Sessions, locks/blocker chains and pending transactions; explicit cancel/terminate; VACUUM/ANALYZE/REINDEX; materialized-view refresh and safety-override audit | Extract relational administration functions and `load_safety_overrides`; reuse `postgres/admin.rs` and stored policy. Bind action/review to the current connection and observed backend identity. Revalidate targets to prevent stale/PID-reuse action; never cancel an arbitrary fixture or developer process. Keep permission failure distinct from an empty list. |
| T14 | PostgreSQL SSH/proxy route configuration; bastion CRUD/test/fingerprint reset; staged tunnel→DNS→TCP→TLS→authentication→database diagnosis with observed encryption; URI import and secret-free Copy URI; remaining colors/recency/health behavior | Extract `commands/bastions.rs` and `diagnosis.rs`, reuse tunnels, ADR-0018/0025 and connection services. Extend profile-scoped atomic secret recovery to bastions before activation. Preserve changed-host-key review, policy fields, password omission rules and URI certificate-path disclosure. Current reachable/failure Test Connection is insufficient for staged-diagnosis parity. |
| T15 | Managed PostgreSQL Docker availability/provision/list/start/stop/destroy/recreate and associated connection behavior | Extract `commands/managed.rs`; reuse the existing managed implementation and ADR-0019. Add an owned-container fixture boundary and profile-bound credentials, including failure rollback. Do not include managed MySQL in this PostgreSQL plan or adopt a pre-existing container by name. |

Transactions, execution selection/current/all, parameter/row-limit activation,
Table Browse preferences/history/presets/count/filter/sort, value inspection,
JSON/array/geometry editing, copy formats, identity-safe mutation review and FK
drill-down belong to Plan 028. Shell navigation, global Open Anything/palette,
theme/density/preferences, geometry, logging and compatibility finish in Plan
030. Their placement is explicit so they cannot disappear between plans.

## Shared architecture and limits

Read ADR-0032 and the relevant family ADR before extraction. Keep `Backend` as
the narrow public surface. Services accept typed requests and own stored policy,
connection generation, credential resolution, audit and activity; raw managers,
pools and hydrated connection records stay private. Tauri adapters keep existing
command names and JSON. Preserve backend tests under both host configurations.
The later `dbunk-core` crate move is not a prerequisite or permission for a broad
restructure during these ports.

Every new network service must verify the launcher-provided owned endpoint and
connection/profile generation before secret hydration or socket creation. SSH,
managed containers and PG16 comparison need explicit new manifest capabilities;
do not widen an old profile's immutable manifest or enable arbitrary endpoints.
Schema names, file paths and forwarded ports are data, never shell fragments.

Carry each manager's existing limits without silently increasing them. Share
workspace admission/retention accounting from Plans 027/028 across active tools;
reserve memory before retaining new payloads. Catalog, completion, plan trees,
map layout, export preparation and buffered legacy paths need measured finite
budgets and an explicit refusal/export path before activation. Avoid whole-file
or whole-table arrays in the renderer for bounded CSV and comparison. Table and
result snapshots must distinguish truncation from a complete export.

A job's lifetime is not its setup view's lifetime. Backup/transfer/comparison
continue under one app-owned observer when their setup tab closes, as in the
baseline. Reopening the view reconciles existing job identity, never restarts it.
Document reads, previews and map/layout work cancel on replacement; running jobs
cancel only through explicit cancellation or their service lifecycle fence.
Connection edits, credential changes and application quit fence admissions and
join the relevant workers, sockets, observers, tunnels and children. Preserve
ADR-specific reaper/quarantine and uninterruptible-worker behavior: a deadline
cannot turn unfinished cleanup into success or free an admission early.

## Execution sequence

1. **Freeze the ledger and prepare fixtures.** Trace T01–T15 to baseline code/tests,
   list each reusable service and frontend-only port, record selected historical
   layouts and tools A, and identify any uncovered UI decisions. Produce a scoped fixture manifest and
   cleanup proof before any listener/container is started. Use the source-confirmed map scopes above and resolve formatter behavior from
   its baseline corpus, not assumed parity.
2. **Extract storage, catalog and administration services.** Port history/saved
   queries, metadata, DDL, map preferences and admin orchestration one family at a
   time. Add no native caller that bypasses admission or policy. Verify unchanged
   Tauri adapter contracts and new facade refusals before activation.
3. **Prepare frontend-only models.** Port completion, formatter/snippets, EXPLAIN,
   export and graph models using meaningful baseline fixtures. Test Unicode,
   quoting, escaping, NULL/empty, malformed plans and deterministic map identities.
   Decide bounded input/output behavior before connecting views.
4. **Extract job/file services and the native comparison reader.** Start with one
   manager, prove cancel/invalidation/quit ownership, then add the next. Keep CSV
   and backup as distinct protocols. Exercise all three managers together against
   global admission and shutdown; independent family passes alone are insufficient.
5. **Extract transport and managed services.** Finish atomic bastion credentials,
   SSH/proxy/fingerprint handling, diagnosis and owned Docker lifecycle. Preserve
   direct TLS checks. Only then activate advanced connection controls using
   selected tools A; leave unsupported stored records intact until supported.
6. **Activate approved tools in usable slices.** Catalog/DDL/designer; SQL/history/
   EXPLAIN/export; map/overview/admin; jobs/transfers/comparison; advanced transport.
   Apply selected Tool tabs A to the affected slices. For each, deliver
   loading/locked/empty/error, keyboard/AX, stale/cancel and teardown behavior in
   the same slice, then mark that ledger row implemented.
7. **Run integrated acceptance and hand off to Plan 030.** Exercise saved query →
   EXPLAIN → table/DDL review → export/import → comparison → admin on owned data;
   separately exercise SSH and managed PostgreSQL. Record every remaining gap by
   ID and failing gate. Do not report full parity until Plan 030 passes.

Each step is a reviewable unit, not a mandate for one large implementation diff.
Re-estimate after service/fixture preparation if a formatter dependency, transport
boundary or workload budget changes the scope.

## Owned fixtures and focused failure tests

| Boundary | Fixture and acceptance |
| --- | --- |
| Catalog/DDL/admin/map | New uniquely named schemas/roles/tables on an explicitly verified disposable PostgreSQL instance. Exercise mixed-case/quoted names, overloads, dependency caps, lock/permission failures, generated/identity columns, partial standalone DDL and stale refs. Cancel/terminate only a backend created by the test, matching database, user, PID and start identity. No production targets. |
| Query tools/files | Use the shared SQL and many/wide/large fixtures plus owned temporary output directories. Compare completion/format/EXPLAIN/export semantics to baseline tests, not screenshots. Verify exact encoding, NULL/empty and escaping; refuse truncated/failed reads instead of publishing an apparently complete file. |
| Backup/restore/transfer | Restore only into a new owned disposable target. Include destination-exists, symlink/path changes, wrong client, denied credentials, stale inspection, malformed late CSV rows, disk/write/publication failure and lost commit acknowledgement. Verify the original target/output is preserved when the contract promises that; never claim rollback for an uncertain or already committed write. |
| Comparison | Reuse the ownership-checked PG16 comparison fixture methodology, with explicit endpoints and version-refusal controls. Stage03's PG17 fixture cannot establish successful PG16 comparison. Exercise stale/expired response, UTF-8 chunk edges, retained leases after delivery, document close and endpoint invalidation while a read is in flight. |
| SSH/proxy/TLS | New private fixture keys and fingerprints, loopback listeners and generated certificates with no global trust installation. Name service/account identities before any OS Keychain operation. Test changed host key, denied access, failed mode conversion, cancellation during resolution and exact owned worker/route cleanup. Preserve SQLx's CA-only hostname limitation over SSH and missing pooled TCP keepalive disclosure. |
| Managed PostgreSQL | Use unique labels, container/volume IDs and loopback ports in a manifest; validate all before each operation. Test provision rollback, stop/recreate with live jobs, missing Docker and denied permissions. Cleanup deletes only resources created by that fixture and verifies absence by immutable identity. No broad Docker prune. |

Fixture setup must record endpoint, ownership marker/instance, process or container
identity and source/build hash before mutation. Existing stage03/stage04 fixtures
remain unchanged. Failed checks preserve evidence and any source files needed for
recovery. Final acceptance includes database backend counts, child/worker counts,
owned temporary files and queue/model high-water marks, with all cleanup failures
reported explicitly.

Use focused model/service tests for refusal-before-dispatch, preview/apply binding,
stale replies, unknown outcomes and ownership. Run meaningful existing tests while
the Tauri path remains supported. Do not mirror every React component snapshot.
Actual-window tests cover all T01–T15 rows; record keyboard/AX and real IME for new
forms, dialogs, editor tools and map navigation separately. VoiceOver is deferred,
not passed; automated AX is not a human listening result. Measure release input/scroll/idle and repeat at least 20 open/run/
close or job-release cycles on representative heavy tools.

## Evidence and completion

During implementation run required `pnpm format`, `pnpm lint`, `pnpm typecheck`,
relevant `pnpm test`, `just fmt`, `just lint`, `just test`, native debug/release
Clippy/tests/build, custom-protocol Tauri build and core dependency proof. Add
noninteractive checks to macOS CI; interactive OS Keychain/AX work stays explicit.

Store per-row baseline references, source/executable hashes, commands/toolchains,
fixture/profile identities, expected outcomes, automated/live/window/human
results, budgets and teardown evidence under `plans/evidence/029/`. An honest
pending gate is not a failed implementation claim, but it prevents READY FOR
REVIEW for this plan. DONE requires reviewed committed evidence and a completion
SHA. Update the canonical plan index only when that status is justified.

Stop the affected slice for a safety bypass, foreign resource access, secret
leak, data loss presented as success, unbounded retention, unjoined ownership or
missing design selection. Continue independent authorized backend work while a
UI choice or an active keyboard/AX, IME or actual-window gate remains pending.

Current file-job increment: [native backup/restore progress](./evidence/029/tool-jobs-progress.md). Stable attempt registration, owned source/process cleanup, restore data fences, transient Tool-tab setup and the shared observer are under implementation and verification. Final source/package, real-tool and window evidence remain pending; this does not close any plan.

Backup/Restore now has [frozen source, package and owned real-client evidence](./evidence/029/tool-jobs-source-checks/README.md). Plain/custom backup and restore pass the scoped fixture probe. Native window discovery failed before interaction, so keyboard/AX, real IME and complete job acceptance remain pending. CSV transfer ownership work follows; full PostgreSQL parity remains IN PROGRESS.

CSV transfer now has [frozen source, package and owned import/export evidence](./evidence/029/csv-transfer-source-checks/README.md). Indexed mapping, dialect/NULL controls, exact review, app-owned observation, joined cancellation and import-source invalidation are implemented. The real probe passed exact values, source replacement refusal and late-error rollback. Required/native checks and the separate package pass; CUA discovery failed before interaction, so keyboard/AX, real IME and full transfer acceptance remain pending. Workspace format 6 persists only CSV-tab identity/binding. Full PostgreSQL parity remains IN PROGRESS.

Current comparison increment: [native PostgreSQL 16 comparison progress](./evidence/029/schema-comparison-progress.md). App-owned jobs, exact request reconciliation, dedicated result-reader generations, bounded typed pages and UTF-8 value chunks are implemented. Focused checks and the owned PG16 facade probe pass; the corrected backend and refined native checks pass, and the separate package matches 435 source hashes. CUA window discovery failed; actual-window acceptance remains pending. Cached schema suggestions and presentation/copy/keyboard parity refinements are now source-implemented. This does not close any plan.

Current formatter increment: [keyword normalization progress](./evidence/029/keyword-format-progress.md). Known PostgreSQL keywords and phrases now normalize through Format SQL; exact bound parameters and protected text remain unchanged. Focused tests and the pinned baseline vocabulary probe pass. Required checks and the separate package pass against 439 source hashes. Actual-window acceptance remains pending.

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

The export correction now passes scoped individual-key, click-to-keyboard focus,
AX status, complete/cancelled parent status and disconnected recipe reopen checks.
The [broader button activation correction](./evidence/029/button-activation-source-checks/README.md)
removes duplicate Enter/Space callbacks while preserving editor and navigation
shortcuts. Its native matrix and representative window checks pass, including a subsequent
form-toggle focus correction.
Full keyboard/AX and real Tool-tab IME acceptance remain open; VoiceOver is deferred.

[Existing-table comment/rename activation](./evidence/029/table-ddl-activation-source-checks/README.md)
now connects typed Structure selection to the approved Objects bottom review,
with version-13 recovery, owned worker messages and exact-save Apply/Confirm.
Backend checks and the corrected native debug/release matrix and package pass.
CUA native-pipe startup failed before any DDL window interaction; the owned
process and unused fixture were cleaned up with explicit receipts. Actual-window
DDL/recovery acceptance remains pending. This does not implement other table
alterations or the full designer.
