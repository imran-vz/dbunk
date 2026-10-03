# Overview and server inspection reconnaissance

2026-10-03. Read-only source comparison against `102568b`. No app, network, fixture, database, or credential access; no source changes or test execution. This note covers the next read-only part of Plan 029 T12 and the T13 audit list. It is not implementation or window acceptance evidence.

## Recommended next slice

Add **Server facts, Settings, and Extensions** sections to the existing Administration Tool tab, using its current explicit Connect / Refresh / Cancel read / Clear capture and selected-row details conventions. Keep settings and extensions read-only. A changed-settings filter and bounded text search operate on the captured settings, not through new SQL on every keystroke. This gives a coherent bounded service with no relation-size scan, new data document, polling, DDL, or policy mutation.

Follow with database/relation/schema statistics, then profile-local recent history and safety audit. Do not couple failure of an optional settings section to the existing Sessions/Locks/Pending capture. These are separate request kinds over the same serialized document lane and can retain independently admitted captures.

## Exact baseline authority and gaps

| Capability | Baseline source and actual behavior |
| --- | --- |
| Visible overview | `src/components/workbench/relational-workbench.tsx:404` renders `workspace-overview/overview-rail-view.tsx`. Shows connection name/engine/database, connected state and stored latency; six summary metrics; all catalog tables with relation row estimates; first 20 connection-filtered history entries. Table click opens the exact schema/table. History click reopens exact SQL and connection ID; it does not run SQL. |
| Loading and errors | `workspace-overview/use-database-overview.ts` lazily requests overview and relation stats while connected and absent. `workspace-overview/format.ts` distinguishes pending, unavailable, and missing numeric values. Existing successful values win over a later error status, so native stale-capture disclosure should be clearer. `src/lib/store/relational-tables.ts:1488–1647` stores loading/success/error by connection and fences publication with the object-catalog generation. No periodic overview poll was found. |
| Favorite tables | The hook's `favoriteTables` is `tables.slice(0, 5)`, not saved favorites. The current rail view does not consume that field. Do not invent persisted table favorites or label arbitrary first rows favorites as verified baseline behavior. |
| Health | The hook constructs “All checks passing” unconditionally and uses the last query runtime as a latency item. The rail view does not consume `overviewStatus`; its health header only uses connection status and stored latency. There is no evidence here of an actual health-check suite. Native should show the observed connection/read state and collection time, not synthetic healthy status. |
| Server details | `src-tauri/src/postgres/admin.rs:108`, DTOs in `types.rs:2151–2200`, command `commands/relational.rs:689`, dispatch and store loader exist. Exact `102568b` frontend search finds **no component consumer of `serverDetails`, `bootVal`, or `resetVal`**. The source comments describe former Details/Tables/Schemas sub-tabs, but those comments do not establish an active view. |
| Changed settings | `PgSetting.source` documentation says non-`default` identifies overrides. No baseline component implements the planned modified filter. Choose and document `source != "default"` as “Non-default source”; `setting != boot_val` is a different predicate. Do not claim an exact baseline UI predicate. |
| Settings mirror / Edit | Plan T12 explicitly targets this. The active baseline connection settings surface is `components/connections-view.tsx` with Edit/Delete/connection actions and `edit-connection-dialog.tsx`; no read-only connection Settings mirror was found in the current overview rail. A future native mirror should use already-loaded secret-free metadata and invoke the existing exact-connection edit form, without remote reads or credential hydration. |
| Safety audit | `commands/safety.rs:61` exposes `load_safety_overrides`; no frontend invocation of that command was found at the baseline. Service/storage parity is real, a visible audit list is still a planned target. |

## Exact baseline remote DTOs and queries

All three legacy readers in `102568b:src-tauri/src/postgres/admin.rs` call the global `connect(connection)` path. Do not call these directly from native; port their SQL through the owned facade.

**DatabaseOverviewStats** has eight `i64` fields: `database_size_bytes`, `table_size_bytes`, `index_size_bytes`, `table_count`, `schema_count`, `row_count_estimate`, `index_count`, `connection_count`.

- `user_relations` includes `pg_class` joined to `pg_namespace`, excluding only `pg_catalog`, `information_schema`, and `pg_toast%`. It does not exclude `pg_temp%`.
- Database size is `pg_database_size(current_database())`. Table/index bytes sum `pg_table_size` / `pg_indexes_size` for relkinds `r,p`.
- Table count counts `r,p`; schema count counts namespaces represented by *any* row in that CTE, not all namespaces. Index count counts relkind `i`, not partitioned-index kind `I`.
- Row estimate sums `GREATEST(reltuples,0)::bigint` for `r,p`. Negative/unanalysed estimates become zero. Partition parents and their descendants participate independently, so this is not a guaranteed distinct-row total.
- Connection count counts all current-database `pg_stat_activity` rows, including idle sessions and potentially the reader. The rail calls this “Active connections”, which should not be confused with `state='active'`.
- Numeric decode errors become zero via `unwrap_or(0)` in the legacy mapper. Native must not copy that behavior.

**RelationInfo** contains `schema`, `name`, string `kind`, `row_count_estimate:i64`, and `total_size_bytes:i64`.

- Includes `r,p,v,m`, excludes `relispartition IS TRUE`, and uses the same namespace exclusions. Foreign tables are absent. Ordered by namespace/name.
- Maps `r,p` to table, `v` to view, `m` to materialized view.
- `r,p,m` use `GREATEST(reltuples,0)` and `pg_total_relation_size`; views report zero for both. Parent partitioned-table storage is not a recursive descendant total. Native should label views not applicable and unknown estimates unavailable rather than imply empty relations.
- There is no separate baseline schema-stat query. DTO comments describe frontend aggregation from relation rows; no active schema aggregate component was found. A native aggregate over a capped relation list must say partial, never present it as complete database/schema statistics.

**ServerDetails** contains strings `server_version`, `encoding`, `locale`, `timezone`, plus settings and extensions:

- Facts: `version()`, `current_setting('server_encoding')`, `current_setting('lc_collate')`, `current_setting('timezone')`.
- Settings: all `pg_settings` ordered by `category,name`; fields `name`, `setting`, optional `unit`, `category`, optional `short_desc`, `source`, optional `boot_val`, optional `reset_val`.
- Extensions: `pg_extension` joined to namespace, ordered by `extname`; `name`, `version`, `schema`, nullable `obj_description(e.oid,'pg_extension')`.
- Legacy readers fetch entire rowsets and substitute empty/default values on decode failure. One query failure fails the whole call. Native should preserve NULL, permission refusal, and malformed-response distinctions; do not return synthetic empty success.

## Owned native service contract

Reuse `Backend::object_read` in `src-tauri/src/backend/objects.rs:89`: `data_call` checks document/profile/connection generation before hydration, owns the permit and cancellation epoch, and registers drivers with the host task group. `backend/admin.rs::admin_snapshot` is the existing narrow facade example. Both general-profile authority and fixture-only authority remain unchanged.

Add a typed `server_details(&DataDocument)` read rather than enlarging every activity refresh. Use `postgres/native_catalog`'s `owned_read`, bounded read-only transaction, configured shorter statement timeout, 30-second asynchronous operation deadline, cancellation, and joined cleanup. Deadline wording must not imply a hard wall-clock bound for synchronous TLS-file work and joined abort.

Important observation scope: `begin_snapshot` currently issues `SET LOCAL statement_timeout` and `lock_timeout`. Reading `pg_settings` afterwards shows the inspection socket's overrides, not unmodified server settings or the SQL tab's session. Record the reader identity/collection interval and disclose reader-session scope. Either capture those two pre-inspection values before setting local limits under the existing outer deadline, or explicitly identify inspection-induced values. Do not silently label them operator changes. Connection driver role/search-path settings also belong to this reader's session.

Suggested initial contract and caps, to finalize with implementation:

- `ServerSnapshot { database, reader_pid, collected_start, collected_end, facts, settings, extensions }`; typed section outcome (loaded, restricted, unavailable) and truncation/omission metadata, not empty arrays alone. Use savepoints for optional sections on SQLSTATE `42501`; other database/parse errors remain errors. Do not catch every error as a permission restriction.
- At most 1,024 settings and 256 extensions, using SQL `LIMIT cap+1`, deterministic ordering and streamed `query_raw`. At most 1 MiB encoded reply and checked heap capacities. An extra row means truncated even if the displayed filtered list is empty.
- SQL-side `octet_length`/`CASE` prevents allocating oversized setting values/comments on the wire. Proposed 8 KiB per value/description and 256 bytes per name/category; over-limit cells are explicitly omitted with original byte length, or the section refuses. Never silently truncate a GUC value and display it as exact.
- A 2 MiB retained snapshot/view allowance under the shared 128 MiB budget, checked against actual DTO capacities/encoded size like `admin_model::Snapshot`. Admission of replacement accounts for old and new simultaneously; failure keeps the last good capture. Mailbox encoded delivery remains within 16 MiB.
- Virtualized rows, stable selected identity `(section,name)` rather than stale row index, persistent focus handles, bounded filter editor/history, actual marked-composition guards, and truthful copy readback. Search/filter results must preserve incomplete-catalog disclosure.
- Refresh has one monotonically identified request on the existing Administration `TableControls` lane; no second DataDocument. Cancel only that read; retain last good capture; stale/cancelled replies drop immediately. Restore remains disconnected; close/retarget/credentials edits keep existing host join/fence behavior. No automatic external read on opening or selecting a section.

## Later local and statistics additions

Recent queries should reuse `backend/query_library.rs::load_query_history` with the exact connection filter and cursor-aware bounded request, presenting up to 20 recent entries. Empty pages may still have a continuation. Reopening uses immutable saved SQL and connection binding; never autoexecute. Keep this profile-local data available disconnected and distinguish deleted connection bindings.

Safety audit is profile-local SQLite, not a PostgreSQL read or evidence that all server writes were observed. `102568b:storage.rs:1836–1898` retains the latest **1,000 globally**, then reads one connection ordered by `occurred_at DESC,id DESC`; DTO fields are `command`, `classes:Vec<String>`, `occurred_at`. The existing reader uses `fetch_all` and trusts persisted field lengths/JSON. Add a bounded native-only reader (e.g. 100 rows/page, 256 KiB, SQLite length/CASE before materialization, stable timestamp+id cursor); reject corrupt classes instead of silently returning no history. Label it retained successful safety overrides, not a full security audit. Current native query/data paths already record successful confirmed overrides; refused writes are not successful audit records. Never add clear/delete/replay/policy-changing actions in this slice.

For overview/relation stats, perform aggregate counts independently of paged relation listings. Preserve estimates and the documented relkind/schema scope; show unknown count of unanalysed relations rather than collapsing unknown estimates to zero. Relation pages should carry OID/kind/schema/name and full-scope-versus-displayed-subset metadata. Counts and filesystem size functions are observations collected over an interval, not an atomic cluster truth even inside repeatable-read. A restricted size query must not erase separately available counts.

## Narrow ownership and focused verification

1. Backend owner: new `postgres/native_catalog/server_details.rs` and child SQL/tests; new `backend/server_details.rs` export; minimal module/error declarations. No edits to ordinary Tauri DTO/command contracts or transport/admission capability.
2. Native model/view owner: new `server_details_model.rs` and child tests; extend or factor `admin_view.rs` sections using approved Tool-tab layout. Root owns command/message/runtime/module wiring. No new workspace document kind or layout decision required.
3. Later local audit owner: native-only bounded storage reader plus backend facade, separate from PostgreSQL reader ownership. Recent history uses existing library service. Root owns exact-connection Edit/OpenTable/OpenQuery event routing when those actions are added.

Focused tests: section NULL/restricted/error distinctions; all-filtered-but-truncated state; settings source-vs-boot-value predicate; inspection-induced GUC disclosure; row/field/byte cap boundaries and capacity rejection; duplicate-name/stale-selection refusal; old-capture retention on budget failure/cancel; monotonic request rejection after disconnect; exact page continuation; local audit corrupt/oversize rows and global-retention disclosure. Owned live read-only probes and actual-window focus/AX/IME checks belong to later authorized verification, not this reconnaissance. No write-action activation is proposed.
