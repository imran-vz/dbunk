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
| Schema relationship map (auto-layout, table nodes, FK edges) | ✅ Phase 2 | `src/components/schema-relationship-map.tsx`, `lib/schema-graph.ts` |
| Data export — CSV, JSON, SQL, HTML, Markdown, TXT, XLSX | ✅ | `src/lib/export.ts`, `src/components/data-grid.tsx` (whole-table + selection, gzip, encoding picker, NULL token, saved tasks) |
| Connection-level sub-tabs (Tables/Schemas/Query History/Details/Settings) | ✅ Phase 1 | `src/components/workspace-overview/{tables,schemas,query-history,details,settings}-tab.tsx` |
| Connection-level settings page | 🟡 | Sidebar gear-icon view + driver options end-to-end (ADR-0013); TLS modes, keepalive, and staged Test Connection on the form (ADR-0025) — the Settings tab is still a read-only mirror, see § 1 below |

---

## Postgres feature gap vs. DBeaver

Grouped by area. ❌ = absent, 🟡 = partial, ✅ = parity.

### 1. Connection-level navigation — ✅ Phase 1
- ✅ "Tables" tab — flat searchable list (degrades stats columns on non-PG)
- ✅ "Schemas" tab — per-schema table/view/matview counts + size (PG-only)
- ✅ "Query History" tab — current-connection-scoped view with search + status filters; cap raised to 2000 entries
- ✅ "Details" tab — server version, encoding, locale, timezone, `pg_settings` catalogue with modified-only filter, installed extensions (PG-only)
- 🟡 "Settings" tab — read-only mirror of the connection's configured fields (including the driver knobs) + Edit dialog launcher. Driver options are complete end-to-end (ADR-0013): `PgDriverOptions` on the connection record, the Advanced-expander form in `<ConnectionForm>`, SET-statement plumbing for `statement_timeout`, `idle_in_transaction_session_timeout`, `search_path`, `ROLE`, and a bounded initial handshake for `connect_timeout_ms`. SSH tunnel shipped separately (ADR-0018). TCP keepalive has a control and is applied on the dedicated driver — query sessions, table browse, result mutation (ADR-0025); the pooled metadata/admin driver (SQLx) has no keepalive option, and the form says so next to the field. Transport security is on the form too: libpq TLS modes with certificate paths, and Test Connection as a per-stage diagnosis (tunnel → DNS → TCP → TLS → authentication → database) available in edit mode.

### 2. Schema relationship map
- ✅ Cardinality notation (Crow's Foot)
- ✅ Column-level edge anchors with FK-column labels
- ✅ Persistent drag positions ("keep layout")
- ✅ Attribute display modes (All / Keys-only / None; toggles for types, NULL, comments)
- ✅ Image export (PNG / SVG)
- ❌ Virtual / user-drawn relationships
- ❌ Notes / annotations on canvas
- ❌ Multi-schema / custom-pick diagrams
- ✅ Routing choice (bezier vs orthogonal)

Deferred schema-map items are tracked in [GitHub issue #17](https://github.com/imran-vz/dbunk/issues/17).

### 3. Object types in navigator
Today: schemas → tables + views + the items below.

- ✅ Materialized views (+ refresh action via `refresh_materialized_view`)
- ✅ Foreign tables
- ✅ Functions, procedures, aggregate functions (open with `pg_get_functiondef`)
- ✅ Sequences — browse + per-item actions (Inspect, Advance via `nextval`, Set value, Restart from) via sidebar dropdown
- ✅ Custom types, domains
- ✅ Event triggers
- ✅ Extensions (browse + open definition; install/drop UI not wired)
- ✅ Per-table sub-nodes: Triggers, Rules, Policies, Partitions, Dependencies, References
- ✅ Database-level: Roles / users, Tablespaces

### 4. Administration tools
- ✅ Session Manager — `pg_stat_activity` with terminate/cancel (`admin-tab.tsx`)
- ✅ Lock Manager — `pg_locks` with blocker/blocked chains
- ✅ Pending transactions view
- ✅ VACUUM / ANALYZE / REINDEX as table-header actions (`run_pg_maintenance`)
- 🟡 Deeper database statistics — cache-hit, idle-in-transaction, basic metrics shipped; size growth and slow-query digest are still pending

### 5. SQL editor depth
- ✅ Basic SQL completion (`src/lib/sql-completions.ts`)
- ✅ EXPLAIN / EXPLAIN ANALYZE plan visualizer (tree + cost, JSON + text)
- ✅ SQL templates / snippets library (toolbar dropdown)
- ✅ Bind variables / parameterized executions (`src/lib/bind-variables.ts`)
- ❌ PL/pgSQL debugger (breakpoints, step, variable inspect)
- ❌ Visual query builder
- ✅ SQL formatting (Format toolbar button + Cmd/Ctrl+Shift+F via `sql-formatter`)

### 6. Data export
Have: per-table or selection export through the data grid.

- ✅ Whole-table export (paginates `load_table_data`)
- ✅ SQL INSERTs export
- ✅ DDL export — table, schema, or database (`export_ddl`)
- ✅ HTML / Markdown / TXT
- ✅ XLSX (Excel) — hand-rolled single-sheet writer
- ❌ XML
- ❌ Parquet
- ✅ Table-to-table copy (`copy_table_rows` over `import_rows`)
- ✅ Save export config as re-runnable task (`export-tasks.ts`)
- 🟡 File-backed PostgreSQL backup/restore backend: typed cancellable jobs, safe archive publication, restore policy, and teardown fencing (Plan 018). Dark until Plan 019 adds UI activation.
- 🟡 Compression / split / encoding — gzip + encoding picker + NULL token shipped; split-file not yet

### 7. Data import
Have: CSV / XLSX import wizard with column mapping.

- ✅ CSV → existing table with column-mapping wizard
- ✅ XLSX → table (uses the `xlsx` package)
- ❌ XML → table
- 🟡 Multi-sheet imports, header detection, date-format / NULL-token configuration — XLSX picks the first sheet today; richer multi-sheet + date-format options pending
- ✅ `COPY FROM` streaming (Postgres fast-path bulk load)

### 8. Other Postgres-shaped tooling
- ✅ Create table designer — live typed preview and reviewed apply
- ✅ Function / procedure editor — typed reviewed create and edit flows in the navigator and Object Viewer
- ✅ Permissions / GRANT editor — typed reviewed Structure and Specialized flows
- ✅ Row-Level Security policies UI — typed reviewed Structure and Specialized flows
- ✅ Index creation UI — Structure and Specialized panels queue typed reviewed operations
- ✅ Cross-table FK creation UI — Structure and Specialized panels queue typed reviewed operations
- ✅ Trigger creation UI — typed reviewed Structure and Specialized flows
- ✅ Postgres `array` cell editor — list editor in the data grid; writes a PG array literal back into pending edits
- ✅ `json` / `jsonb` tree editor — modal editor with `JSON.parse` validation + Pretty print; writes into pending edits
- ✅ PostGIS / geometry visualization — WKT modal editor with sanity validation; writes into pending edits
- ✅ Schema compare (two schemas → diff)
- ✅ Data compare (two tables → diff, sampled)
- ✅ Mock data generator (column-aware INSERTs from real `tableStructure`)

---

## Queued work

Feature-scope items not covered by the gap sections above (absorbed from the
retired `docs/PENDING_TASKS.md`; deep detail lives in `designs/FOLLOWUPS.md`
and the ADRs cited). Treat each bullet as the seed for its own design pass.

### Postgres / relational

- **Default-value tagged-union, remaining engines** — PostgreSQL delivered
  the tagged `PgDefaultValue` (Literal/Expression) in Plan 015; the
  ClickHouse (and dead-end MySQL/SQLite) column path still runs defaults
  through the `formatDefault` bareword-whitelist heuristic in
  `src/lib/ddl/shared.ts`.
- **Connection Settings tab expansion** — `settings-tab.tsx` is a read-only
  mirror; expand it to edit the driver/session knobs the connection form
  already exposes (ADR-0013, ADR-0025).
- **PL/pgSQL debugger** (ADR-0016, unbuilt), **visual query builder**
  (ADR-0015, unbuilt), **Parquet export / XML import-export** (ADR-0017,
  unbuilt) — see the ❌ rows in the gap sections above.

### Redis Tier 2 + cross-cutting

- **Cross-tab DB switching cascade** — key tabs still open on the
  connection's default DB; re-key open key tabs or reuse the scan session's
  connection.
- **Sentinel discovery + Cluster awareness** — form changes, dispatch
  routing, slot-aware command routing.
- **Module viewers** — RediSearch, RedisTimeSeries, RedisBloom.
- **Advanced tab kinds** — Transaction Builder, Lua / Redis Functions
  scripting, MONITOR capture; parameter substitution for saved commands;
  non-string multi-key compare.
- **Geo static-map rendering.**

### Cross-cutting UX

- **Empty / loading / error state polish** — surface-specific empty states,
  skeleton loaders, inline error + retry on every data-bound view.
- **Query editor transaction status footer** — per-connection transaction
  state surfaced as `Auto-commit ON / In transaction / Failed transaction`
  with commit/rollback controls.
- **Reserved store slices** — `keyvalue-pubsub.ts` (no-op until Pub/Sub
  auto-reconnect) and parts of `keyvalue-workspace.ts` (watched keys,
  per-session DB switcher) are documented placeholders awaiting their
  features.

## Phased implementation

The gap above is sequenced into ten phases in [`docs/design/PHASES.md`](docs/design/PHASES.md). Each phase is a discrete deliverable that gets planned individually before it ships. Phases 1–4 cover the user-facing pain points that motivated this roadmap (dead overview tabs, weak schema map, missing import/export).
