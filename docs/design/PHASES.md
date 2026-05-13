# Postgres parity — implementation phases

Sequencing plan for the DBeaver–Postgres parity goal described in [`ROADMAP.md`](../../ROADMAP.md). Each phase is a discrete deliverable that gets planned individually before it ships. Sequencing inside a phase is decided when that phase is planned.

Phases are ordered by user-facing pain first, then by dependency.

## Summary

| # | Phase | Why this slot |
|---|---|---|
| 1 | Wire the connection-level tabs | Smallest scope, biggest perceived completeness win; most views are thin wrappers over data the store already has |
| 2 | Schema map overhaul | Directly addresses the "schema map is really not great" pain |
| 3 | Data export upgrade | First half of the import/export pain — lay the transfer foundation |
| 4 | Data import wizard | Second half — bulk inbound; pairs with Phase 3's transfer surface |
| 5 | DDL + dump/restore | Schema-level export and full `pg_dump`/`pg_restore` backups |
| 6 | Object navigator depth | Materialized views, functions, sequences, extensions, roles, tablespaces, etc. |
| 7 | Admin tools | Sessions, locks, pending transactions, VACUUM/ANALYZE actions |
| 8 | SQL editor depth | EXPLAIN visualizer, snippets, bind vars (debugger as stretch) |
| 9 | Compare + generate | Schema compare, data compare, mock data — depends on Phase 6 coverage |
| 10 | Specialized editors | GRANT/RLS/index/FK/trigger UIs, array & JSON cell editors, PostGIS |

Phases 1–4 cover the three pain points named explicitly when setting the parity goal.

---

## Phase 1 — Wire the connection-level tabs
Make the existing `OverviewHeader` tabs real views. Smallest scope, biggest perceived completeness win, and several views are thin wrappers around data the store already holds.

- Tables tab: flat searchable list of all tables in the connection
- Schemas tab: schema list with object counts and sizes
- Query History tab: dedicated view backed by existing `queryHistory`
- Details tab: server version, encoding, locale, timezone, extensions, `SHOW ALL`
- Settings tab: read-only view of the connection's existing fields (host, port, database, user, role, engine-specific TLS), with an Edit button that opens the existing `EditConnectionDialog`. SSH tunnel, keepalive, statement-timeout, and other driver-level fields are explicitly **out of scope** for Phase 1 — they need new connection-record fields and backend wiring, and are tracked as a follow-up phase.

## Phase 2 — Schema map overhaul
Address the explicit "schema map is not great" pain.

- Crow's Foot cardinality notation on FK edges
- Column-level handles + `source.col → target.col` edge labels
- Persistent drag positions per (connection, schema)
- Attribute display modes (All / Keys-only / None; toggles for type, NULL, comment)
- Image export (PNG, SVG)
- Routing choice (orthogonal as alternative to default)
- Stretch: notes/annotations on canvas

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
