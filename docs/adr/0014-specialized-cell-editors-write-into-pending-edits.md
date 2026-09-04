# ADR-0014 — Specialized cell editors write into pending row edits

**Status**: Accepted (2026-09-01; proposed 2026-05-14) — delivered; Plan 015 moved the structure editor onto typed ops

## Context

The Specialized Editors panel today parses JSON, formats arrays, and
previews PostGIS geometry, but it doesn't persist those values back
into the row. A user editing a `jsonb` cell has to:

1. Open Specialized Editors.
2. Edit the JSON in a textarea.
3. Click Copy.
4. Switch to the data grid.
5. Paste into the single-line cell editor.

That's not a "specialized cell editor" — it's a notepad. Worse, types
like `text[]` and `jsonb[]` aren't representable at all in the
default single-line cell editor because their literal form needs
careful quoting.

Cell types that warrant a specialized editor:

- `json`, `jsonb` — multi-line tree / textarea with `JSON.parse`
  validation.
- `text[]`, `int[]`, `<type>[]` — list editor with reorderable rows.
- `geometry`, `geography` (PostGIS) — WKT textarea with map preview.
- `bytea` — hex editor (later).
- `interval` — composite picker (later).

`PENDING_TASKS.md §Other Postgres-shaped tooling` flags this as the
gap between "viewer exists" and "you can actually edit".

## Decision

Introduce a pluggable cell-editor registry that the data grid
consults on cell double-click:

```ts
type CellEditorRegistry = {
  [category: string]: React.ComponentType<CellEditorProps>;
};

type CellEditorProps = {
  initialLiteral: string;        // PG literal already in the cell
  column: ColumnInfo;
  onSave: (literal: string) => void;
  onCancel: () => void;
};
```

The registry is keyed by the same `categoryFor(dataType)` helper
that `src/lib/mock-data.ts` already exposes — extending it covers
both write and generate paths from one classifier.

**Default cell editor stays.** The single-line inline editor remains
the renderer for primitive types (text, numeric, boolean, date,
timestamp, uuid, inet). The registry only kicks in for the
"specialized" categories listed above. If a column doesn't match,
the grid falls through to the default editor — zero regression risk.

**Write path.** On save, the editor passes a PG literal string. The
grid routes the literal through the existing `setQueryEdit` /
`setTableEdit` action (same path as today's single-line edit). The
pending-mutations layer is unchanged.

**Validation.** Client-side, in the editor component, before
`onSave` fires:

- JSON: `JSON.parse(value)` must succeed.
- Array: every row non-empty unless `nullable`.
- WKT: `parseWkt(value)` returns a non-error result (helper already
  in `specialized-editors.tsx`).

Server-side validation continues via the existing
`save_pending_edits` round-trip; the editor's check is purely UX.

**Lifecycle.** The editor opens as an overlay positioned above the
cell (existing pattern in the data grid for the row-details panel).
Escape cancels; Cmd-Enter saves. The grid sets `aria-modal` and
focus-traps until close. Failure on save shows an inline error and
keeps the overlay open with the user's input intact.

## Consequences

- Adding a new specialized cell type is a registry entry + a React
  component. No grid changes, no store changes.
- `specialized-editors.tsx` graduates from "generate DDL" panel to
  the home of two distinct types of editors: schema-level (GRANT,
  RLS, INDEX, FK, TRIGGER — typed reviewed operations on PostgreSQL,
  ADR-0014 *not* applicable) and cell-level (JSON, array, WKT —
  registry, ADR-0014 applicable). The file may split when it gets
  there.
- The Specialized Editors *panel* queues schema-level operations on
  PostgreSQL. Engines without the typed operation contract retain the
  "Copy" / "Open in SQL editor" fallback. Cell-level editors don't
  appear in the panel at all — they fire from the data grid.
- Pending mutations diff format is unchanged: editors emit PG
  literals, same as today's inline edit.
- Every schema-level panel queues typed backend DDL operations into the
  structure editor's shared pending list on PostgreSQL (index and foreign key
  in Plan 015; GRANT, RLS, and trigger in Plan 017). They do not use the
  cell-editor registry. Other engines retain their prior generated-SQL
  fallback.
- A future "fancy" editor (graph view for `tsvector`, hex grid for
  `bytea`) is a registry entry, not a redesign.

## Alternatives considered

1. **Modal dialogs per cell type.** Same UI, different positioning.
   Rejected — modal context-switches the user out of the grid; the
   overlay-above-the-cell pattern keeps the row visible.
2. **Open every specialized type in the SQL editor with an
   `UPDATE ...` template.** Trivial to ship but breaks the
   "double-click cell to edit" muscle memory and bypasses the
   pending-mutations transaction model.
3. **Make the registry generic across all types (incl. text).**
   Replace the single-line inline editor too. Rejected — the
   single-line editor is hot-path during bulk-edit and the registry
   adds a lookup + portal render per cell. Keep it scoped to
   specialized types only.

## Related

- ADR-0012 — `ConnectionFormPolicy`. Same "policy-driven render"
  pattern, applied at a different layer (forms vs cells).
- `src/lib/mock-data.ts` `categoryFor()` — reuses the classifier
  this ADR keys off.
- `PENDING_TASKS.md §Postgres array / json / PostGIS editors` — the
  tracked debt this resolves.
