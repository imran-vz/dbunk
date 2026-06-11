# Schema Map Relationship Inspection Plan

Status: implemented (2026-06-11). Browser verification note: automated
GUI driving was unavailable (no Screen Recording / Accessibility
permission for the agent process); the backend payload was verified
end-to-end against the live seeded test database (`pnpm db:postgres`),
including cardinality, junction detection, trigger metadata, FK
actions/nullability, and partitioned/multi-schema layouts, and the app
was built and launched successfully. Interactive drag/focus/popover
behavior is covered by component tests and should get a quick manual
pass in the running app.
Date: 2026-06-11

## Goal

Improve the relational Schema Map so users can inspect and rearrange
relationships accurately:

- Table Cards are freely draggable and positions persist per
  `(connection, schema/map scope)`.
- Relationship Edges use standard notation backed by backend-provided
  Relationship Cardinality.
- Relationship Edges are clickable and show authoritative relationship
  metadata in a Relationship Detail Popover.
- Focusing a Table Card or Relationship Edge emphasizes directly related
  graph elements and dims unrelated graph elements.
- A UI glossary explains the canonical Schema Map terms.
- Table workspaces get a peer Schema Map subtab scoped to the current table's
  direct incoming and outgoing relationships.

This should ship as one PR, implemented in reviewable slices with verification
gates.

## Assumptions

- This work applies to relational engines only.
- PostgreSQL is the first full-fidelity implementation, consistent with
  ADR-0001.
- Engines without enough relationship metadata should return `unknown`
  cardinality and omit unsupported metadata rather than guessing.
- The Schema Map payload should be compact and ready to render. It may use
  multiple backend-side SQL queries when that is clearer or more reliable, but
  it should avoid per-click database calls for data required by the map or
  edge popovers.
- Full trigger function bodies and full DDL are out of scope for the map
  payload. They can remain in existing structure/SQL tooling or be fetched
  lazily in future drill-down flows.

## Slice 1: Backend Relationship Metadata

Extend `load_schema_relationships` so PostgreSQL returns compact authoritative
metadata for each Relationship Edge and Table Card.

Data to add:

- FK constraint name.
- Relationship type: `foreign key`.
- Relationship Cardinality: `one-to-one`, `one-to-many`, or `unknown`.
- Optional cardinality reason.
- Referencing table and columns.
- Referenced table and columns.
- `ON UPDATE` action.
- `ON DELETE` action.
- Whether FK columns are nullable.
- Whether FK columns are unique.
- Whether the edge participates in a detected junction-table path.
- Junction Table Card marker when detected.
- Compact Trigger Indicator metadata for Table Cards and Column Rows:
  trigger name, table, targeted columns when any, timing, events,
  orientation, enabled state, and function/procedure name.

Cardinality rules:

- `one-to-one`: FK columns are constrained unique on the referencing table.
- `one-to-many`: FK columns are not constrained unique on the referencing
  table.
- `unknown`: metadata is unavailable or unsupported.
- Many-to-many remains represented by the real FK Relationship Edges; detected
  junction tables and participating edges are labelled as many-to-many
  participants rather than replaced by synthetic direct edges.

Verification gate:

- Rust tests cover PostgreSQL cardinality classification, FK actions,
  FK nullability, FK uniqueness, junction-table detection, and compact trigger
  metadata.
- TypeScript types and store tests accept the expanded payload.
- Unsupported engines keep rendering with `unknown` or absent optional fields.

## Slice 2: Graph Model And Relationship Popovers

Update `src/lib/schema-graph.ts` and the Schema Map renderer to consume the
expanded backend metadata.

Behavior:

- Relationship Edges carry the backend-provided relationship metadata.
- Edge labels remain compact; detailed metadata appears in the Relationship
  Detail Popover.
- Clicking a Relationship Edge opens the popover and sets a Focused
  Relationship Edge.
- The popover shows:
  - FK constraint name.
  - Relationship type.
  - Cardinality.
  - Cardinality reason.
  - Referencing table and columns.
  - Referenced table and columns.
  - `ON UPDATE` and `ON DELETE`.
  - FK column nullable status.
  - FK column unique status.
  - Junction-table participation when applicable.

Verification gate:

- Unit tests assert graph edges contain the new metadata.
- Component tests assert edge click opens the popover with authoritative
  metadata.
- Existing relationship rendering tests still pass for minimal payloads.

## Slice 3: Standard Relationship Notation

Render Relationship Edges with crow's-foot notation driven by backend
Relationship Cardinality and FK optionality.

Notation:

- `one-to-one`: one marker at both ends.
- `one-to-many`: one marker at the referenced/parent end and crow's-foot at
  the referencing/child end.
- Nullable FK columns: zero-or-one on the referencing side.
- Non-null FK columns: exactly-one on the referencing side.
- `unknown`: neutral/unknown marker or explicit unknown label.
- Many-to-many participant: keep real FK markers and show participant status
  in the edge popover and junction Table Card.

Verification gate:

- Graph tests cover marker selection for one-to-one, one-to-many, nullable,
  non-null, and unknown relationships.
- Component tests assert marker IDs/styles are applied from backend
  cardinality rather than re-inferred solely in the UI.

## Slice 4: Focus, Dimming, And Dragging

Make Schema Map exploration inspectable without accidental navigation.

Behavior:

- Single-clicking a Table Card sets the Focused Table.
- A Focused Table keeps the selected Table Card, directly referencing Table
  Cards, directly referenced Table Cards, and connected Relationship Edges
  emphasized.
- Unrelated Table Cards and Relationship Edges dim.
- Clicking the empty canvas clears focus.
- Clicking a Relationship Edge sets a Focused Relationship Edge and keeps only
  its endpoint Table Cards and the selected edge emphasized.
- Opening a table moves to an explicit Table Card header action and/or
  double-click.
- Dragging a Table Card does not focus or open the table.
- Dragged positions persist per `(connection, schema/map scope)`.
- Reset layout continues to clear saved positions.

Verification gate:

- Component tests cover table focus, edge focus, dimming, canvas clear, and
  explicit table open behavior.
- Drag tests prove drag stop persists position without calling the table-open
  path.
- Browser verification confirms Table Cards are actually draggable in the
  rendered React Flow map.

## Slice 5: Trigger Indicators On Table Cards And Column Rows

Show compact trigger metadata where it belongs in the Schema Map.

Behavior:

- Table Cards indicate table-level triggers.
- Column Rows indicate triggers that explicitly target that column, such as
  PostgreSQL `UPDATE OF column`.
- Relationship Edges do not show trigger metadata unless a future feature
  explicitly models trigger-to-relationship behavior.

Verification gate:

- Backend tests cover table-level and column-targeted trigger extraction.
- Component tests assert Table Card and Column Row Trigger Indicators render
  from the payload.
- Edge popover tests assert trigger metadata is not mixed into relationship
  details.

## Slice 6: Schema Map Glossary

Add a compact glossary entry point to the Schema Map toolbar.

Behavior:

- Toolbar includes a Glossary button.
- The glossary uses the canonical terms from `CONTEXT.md`:
  Schema Map, Table-Level Schema Map, Table Card, Junction Table Card,
  Column Row, Trigger Indicator, Relationship Edge, Relationship Cardinality,
  Relationship Detail Popover, Focused Table, and Focused Relationship Edge.
- Glossary text is UI-facing help text, but it should not coin different
  terms for the same concepts.

Verification gate:

- Toolbar/component tests assert the glossary opens and contains the canonical
  terms.
- No glossary copy is embedded directly over the map canvas.

## Slice 7: Table-Level Schema Map Subtab

Add a new table workspace subtab named `Schema Map`, as a peer to `Data`,
`Schema`, `Indexes`, `Relations`, and `Specialized`.

Behavior:

- Keep the existing `Relations` subtab as the textual/detail list.
- The table-level Schema Map shows only:
  - the current Table Card,
  - directly referenced Table Cards,
  - directly referencing Table Cards,
  - the Relationship Edges connecting those direct neighbors.
- The table-level map reuses the same notation, edge popover, focus/dimming,
  dragging, and glossary behavior as the global Schema Map.
- Layout persistence remains scoped by `(connection, selected schema/map
  scope)`. The table-level scope needs a stable key that cannot collide with
  the global per-schema and all-schema scopes.

Verification gate:

- Table editor tests assert the `Schema Map` subtab exists and `Relations`
  remains available.
- Table-level graph tests assert both incoming and outgoing direct neighbors
  are included, while second-degree neighbors are excluded.
- Component tests assert edge popovers and focus/dimming work in table-level
  mode.

## Final Verification

Before the task is considered complete:

- `pnpm format`
- `pnpm lint`
- `pnpm typecheck`
- `just fmt`
- `just lint`
- `just test`
- Browser verification of the global Schema Map and table-level Schema Map
  for dragging, focus/dimming, edge popovers, and nonblank rendering.

## ADR Review

No ADR is required for this plan at this stage.

Reasoning:

- PostgreSQL-first metadata follows ADR-0001 rather than changing it.
- React Flow and persisted schema-map positions are already established in
  the codebase.
- The main decisions are UI behavior and payload shape, which are reversible
  implementation choices and documented here plus in `CONTEXT.md`.
