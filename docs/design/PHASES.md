# Postgres parity — implementation phases

Sequencing plan for the DBeaver–Postgres parity goal described in [`ROADMAP.md`](../../ROADMAP.md). Each phase is a discrete deliverable that gets planned individually before it ships. Sequencing inside a phase is decided when that phase is planned.

Phases are ordered by user-facing pain first, then by dependency.

## Summary

| # | Phase | Status | Why this slot |
|---|---|---|---|
| 1 | Wire the connection-level tabs | ✅ shipped | Smallest scope, biggest perceived completeness win; most views are thin wrappers over data the store already has |
| 2 | Schema map overhaul | ✅ shipped | Directly addresses the "schema map is really not great" pain |
| 3 | Data export upgrade | planned | First half of the import/export pain — lay the transfer foundation |
| 4 | Data import wizard | planned | Second half — bulk inbound; pairs with Phase 3's transfer surface |
| 5 | DDL + dump/restore | planned | Schema-level export and full `pg_dump`/`pg_restore` backups |
| 6 | Object navigator depth | planned | Materialized views, functions, sequences, extensions, roles, tablespaces, etc. |
| 7 | Admin tools | planned | Sessions, locks, pending transactions, VACUUM/ANALYZE actions |
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

## Phase 3 — Data export upgrade
Lay the foundation for a real data-transfer surface. Result-set export stays; this phase adds whole-table export and more formats.

- Whole-table export pipeline (streaming, not just current page)
- New formats: SQL INSERTs, HTML, Markdown, TXT
- XLSX (Excel) export
- Compression + encoding options
- Save export config as a re-runnable task

## Phase 4 — Data import wizard
Bulk inbound. Pairs naturally with Phase 3's transfer surface.

- CSV → existing table with column-mapping UI
- XLSX → table
- Header detection, date-format and NULL-token settings
- `COPY FROM`-backed fast path for large CSVs
- Multi-sheet imports

## Phase 5 — DDL + dump/restore
Schema-level export and full backup/restore via Postgres-native tooling.

- DDL export: single table, schema, full database
- `pg_dump` integration (plain + custom formats)
- `pg_restore` integration
- Cross-connection table-to-table copy

## Phase 6 — Object navigator depth
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

## Phase 7 — Admin tools
Operational visibility that lets a Postgres user diagnose live problems without leaving dbunk.

- Session Manager (`pg_stat_activity` + terminate/cancel)
- Lock Manager (`pg_locks` blocker/blocked chains)
- Pending transactions view
- VACUUM / ANALYZE / REINDEX as table context actions
- Deeper DB stats dashboard (size growth, cache hit ratio, slow queries)

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
