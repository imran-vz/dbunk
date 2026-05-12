# dbunk — Pre-Alpha Social Launch

Drafts for the first public announcement of dbunk. All copy mentions pre-alpha
status explicitly. Update version numbers and the release link before posting.

## Twitter / X

### Option A — Single post

> Building **dbunk** — an open-source desktop database workspace.
>
> Postgres, MySQL, SQLite, ClickHouse, Redis. One app. Local. Fast.
>
> SQL editor with IntelliSense, schema maps, table editing, keyspace browser, pub/sub monitor.
>
> ⚠️ Pre-alpha. Heavy development. Rough edges guaranteed.
>
> Built with Tauri + Rust + React.

### Option B — Thread

1/ I've been building **dbunk** — an open-source desktop database client. One app for Postgres, MySQL, SQLite, ClickHouse, and Redis.
   ⚠️ Pre-alpha — still in heavy development. Expect breakage.

2/ Connections sidebar with live status + latency. Foreground health checks every 30s so you know what's alive before you fire a query.

3/ Monaco-powered SQL editor with IntelliSense for tables, views, columns. Run the whole buffer, current statement, or just a selection.

4/ Schema explorer + schema maps — foreign-key graph rendered visually. Drill from a table into its relationships.

5/ Edit table data inline when the backend can identify rows safely. Buffered cell edits, committed in a transaction. ClickHouse mutations tracked to terminal state.

6/ Redis isn't a SQL afterthought — full keyspace browser (lazy prefix tree + SCAN), key inspector with TTL/encoding/refcount, CLI REPL, pub/sub monitor with capture-to-file.

7/ Tauri + Rust backend, React + TypeScript frontend, Bun for tooling. MIT licensed. Contributions welcome — bug reports especially while it's this raw.
   👉 github.com/imran-vz/dbunk

## LinkedIn

> I've been quietly building **dbunk** — an open-source desktop database workspace.
>
> The pitch: one fast, local app for exploring data, running SQL, inspecting schemas, and editing rows across **PostgreSQL, MySQL, SQLite, ClickHouse, and Redis**.
>
> What's in it today:
> • Connections manager with live status + latency
> • Monaco-powered SQL editor with IntelliSense
> • Schema explorer, schema maps, table-structure inspector
> • In-place table editing where row identity can be resolved safely
> • Redis keyspace browser, key inspector, CLI, and pub/sub monitor
> • Database overview dashboards per engine
>
> The stack: Tauri + Rust on the backend, React + TypeScript + Monaco on the frontend, Bun for tooling. Local-first — your data and credentials stay on your machine (encrypted SQLite, OS keychain, or plaintext with warning — your choice).
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
   - **Relational overview** (Postgres ideally — richest data)
   - **Redis overview** (server identity, memory, clients, keyspace counts)
2. **Connections list / sidebar** — connections with live status badges (Connected / Read only / Disconnected) and latency.
3. **New connection dialog** — show that all 5 engines are pickable. Engine-specific fields visible.
4. **SQL editor with IntelliSense open** — autocomplete dropdown showing table or column suggestions mid-query. The money shot.
5. **Query results grid** — a query that returned a few hundred rows, runtime + row count visible in the footer.
6. **Schema explorer expanded** — sidebar tree with schemas → tables → views drilled open.
7. **Schema map** — foreign-key graph rendered. Pick a schema with a few related tables so the edges are legible.
8. **Table structure inspector** — columns, primary key, indexes, constraints, relationships tab.
9. **Inline cell edit** — a cell mid-edit with the dirty/buffered state visible, plus the Commit button.
10. **Redis keyspace browser** — prefix tree expanded (e.g. `user:*`, `session:*`), type-filter chips visible.
11. **Redis key inspector** — open key with the value panel + right-side metadata drawer (type, TTL, encoding, refcount).
12. **Redis CLI tab** — a few commands run, history visible, ideally with a destructive-command warning shown.
13. **Redis pub/sub monitor** — active subscription with a few messages streaming in.
14. **Query history panel** — recent queries with status + runtime, to show persistence across sessions.
15. *(Optional)* **Settings → credentials backend picker** — to subtly signal the security/privacy story (keychain / encrypted SQLite / plaintext).

## Suggested mapping

- **Twitter single post:** #1 (relational overview) + #4 (IntelliSense) + #7 (schema map) + #11 (Redis key inspector) — four-image grid.
- **Twitter thread:** one screenshot per numbered tweet, in order.
- **LinkedIn:** carousel of #1, #4, #7, #10, #11, #13 — leads with the SQL story, lands on the Redis story to differentiate from generic DB clients.
