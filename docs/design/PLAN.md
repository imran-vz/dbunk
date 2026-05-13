# Phase 2 — Schema map overhaul — implementation plan

This document breaks the Phase 2 brief from [`PHASES.md`](./PHASES.md) into five sequenced sub-phases that ship linearly on `main`. Each sub-phase is independently revertible and gated on `bun format && bun lint && bun typecheck && bun run test && cargo check`.

The decisions captured below came out of a grilling session — they are the answers, not the questions. If the rationale isn't obvious from the spec, the [Decision log](#decision-log) at the bottom records the trade-off.

---

## Progress

**Status:** ✅ shipped on 2026-05-13.

| Step | Status | Notes |
|---|---|---|
| 1 — Shell + infrastructure | ✅ shipped | Schema Map sub-tab, dagre layout, persisted positions/prefs, reset layout, and Tauri storage commands landed. |
| 2 — Column-level handles + edge labels | ✅ shipped | FK edges anchor to column rows and always show column-pair labels. |
| 3 — Crow's Foot cardinality | ✅ shipped | FK optionality renders with Crow's Foot markers inferred from nullability. |
| 4 — Routing + attribute prefs + comments | ✅ shipped | Toolbar prefs persist per `(connection, schema)` and Postgres column comments flow through `pg_description`. |
| 5 — Image export | ✅ shipped | PNG/SVG export uses `html-to-image`, safe filenames, and a light export theme. |
| Wrap-up | ✅ shipped | `ROADMAP.md` and `PHASES.md` were updated; deferred work is tracked in [GitHub issue #17](https://github.com/imran-vz/dbunk/issues/17). |

Completion gates last run successfully: `bun format`, `bun lint`, `bun typecheck`, `bun run test -- --silent`, `cargo check`, `cargo test`, and `bun build:vite`.

---

## Scope envelope

### What ships in Phase 2

- Dedicated **Schema map** sub-tab in the connection overview, with a schema picker.
- **Crow's Foot** cardinality markers on FK edges.
- **Column-level handles** + always-on `(a, b) → (x, y)` edge labels.
- **Persistent drag positions** per `(connection, schema)`, SQLite-backed.
- **Attribute display modes** (All / Keys-only / None) + independent toggles (Types / NULL / Comments).
- **Image export** — PNG and SVG.
- **Routing toggle** — bezier (default) vs sharp orthogonal (`step`).
- **dagre** auto-layout as the initial / Reset-to layout.
- **Embedded preview** on the table-editor structure tab + **fullscreen overlay** retained.

### Engine matrix

| Feature | PG | ClickHouse | MySQL / SQLite |
|---|---|---|---|
| Map renders | ✅ | ✅ (with no-FK banner today) | unchanged — empty (separate roadmap entry) |
| Drag positions, routing, export, attribute modes | ✅ | ✅ | n/a |
| Crow's Foot, column handles | ✅ | n/a (no FKs) | n/a |
| Column comments toggle | ✅ | renders empty | n/a |

### Out of scope (deferred)

- Multi-schema canvases.
- IDEF1X / Bachman notation toggles.
- 1:1 cardinality detection (UNIQUE on FK).
- Notes / annotations on canvas.
- MySQL / SQLite FK introspection backend (separate roadmap entry).
- "Save view as / Save full graph as" submenu — export is fit-bounds only.

---

## Cross-cutting infrastructure

### SQLite tables (added in Step 1)

```sql
CREATE TABLE schema_map_positions (
  connection_id TEXT NOT NULL,
  schema        TEXT NOT NULL,
  table_id      TEXT NOT NULL,      -- `${schema}.${name}`
  x             REAL NOT NULL,
  y             REAL NOT NULL,
  updated_at    TEXT NOT NULL,
  PRIMARY KEY (connection_id, schema, table_id)
);

CREATE TABLE schema_map_prefs (
  connection_id  TEXT    NOT NULL,
  schema         TEXT    NOT NULL,
  routing        TEXT    NOT NULL DEFAULT 'bezier',    -- 'bezier' | 'step'
  attr_mode      TEXT    NOT NULL DEFAULT 'all',        -- 'all' | 'keys-only' | 'none'
  show_types     INTEGER NOT NULL DEFAULT 1,
  show_nulls     INTEGER NOT NULL DEFAULT 0,
  show_comments  INTEGER NOT NULL DEFAULT 0,
  updated_at     TEXT    NOT NULL,
  PRIMARY KEY (connection_id, schema)
);
```

Both tables cascade-clean on `delete_connection`. Orphan rows (schema or table gone) are tolerated; the frontend just doesn't apply overrides for ids it can't resolve.

### Store additions

- `ConnectionsSlice`
  - `connectionSchemaMapSchema: Record<connectionId, string>` — last-viewed schema per connection. Lifecycle mirrors `connectionOverviewTab`.
  - `setConnectionSchemaMapSchema(connectionId, schema)`
- `RelationalTablesSlice`
  - `schemaMapPositions: Record<connectionId, Record<schema, Record<tableId, {x, y}>>>`
  - `schemaMapPositionsStatus: Record<connectionId, Record<schema, LoadingStatus>>`
  - `loadSchemaMapPositions(connectionId, schema)`
  - `saveSchemaMapPosition(connectionId, schema, tableId, x, y)`
  - `resetSchemaMapPositions(connectionId, schema)`
  - `schemaMapPrefs: Record<connectionId, Record<schema, SchemaMapPrefs>>`
  - `schemaMapPrefsStatus: Record<connectionId, Record<schema, LoadingStatus>>`
  - `loadSchemaMapPrefs(connectionId, schema)`
  - `setSchemaMapPref(connectionId, schema, patch)`
  - All four maps dropped in `dropRelationalCachesForConnection`.

### New Tauri commands (Step 1)

- `load_schema_map_positions(connection_id, schema) -> Vec<PositionRow>`
- `save_schema_map_position(connection_id, schema, table_id, x, y)`
- `reset_schema_map_positions(connection_id, schema)`
- `load_schema_map_prefs(connection_id, schema) -> SchemaMapPrefs`
- `save_schema_map_prefs(connection_id, schema, patch)`

All are SQLite reads/writes; no engine dispatch. Backend extension for column comments (`pg_description`) lands in Step 4, not here.

---

## Step 1 — Shell + infrastructure — ✅ shipped

**Goal:** all the moving parts in place, no new visual features. Dagre replaces the grid; the new sub-tab renders with the existing node + edge style.

### In scope

- Add `"schema-map"` to `OverviewTabId` union and `OVERVIEW_TABS` in `overview-header.tsx`.
- New `SchemaMapTab` component (`src/components/workspace-overview/schema-map-tab.tsx`).
  - Toolbar: schema picker dropdown only (other toolbar controls land in later steps).
  - Body: `SchemaRelationshipMap` rendered against the picked schema.
  - Default schema selection: persisted in `connectionSchemaMapSchema`; on first visit, prefer `public`, else first schema alphabetically from `schemaExplorer`.
- Wire the Phase 1 Schemas sub-tab to deep-link in: clicking a schema's "View map" action (new icon button on each row) sets `connectionSchemaMapSchema` + switches to the schema-map sub-tab.
- Add **dagre** as a dependency. Replace the grid layout in `buildSchemaGraph`:
  - Direction `LR`, rank-sep 120, node-sep 60.
  - Compute fresh on every render unless a persisted position exists.
  - New tables (no persisted position) get dagre-placed near existing nodes; persisted positions win.
- Migrations + Tauri commands for `schema_map_positions` and `schema_map_prefs`.
- Drag persistence wired (`onNodeDragStop` → `saveSchemaMapPosition`).
- Reset Layout action exposed as a small "Reset" button in the toolbar (only control besides the picker in this step).
- Embedded preview on table-structure: unchanged — still uses `SchemaRelationshipMap` with the same props. Drag persistence and dagre apply there too because they live in the shared layout function.
- Fullscreen overlay: unchanged.

### Files touched

- `src/lib/store/types.ts` — new types (`SchemaMapPrefs`, `OverviewTabId` extension).
- `src/lib/store/index.ts` — barrel re-exports.
- `src/lib/store/connections.ts` — `connectionSchemaMapSchema` + setter, cleanup.
- `src/lib/store/relational-tables.ts` — positions + prefs caches, loaders, drop in cascade.
- `src/lib/schema-graph.ts` — dagre layout, position-override layer.
- `src/components/workspace-overview/overview-header.tsx` — extra tab.
- `src/components/workspace-overview/schema-map-tab.tsx` — new.
- `src/components/workspace-overview/schemas-tab.tsx` — row action wiring.
- `src/components/workspace-view.tsx` — body switch case.
- `src/components/schema-relationship-map.tsx` — onNodeDragStop wiring; persisted-position lookup.
- `src-tauri/src/storage.rs` — two new tables + read/write helpers + cascade in `delete_connection`.
- `src-tauri/src/types.rs` — `PositionRow`, `SchemaMapPrefs`, payload types.
- `src-tauri/src/lib.rs` — five new Tauri commands.
- `package.json` — `dagre` dependency.

### Tests

- Store: `loadSchemaMapPositions` (success / error / empty-id no-op); `saveSchemaMapPosition` writes through; `disconnectConnection` drops the caches.
- Store: `loadSchemaMapPrefs` + `setSchemaMapPref` behaviour.
- Component: `SchemaMapTab` renders the picker, switches schemas on selection, persists choice.
- Component: clicking the Schemas-tab row action sets `connectionSchemaMapSchema` and `connectionOverviewTab["schema-map"]`.
- Layout: dagre-based `buildSchemaGraph` honors persisted positions over computed ones; reset clears overrides.
- Rust: storage-level CRUD for both tables + cascade.

### Done when

- All five gates green, all new tests pass, embedded + fullscreen + new sub-tab all render the same data with the new layout, and a dragged node stays put after refresh.

---

## Step 2 — Column-level handles + always-on edge labels — ✅ shipped

**Goal:** edges land at columns, not table boxes, and the relationship metadata is readable.

### In scope

- Node redesign in `SchemaTableNode`:
  - For each column that participates in any FK on the table (as `from` or `to`), render `<Handle>` elements on both sides with id `${tableId}.${columnName}.left` / `.right`.
  - Non-participating columns remain handle-less.
  - Visual: small dot marker only on rows with a handle, no marker otherwise.
- Edge wiring in `buildSchemaGraph`:
  - One edge per FK constraint.
  - `sourceHandle = ${fromTableId}.${fromColumns[0]}.right`
  - `targetHandle = ${toTableId}.${toColumns[0]}.left`
  - Label always-on, format `(a, b) → (x, y)` (drop parentheses for single-column FKs).
- `labelStyle: { display: "none" }` removed; label gets a readable bg-card / border style.

### Files touched

- `src/components/schema-relationship-map.tsx` — node renderer changes.
- `src/lib/schema-graph.ts` — edge handle ids, label formatting.
- Existing tests updated.

### Tests

- Single-column FK: edge connects the expected column handles on both ends.
- Multi-column FK: still one edge, anchored at first column on each side, label lists every pair.
- Non-FK columns: no handle rendered.
- Label visible by default.

### Done when

- Edge endpoints visually land on column rows; labels appear next to each edge; multi-column FKs render as one edge.

---

## Step 3 — Crow's Foot cardinality — ✅ shipped

**Goal:** FK edges visually encode the relationship's optionality on each side.

### In scope

- Custom SVG `<marker>` defs registered as ReactFlow children:
  - `crowsfoot-one` — two parallel lines.
  - `crowsfoot-zero-or-one` — line + open circle.
  - `crowsfoot-many` — open circle + crow's foot (used as the parent-side marker today since 1:1 inference is deferred).
- Frontend cardinality inference in `buildSchemaGraph`:
  - Parent side (`markerStart`): always `crowsfoot-many`.
  - Child side (`markerEnd`): `crowsfoot-one` if every `fromColumn` is NOT NULL, else `crowsfoot-zero-or-one`. Look up nullability via the existing `SchemaTableColumn.nullable` field on the `from` table.
- Stroke style adjusts slightly so markers are visible against the bg.

### Files touched

- `src/components/schema-relationship-map.tsx` — SVG marker defs as ReactFlow children.
- `src/lib/schema-graph.ts` — marker id assignment per edge.
- Tests.

### Tests

- FK with NOT NULL columns → child marker is `crowsfoot-one`.
- FK with one nullable column → child marker is `crowsfoot-zero-or-one`.
- Parent marker always `crowsfoot-many`.
- Edges with no matching column metadata (e.g. external FK targets) fall back to a sensible default.

### Done when

- Crow's Foot markers render at both ends of every FK edge with the correct optionality.

---

## Step 4 — Routing + attribute display modes + prefs persistence + comments — ✅ shipped

**Goal:** the toolbar lands. Every user-controlled rendering preference is persisted per `(connection, schema)`.

### In scope

- Toolbar component (`schema-map-toolbar.tsx`) added to the sub-tab. Same toolbar reused in the fullscreen overlay. Embedded preview stays compact (no toolbar).
- Controls in left-to-right order: **Schema picker** (existing) · **Mode** segmented control (All / Keys-only / None) · **Types** chip · **NULL** chip · **Comments** chip · **Routing** toggle (bezier / step) · **Reset layout** · *(export buttons land in Step 5)*.
- `loadSchemaMapPrefs` lazy on sub-tab activation; writes via `setSchemaMapPref` flush through.
- Node renderer responds to mode + chips:
  - **All**: every column row.
  - **Keys-only**: PK columns + FK-participating columns only.
  - **None**: header + column count only.
  - **Types**: toggle type glyph render.
  - **NULL**: toggle a small marker (`NN` / `?`) on each column.
  - **Comments**: toggle a tooltip / muted secondary line under each column name.
- Edge type toggles between `default` (bezier) and `step` (sharp orthogonal) via `edge.type`.
- **Backend extension** for column comments:
  - `postgres::fetch_schema_relationships` SQL gains `LEFT JOIN pg_description d ON d.objoid = c.oid AND d.objsubid = a.attnum`, returning `description` as the new column's `comment` field.
  - `SchemaTableColumn` shape grows `comment: Option<String>`.
  - ClickHouse impl: returns `None` for every column comment.
  - Frontend `SchemaTableColumn` mirrors the new field.

### Files touched

- `src/components/workspace-overview/schema-map-toolbar.tsx` — new.
- `src/components/workspace-overview/schema-map-tab.tsx` — toolbar mount.
- `src/components/schema-relationship-map.tsx` — node rendering reacts to prefs; fullscreen overlay uses the toolbar.
- `src/lib/schema-graph.ts` — keys-only filtering, edge type from prefs.
- `src/lib/store/types.ts` — `comment: string | null` on `SchemaTableColumn`.
- `src/lib/store/relational-tables.ts` — prefs cache loaders + setter.
- `src-tauri/src/postgres.rs` — SQL extension.
- `src-tauri/src/clickhouse.rs` — backfill `None` for comments.
- `src-tauri/src/types.rs` — `comment` field on the column shape.
- Tests across the board.

### Tests

- Mode changes redraw the node body (all → keys-only → none).
- Type / NULL / Comments chips toggle their respective markers.
- Routing toggle flips `edge.type`.
- Prefs persist across sub-tab remounts.
- Backend SQL returns `comment` for tables with `pg_description` rows.
- ClickHouse mock returns `null` everywhere; node renders without crashing.

### Done when

- A user can rearrange a graph, hide types/NULLs, switch to keys-only, flip routing to orthogonal, leave the tab, come back, and see the same view.

---

## Step 5 — Image export — ✅ shipped

**Goal:** export the current schema map to PNG or SVG for use outside dbunk.

### In scope

- Add **`html-to-image`** dependency.
- Two toolbar buttons: **PNG** · **SVG**.
- Capture scope: whole graph at fit-view bounds. Temporarily apply a `.export-light` wrapper class that forces light-theme CSS variables so the artifact reads outside the app.
- Filename pattern: `${connectionName}-${schema}-schema.{png,svg}`, with safe-character normalization (lowercase, replace non-alphanumeric with `-`, collapse runs).
- New helper `downloadDataUrl(filename, dataUrl)` next to `downloadFile`.
- Errors (CORS in font loading, missing renderer node, etc.) surface as a toast via the existing toast system.

### Files touched

- `src/lib/download.ts` — new `downloadDataUrl` helper.
- `src/components/workspace-overview/schema-map-toolbar.tsx` — PNG / SVG buttons.
- `src/components/schema-relationship-map.tsx` — export entry point that resolves the ReactFlow renderer node and wraps the export with the light-theme class.
- `package.json` — `html-to-image` dependency.
- Tests.

### Tests

- Export helper produces a data URL when the renderer node is present.
- Filename generation normalizes edge cases (spaces, slashes, unicode).
- Light-theme wrapper is applied and removed even if the export throws.
- Toolbar buttons fire the right exporter.

### Done when

- Export buttons produce downloadable PNG and SVG files of the entire current graph at the configured prefs, on a light background regardless of app theme.

---

## Wrap-up — ✅ shipped

Completed after Step 5:

- ✅ Updated [`ROADMAP.md`](../../ROADMAP.md) §"Schema relationship map" — covered bullets flipped to ✅; virtual/user-drawn relationships, notes/annotations, and multi-schema/custom-pick diagrams remain ❌.
- ✅ Updated [`PHASES.md`](./PHASES.md) — Phase 2 is marked `✅ shipped` with a recap.
- ✅ Filed follow-up issue for deferred items: [Schema map Phase 2 deferred follow-ups](https://github.com/imran-vz/dbunk/issues/17).

---

## Decision log

Decisions captured during the Phase 2 grilling, in case future readers wonder why something was chosen over an alternative.

| # | Decision | Alternative considered | Rationale |
|---|---|---|---|
| Q1 | Dedicated overview sub-tab **+** keep embedded preview **+** keep fullscreen | Sub-tab only / status-quo only / workspace tab kind | Embedded preview is a real discovery affordance; sub-tab is the right home for power-user toolbars; workspace-tab-kind adds plumbing we don't need yet. |
| Q2 | Schema picker in toolbar; persist last-viewed; deep-link from Phase 1 Schemas | Empty until navigated in / multi-schema in one canvas | Cold-open should never be broken; multi-schema diagrams deserve their own design pass. |
| Q3 | SQLite-backed drag positions; save on `onNodeDragStop` | In-memory only / JSON sidecar / debounced saves | Layout should survive restart; `onNodeDragStop` is the cleanest write trigger. |
| Q4 | dagre, `LR` direction, used as initial + Reset target | Keep grid / elkjs | dagre is the natural pairing with persistent drag; ELK is overkill at this graph size. |
| Q5 | Handles only on FK-participating columns; one edge per FK constraint; labels always-on | Handles on every column / one edge per column pair / on-hover labels | Visual signal-to-noise; matches DBeaver's mental model of "constraint = relationship." |
| Q6 | Crow's Foot only; nullability inferred frontend-side | Multi-notation toggle / extend backend payload | PHASES.md says Crow's Foot. Frontend inference is correct for the common case; 1:1 detection deferred. |
| Q7 | `step` (sharp orthogonal); per-(connection, schema) prefs in one SQLite table | `smoothstep` / global preference | DBeaver users will recognize sharp orthogonal; per-graph granularity is the right fit. |
| Q8 | All / Keys-only / None + Types / NULL / Comments chips; backend extension for `pg_description` | Drop comments / lazy-load comments | Comments are listed in the brief; one extra JOIN is cheap and avoids over-engineering. |
| Q9 | `html-to-image` library; fit-bounds scope; always-light background; two toolbar buttons | Hand-rolled serializer / viewport capture / theme-aware export / dropdown | Reinventing image export is poor ROI; fit-bounds is what 95% of users want; light bg renders correctly outside dbunk. |
| Q10 | PG content + cross-cutting features for PG and ClickHouse; MySQL/SQLite unchanged | Also wire MySQL/SQLite FK introspection / strictly PG | ROADMAP frames the work as Postgres-first; MySQL/SQLite FK introspection deserves its own roadmap entry. |
| Q11 | Annotations deferred | In-scope as stretch | The other six bullets already form a coherent overhaul; annotations need their own design pass. |
| Q12 | Five linear sub-phases on `main` | One big commit / per-feature worktree fan-out | Matches the cadence used for Phase 1; each sub-phase independently revertible. |
