# Plan 031: Native redesign and multi-engine workspace

- Status: IN PROGRESS. Requested by Imran on 2026-10-03 together with the hard
  migration (ADR-0033). Supersedes the visual direction of Plans 027–029; their
  behavioral contracts (bounded resources, exact-save recovery, owned workers,
  stored policy, no automatic retry) still apply.
- Design: [interactive mock](./mocks/native-redesign/index.html). Decisions:
  environment signal = **frame + tint**; projects are a **new connection field**
  (folders stay as optional sub-grouping).

## Requirements (from the request)

- Developer-tool density: UI text and icons around 11 px, monospace data.
- Sidebar, top third: macOS traffic lights, collapse button and logo on one
  row; connection search; connections grouped by project and environment, with
  fast switching between projects and environments.
- Sidebar, below: collapsible object tree for the selected connection, shaped
  by engine (PostgreSQL/MySQL schemas with tables, views and other objects;
  ClickHouse databases with tables, views, materialized views and dictionaries;
  Redis logical databases with keys grouped by data type).
- Separate top bar with tabs for the current connection (table data, query,
  structure, tools, …); the workspace fills the rest.
- Environment indicator across the whole window (frame + tint), production
  warning strip.
- Collapsible bottom status bar: connected database, last query latency,
  connection active state.
- Redis workspace starts as a console view.

## Steps

1. **Shell** – theme tokens and a small widget kit at 11 px; transparent
   titlebar with traffic lights in the sidebar; new sidebar, tab bar, status bar,
   environment frame/tint and production strip. Existing documents render inside.
2. **Projects** – `project` on connection organization (storage migration,
   native connection API, form field), connection list grouped by project and
   environment, project/environment switcher and shortcuts.
3. **Restyle documents** – query, table, structure, Objects and the Tool tabs
   adopt the kit; tool launchers move into the tab bar menu and Cmd-K.
4. **Engines** – native connections, object trees and documents for MySQL,
   SQLite and ClickHouse; Redis keyspace tree and console.
5. **Verification** – focused tests per step; window/keyboard/IME acceptance
   when automation is available.
