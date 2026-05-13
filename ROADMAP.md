# Roadmap

## North star

**DBeaver–Postgres parity.** Postgres is dbunk's flagship engine. Until a Postgres user can stop reaching for DBeaver, no other engine work takes priority over closing this gap.

ClickHouse and Redis remain supported in their current shape; new feature investments funnel into Postgres first and then propagate to the other engines where the engine-policy abstraction allows.

---

## Where we are today (Postgres baseline)

What already works:

| Area | Status | Where |
|---|---|---|
| Sidebar: connections + schemas + tables/views tree | ✅ | `src/components/sidebar.tsx` |
| Workspace overview (stats, recent queries, favorite tables, health banner) | ✅ | `src/components/workspace-overview/*` |
| Query editor + results grid | ✅ | `src/components/query-editor*`, `query-editor/*` |
| Table editor (browse rows, inline cell commit, insert/delete, pending mutations) | ✅ | `src/components/table-editor*`, `src-tauri/src/postgres.rs` |
| Table structure sub-tab (columns, indexes, FKs, constraints, pending DDL) | ✅ | `src/components/table-structure/*`, `fetch_table_structure` |
| Schema relationship map (auto-layout, table nodes, FK edges) | 🟡 partial | `src/components/schema-relationship-map.tsx`, `lib/schema-graph.ts` |
| Row export — CSV, JSON | 🟡 minimal | `src/lib/export.ts` (current result rows only) |
| Connection-level sub-tabs (Tables/Schemas/Query History/Details/Settings) | ❌ stub only | `overview-header.tsx` — static buttons, no click handler |
| Connection-level settings page | ❌ | Sidebar gear-icon view exists but exposes no Postgres knobs |

---

## Postgres feature gap vs. DBeaver

Grouped by area. ❌ = absent, 🟡 = partial, ✅ = parity.

### 1. Connection-level navigation
- ❌ "Tables" tab — flat searchable list across the DB
- ❌ "Schemas" tab — schema list with object counts/sizes
- ❌ "Query History" tab — `queryHistory` exists in store, no dedicated view
- ❌ "Details" tab — server version, encoding, locale, timezone, installed extensions, `SHOW ALL` settings
- ❌ "Settings" tab — per-connection driver/SSL/SSH tunnel/keepalive/timeout/role

### 2. Schema relationship map
- ❌ Cardinality notation (Crow's Foot / IDEF1X / Bachman)
- ❌ Column-level edge anchors with `source.col → target.col` labels (currently table-box level only)
- ❌ Persistent drag positions ("keep layout")
- ❌ Attribute display modes (All / Keys-only / PK-only / None; toggles for types, NULL, comments)
- ❌ Image export (PNG / SVG / GraphML)
- ❌ Virtual / user-drawn relationships
- ❌ Notes / annotations on canvas
- ❌ Multi-schema / custom-pick diagrams
- ❌ Routing choice (shortest path vs orthogonal)

### 3. Object types in navigator
Today: schemas → tables + views.

- ❌ Materialized views (+ refresh action)
- ❌ Foreign tables
- ❌ Functions, procedures, aggregate functions
- ❌ Sequences (edit / restart / next value)
- ❌ Custom types, domains
- ❌ Event triggers
- ❌ Extensions (with install/drop UI)
- ❌ Per-table sub-nodes: Triggers, Rules, Policies, Partitions, Dependencies, References
- ❌ Database-level: Roles / users, Tablespaces

### 4. Administration tools
- ❌ Session Manager — `pg_stat_activity` with terminate/cancel
- ❌ Lock Manager — `pg_locks` with blocker/blocked chains
- ❌ Pending transactions view
- ❌ VACUUM / ANALYZE / REINDEX as context-menu actions
- ❌ Deeper database statistics (size growth, cache hit ratio, slow queries)

### 5. SQL editor depth
- ✅ Basic SQL completion (`src/lib/sql-completions.ts`)
- ❌ EXPLAIN / EXPLAIN ANALYZE plan visualizer (tree + cost)
- ❌ PL/pgSQL debugger (breakpoints, step, variable inspect)
- ❌ SQL templates / snippets library
- ❌ Bind variables / parameterized executions
- ❌ Visual query builder

### 6. Data export
Have: result-set rows → CSV, JSON.

- ❌ Whole-table export (not just current page)
- ❌ SQL INSERTs export (with native date format option)
- ❌ DDL export — table, schema, or database
- ❌ HTML / XML / Markdown / TXT
- ❌ XLSX (Excel)
- ❌ Parquet
- ❌ Table-to-table copy (within / across connections)
- ❌ Save export config as re-runnable task
- ❌ `pg_dump` / `pg_restore` integration
- ❌ Compression / split / encoding options

### 7. Data import
Have: nothing beyond single-row inserts in the table editor.

- ❌ CSV → existing table with column-mapping wizard
- ❌ XLSX → table
- ❌ XML → table
- ❌ Multi-sheet imports, header detection, date-format / NULL-token configuration
- ❌ `COPY FROM` streaming (Postgres fast-path bulk load)

### 8. Other Postgres-shaped tooling
- ❌ Permissions / GRANT editor (per object)
- ❌ Row-Level Security policies UI
- ❌ Index creation UI (currently DDL-only via pending changes)
- ❌ Cross-table FK creation UI
- ❌ Trigger creation UI
- ❌ Postgres `array` cell editor
- ❌ `json` / `jsonb` tree editor
- ❌ PostGIS / geometry visualization
- ❌ Schema compare (two schemas → diff + migration SQL)
- ❌ Data compare (two tables → diff)
- ❌ Mock data generator

---

## Phased implementation

The gap above is sequenced into ten phases in [`docs/design/PHASES.md`](docs/design/PHASES.md). Each phase is a discrete deliverable that gets planned individually before it ships. Phases 1–4 cover the user-facing pain points that motivated this roadmap (dead overview tabs, weak schema map, missing import/export).
