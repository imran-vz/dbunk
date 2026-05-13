# Postgres parity — implementation phases

Sequencing plan for the DBeaver–Postgres parity goal described in [`ROADMAP.md`](../../ROADMAP.md). Each phase is a discrete deliverable that gets planned individually before it ships. Sequencing inside a phase is decided when that phase is planned.

Phases are ordered by user-facing pain first, then by dependency.

## Summary

| # | Phase | Status | Why this slot |
|---|---|---|---|
| 1 | Wire the connection-level tabs | ✅ shipped | Smallest scope, biggest perceived completeness win; most views are thin wrappers over data the store already has |
| 2 | Schema map overhaul | ✅ shipped | Directly addresses the "schema map is really not great" pain |
| 3 | Data export upgrade | ✅ shipped | First half of the import/export pain — lay the transfer foundation |
| 4 | Data import wizard | ✅ shipped | Second half — bulk inbound; pairs with Phase 3's transfer surface |
| 5 | DDL + dump/restore | ✅ shipped | Schema-level export and full `pg_dump`/`pg_restore` backups |
| 6 | Object navigator depth | ✅ shipped | Materialized views, functions, sequences, extensions, roles, tablespaces, etc. |
| 7 | Admin tools | ✅ shipped | Sessions, locks, pending transactions, VACUUM/ANALYZE actions |
| 8 | SQL editor depth | planned | EXPLAIN visualizer, snippets, bind vars (debugger as stretch) |
| 9 | Compare + generate | planned | Schema compare, data compare, mock data — depends on Phase 6 coverage |
| 10 | Specialized editors | planned | GRANT/RLS/index/FK/trigger UIs, array & JSON cell editors, PostGIS |

Phases 1–4 cover the three pain points named explicitly when setting the parity goal.

---

## Phase 1 — Wire the connection-level tabs — ✅ shipped
Made the `OverviewHeader` tabs real views.

What landed:
- Tables tab: flat list with Schema/Name/Kind columns on every relational engine; adds Rows + Size columns on Postgres (via `load_relation_stats`). Views included with a kind badge. Sortable headers, free-text search, transient schema filter chip set by the Schemas tab.
- Schemas tab (Postgres-only): per-schema aggregates (tables, views, materialised views, rows ≈, size) derived from the same `relationStats` cache. Click a row to jump to the Tables tab pre-filtered.
- Query History tab: persisted log scoped to the current connection by default with a "Showing all connections" toggle, free-text SQL search, success/error chips, Open-in-editor + Copy SQL row actions. Persistence cap raised 200 → 2000 in both the in-memory slice and the SQLite trim.
- Details tab (Postgres-only): summary panel (trimmed `version()`, encoding, locale, timezone) plus a `pg_settings`-backed table with category grouping, search, modified-only filter, and short-desc tooltips; installed extensions table sourced from `pg_extension`.
- Settings tab: read-only view of the connection's existing fields (name, engine, host, port, database, user, role, engine-specific TLS row) with an Edit button that opens the existing `EditConnectionDialog`. SSH tunnel, keepalive, statement-timeout, and other driver-level fields were explicitly out of scope — they need new connection-record fields and backend wiring, and are tracked as a follow-up phase.

Cross-cutting:
- New per-connection `connectionOverviewTab` state on `ConnectionsSlice` drives the sub-tab nav; the existing Overview cards' "View all" buttons jump to the matching deep tabs.
- Postgres-only sub-tabs (Schemas, Details) render a degraded explainer panel on MySQL/SQLite/ClickHouse rather than disappearing from the nav.
- Two new Tauri commands (`load_relation_stats`, `load_server_details`) — lazy on first sub-tab activation, dropped on disconnect, invalidated on DDL commit (relation stats).

## Phase 2 — Schema map overhaul — ✅ shipped
Made the schema map a durable, exportable graph surface rather than a static preview.

What landed:
- Dedicated Schema Map overview sub-tab with per-connection last-viewed schema selection and deep-link actions from the Schemas tab.
- Dagre LR auto-layout with SQLite-backed drag positions per `(connection, schema, table)` and a Reset layout action.
- Column-level FK handles, always-visible FK labels, Crow's Foot markers inferred from FK nullability, and bezier/step routing preferences.
- Attribute controls persisted per `(connection, schema)`: All / Keys-only / None plus Types, NULL, and Comments toggles.
- PNG and SVG export with normalized filenames and a light export theme.
- PostgreSQL column comments via `pg_description`; ClickHouse continues to render maps without FK edges and empty comments.

Deferred:
- Notes / annotations on canvas.
- Virtual / user-drawn relationships.
- MySQL / SQLite FK introspection.
- 1:1 cardinality detection.
- Multi-schema canvases.

Tracking: [Schema map Phase 2 deferred follow-ups](https://github.com/imran-vz/dbunk/issues/17).

## Phase 3 — Data export upgrade — ✅ shipped
Lay the foundation for a real data-transfer surface. Result-set export stays; this phase adds whole-table export and more formats.

- Whole-table export pipeline (streaming, not just current page)
- New formats: SQL INSERTs, HTML, Markdown, TXT
- XLSX (Excel) export
- Compression + encoding options
- Save export config as a re-runnable task

What landed:
- Existing result-set export now supports CSV, JSON, SQL INSERTs, HTML, Markdown, TXT, and valid XLSX downloads for all visible rows and selected rows.
- Whole-table export is wired into the table editor and pages through `load_table_data` in 1000-row chunks until the table is exhausted, so it is no longer limited to the current grid page.
- Export options include NULL token, UTF-8 / UTF-16LE encoding, and gzip compression when supported by the webview.
- Export task configs are saved to localStorage per table and can be rerun from the table export menu.

## Phase 4 — Data import wizard — ✅ shipped
Bulk inbound. Pairs naturally with Phase 3's transfer surface.

- CSV → existing table with column-mapping UI
- XLSX → table
- Header detection, date-format and NULL-token settings
- `COPY FROM`-backed fast path for large CSVs
- Multi-sheet imports

What landed:
- Table editor Import action opens a wizard for CSV and XLSX files.
- CSV parser supports quoted cells, escaped quotes, header detection, NULL-token handling, and source-to-target column mapping.
- XLSX imports support multi-sheet selection through the same mapping flow.
- Imports submit through a backend `import_rows` command in one operation; Postgres large unmapped CSV imports use `COPY FROM STDIN`, while smaller or mapped imports use transactional bulk INSERT chunks.

## Phase 5 — DDL + dump/restore — ✅ shipped
Schema-level export and full backup/restore via Postgres-native tooling.

- DDL export: single table, schema, full database
- `pg_dump` integration (plain + custom formats)
- `pg_restore` integration
- Cross-connection table-to-table copy

What landed:
- Table actions can export single-table DDL as SQL, including columns, defaults, constraints, indexes, views, and materialized views.
- Connection settings expose PostgreSQL database DDL export plus `pg_dump` downloads in plain SQL and custom formats.
- Restore accepts plain SQL through `psql` and custom dumps through `pg_restore`, with an optional clean restore toggle.
- A table-copy panel copies rows from the open table into a matching destination table on any relational connection, chunked at 1000 rows.

## Phase 6 — Object navigator depth — ✅ shipped
Expand the sidebar tree to cover the rest of Postgres's first-class object types. Each object type gets a viewer and, where applicable, an editor.

- Materialized views (+ refresh action)
- Functions, procedures, aggregate functions
- Sequences (edit / restart / next value)
- Foreign tables
- Custom types, domains
- Extensions (install/drop UI)
- Event triggers
- Per-table sub-nodes: Triggers, Rules, Policies, Partitions, Dependencies, References
- Database-level: Roles, Tablespaces

What landed:
- Schema explorer now introspects PostgreSQL materialized views, sequences, foreign tables, functions, procedures, aggregate functions, custom types, domains, installed extensions, event triggers, roles, and tablespaces.
- The left navigator renders those object groups under each schema, with search covering every object type.
- Clicking non-table objects opens an object-specific SQL/catalog viewer query; foreign tables and sequences open directly as queryable relations.
- Materialized views have a direct refresh action backed by a new `refresh_materialized_view` Tauri command.
- Table rows can expand into Triggers, Rules, Policies, Partitions, Dependencies, and References catalog viewers.
- The Tables overview fallback now includes materialized views and foreign tables from the enriched schema explorer.

## Phase 7 — Admin tools — ✅ shipped
Operational visibility that lets a Postgres user diagnose live problems without leaving dbunk.

- Session Manager (`pg_stat_activity` + terminate/cancel)
- Lock Manager (`pg_locks` blocker/blocked chains)
- Pending transactions view
- VACUUM / ANALYZE / REINDEX as table context actions
- Deeper DB stats dashboard (size growth, cache hit ratio, slow queries)

What landed:
- New Postgres-only Admin overview tab with live sessions, locks, pending transactions, and core database health metrics.
- Session rows support `pg_cancel_backend` and `pg_terminate_backend` actions, then refresh the admin snapshot.
- Lock Manager shows granted/pending locks plus blocker PID chains through `pg_blocking_pids`.
- Pending transactions are pulled from `pg_stat_activity` with transaction age and active query preview.
- Table actions now include VACUUM, ANALYZE, and REINDEX backed by `run_pg_maintenance`.
- Dashboard metrics include database size, cache hit ratio, active sessions, idle-in-transaction count, and blocked locks.

## Phase 8 — SQL editor depth
Make the editor competitive for serious query work.

- EXPLAIN / EXPLAIN ANALYZE plan visualizer
- SQL templates / snippets library
- Bind variables / parameterized executions
- Stretch: PL/pgSQL debugger
- Stretch: visual query builder

## Phase 9 — Compare + generate
Cross-cutting tools that depend on Phase 6's object coverage.

- Schema compare (two schemas → diff + migration SQL)
- Data compare (two tables → diff)
- Mock data generator

## Phase 10 — Specialized editors
Final polish — object-creation UIs and cell-level editors for Postgres-shaped data.

- Permissions / GRANT editor (per object)
- Row-Level Security policies UI
- Index creation UI (replacing DDL-only pending-changes path)
- Cross-table FK creation UI
- Trigger creation UI
- Postgres `array` cell editor
- `json` / `jsonb` tree editor
- PostGIS / geometry visualization
