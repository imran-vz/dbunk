# dbunk — Pre-Alpha Social Launch

Drafts for the first public announcement of dbunk. All copy mentions pre-alpha
status explicitly. Update version numbers and the release link before posting.

## Twitter / X

### Option A — Single post

> Building **dbunk** — an open-source desktop database workspace.
>
> Postgres, MySQL, SQLite, ClickHouse, Redis. One app. Local. Fast.
>
> SQL editor with IntelliSense, EXPLAIN-plan analysis, schema maps, FK drill-down, table editing, Redis keyspace browser, pub/sub monitor.
>
> ⚠️ Pre-alpha. Heavy development. Rough edges guaranteed.
>
> Built with Tauri + Rust + React.

### Option B — Thread

1/ I've been building **dbunk** — an open-source desktop database client. One app for Postgres, MySQL, SQLite, ClickHouse, and Redis.
   ⚠️ Pre-alpha — still in heavy development. Expect breakage.

2/ Connections sidebar with live status + latency. Foreground health checks every 30s so you know what's alive before you fire a query.

3/ Monaco-powered SQL editor with IntelliSense for tables, views, columns. Run the whole buffer, current statement, or just a selection.

4/ Schema explorer + schema maps — foreign-key graph rendered visually. Drill from a table into its relationships, or follow FKs inline as a mini-table right under the row you clicked.

5/ EXPLAIN analysis built in. Visual plan tree, self-time bars, and warnings for plan-vs-actual overestimates, sequential scans, and planning overhead — no copy-paste into explain.dalibo.com.

6/ Edit table data inline when the backend can identify rows safely. Buffered cell edits, committed in a transaction. ClickHouse mutations tracked to terminal state.

7/ Redis isn't a SQL afterthought — full keyspace browser (lazy prefix tree + SCAN), key inspector with TTL/encoding/refcount, CLI REPL, pub/sub monitor with capture-to-file.

8/ Tauri + Rust backend, React + TypeScript frontend, pnpm for tooling. MIT licensed. Contributions welcome — bug reports especially while it's this raw.
   👉 github.com/imran-vz/dbunk

## LinkedIn

> I've been quietly building **dbunk** — an open-source desktop database workspace.
>
> The pitch: one fast, local app for exploring data, running SQL, inspecting schemas, and editing rows across **PostgreSQL, MySQL, SQLite, ClickHouse, and Redis**.
>
> What's in it today:
> • Connections manager with live status + latency
> • Monaco-powered SQL editor with IntelliSense
> • EXPLAIN-plan viewer with self-time bars and bottleneck warnings
> • Schema explorer, schema maps, table-structure inspector
> • Inline foreign-key drill-down — follow relationships without losing context
> • In-place table editing where row identity can be resolved safely
> • Searchable query history scoped per connection
> • Redis keyspace browser, key inspector, CLI, and pub/sub monitor
> • Database overview dashboards per engine
>
> The stack: Tauri + Rust on the backend, React + TypeScript + Monaco on the frontend, pnpm for tooling. Local-first — your data and credentials stay on your machine (encrypted SQLite, OS keychain, or plaintext with warning — your choice).
>
> ⚠️ **Important caveat: dbunk is pre-alpha and still in heavy development.** Expect rough edges, missing features, and breaking changes. I'm sharing it now because feedback at this stage is the most valuable kind.
>
> If you work with multiple databases day-to-day, I'd love your eyes on it. Bug reports, feature ideas, and "I would never use it unless it did X" comments are all welcome.
>
> Repo + contributing guide: github.com/imran-vz/dbunk

## Screenshots to capture

Some of these already exist in `marketing/` and `designs/images/`. Re-take any
that have drifted from the current UI before posting.

1. **Hero / overview dashboard** — landing view after picking a connection.
   - **Relational overview** (Postgres ideally — richest data) ✅ `images/pg-overview.jpeg`
   - **Redis overview** (server identity, memory, clients, keyspace counts) — needed
2. **Connections list / sidebar** — connections with live status badges (Connected / Read only / Disconnected) and latency. ✅ visible in `images/pg-overview.jpeg`
3. **New connection dialog** — show that all 5 engines are pickable. Engine-specific fields visible. ✅ `images/connections.jpeg`
4. **SQL editor with IntelliSense open** — autocomplete dropdown showing table or column suggestions mid-query. The money shot. — partial: `images/pg-sql-editor.jpeg` (no IntelliSense popover; re-shoot mid-completion)
5. **Query results grid** — a query that returned a few hundred rows, runtime + row count visible in the footer. ✅ `images/pg-sql-editor.jpeg`
6. **Schema explorer expanded** — sidebar tree with schemas → tables → views drilled open. ✅ visible in `images/pg-table-view-page.jpeg`
7. **Schema map** — foreign-key graph rendered. Pick a schema with a few related tables so the edges are legible. ✅ `images/schema-map.jpeg`
8. **Table structure inspector** — columns, primary key, indexes, constraints, relationships tab. ✅ `images/pg-table-view-page.jpeg`
9. **Inline cell edit** — a cell mid-edit with the dirty/buffered state visible, plus the Commit button. — needed
10. **Redis keyspace browser** — prefix tree expanded (e.g. `user:*`, `session:*`), type-filter chips visible. ✅ `images/redis-home.jpeg`
11. **Redis key inspector** — open key with the value panel + right-side metadata drawer (type, TTL, encoding, refcount). ✅ `images/redis-hash-view.jpeg`
12. **Redis CLI tab** — a few commands run, history visible, ideally with a destructive-command warning shown. — needed
13. **Redis pub/sub monitor** — active subscription with a few messages streaming in. — needed
14. **Query history panel** — recent queries with status + runtime, to show persistence across sessions. ✅ `images/pg-query-history.jpeg`
15. *(Optional)* **Settings → credentials backend picker** — to subtly signal the security/privacy story (keychain / encrypted SQLite / plaintext). ✅ `images/security-page.jpeg`
16. **EXPLAIN plan viewer** — plan tree with self-time bars and at least one orange warning (overestimate / seq scan). ✅ `images/explain-page.jpeg`
17. **FK drill-down** — inline mini-table rendered under a clicked row showing the resolved foreign-key target. ✅ `images/pg-table-fk-dig-view.jpeg`
18. *(Optional)* **Theme presets** — settings page with Dracula / GitHub / Gruvbox swatches. ✅ `images/theme-selection-page.jpeg`

## Suggested mapping

- **Twitter single post:** #1 (relational overview) + #16 (EXPLAIN viewer) + #17 (FK drill-down) + #11 (Redis key inspector) — four-image grid that hits both the SQL-power and Redis-breadth angles.
- **Twitter thread:** one screenshot per numbered tweet, in order. Thread numbering above already aligns: 4 → #17 (FK drill-down), 5 → #16 (EXPLAIN), 7 → #10/#11 (Redis).
- **LinkedIn:** carousel of #1, #4, #16, #17, #7, #10, #11, #14 — leads with the SQL story, peaks on EXPLAIN + FK drill-down (the new stuff), lands on the Redis story to differentiate from generic DB clients.
