# ADR-0015 — Visual query builder: scaffold-then-text, no round-trip

**Status**: Proposed (2026-05-14)

## Context

DBeaver-parity asks for a visual query builder. The expensive part
isn't building SQL from a canvas — that's a pure render. The
expensive part is the inverse direction: once a user edits the
generated text, they expect the builder to reflect those edits when
they reopen the canvas. Round-tripping requires a full PostgreSQL AST
parser plus a layout-preserving transformation that survives
arbitrary text. Every shipped builder we've looked at either:

- Punts the round-trip (Looker, Hasura console, Metabase notebook).
- Locks the user out of the text editor once they "go visual"
  (DataGrip Query Builder).
- Has a parser that gives up on half the dialect (DBeaver Query
  Builder, which routinely loses CTEs, window functions, lateral
  joins).

Punting the round-trip is the only honest v1 ship for an OSS app
that doesn't want a parser maintenance burden.

## Decision

**Scaffold-then-text.** The visual builder generates SQL into a new
or existing query tab. Once it's in the tab, the text is the source
of truth. Reopening the builder on the same tab starts fresh from
the canvas template — it does **not** parse the text back.

UX contract surfaced explicitly: the builder button reads "Generate
SQL" (not "Open in builder"). Generating overwrites the tab if it
came from a previous build; if the user has edited the text since,
prompt before overwriting.

**Data model.** A TypeScript IR in `src/lib/query-builder/ir.ts`:

```ts
type QueryNode = {
  select: SelectItem[];
  from: FromItem;
  joins: JoinItem[];
  where: PredicateGroup | null;
  groupBy: ColumnRef[];
  having: PredicateGroup | null;
  orderBy: OrderItem[];
  limit: number | null;
  offset: number | null;
};
```

`render(node: QueryNode, dialect: "postgres" | "mysql" | "sqlite"): string`
emits SQL. The dialect knob is reserved so the same builder works on
the other engines once they get their introspection in shape.

**Canvas.** React Flow (`@xyflow/react`, already a dep via schema-map).
Tables drag from the existing sidebar. Joins inferred from
`schemaRelationships`; user can also draw a join edge manually. The
right-rail edits the active node — adding `WHERE` clauses, GROUP BY
columns, etc.

**Reuse of schema-map work.** Layout positions, edge routing, and
node theming come from `lib/schema-graph.ts` / `lib/edge-routing.ts`
verbatim. The builder is "schema-map nodes + a SELECT-list right-
rail" structurally.

**File layout:**

- `src/lib/query-builder/ir.ts` — types + render.
- `src/lib/query-builder/from-schema.ts` — derive default join paths
  from `schemaRelationships`.
- `src/components/query-builder/*.tsx` — canvas, right-rail, header.
- `src/components/query-builder/query-builder-panel.tsx` — top-level,
  rendered as a workspace tab variant (`kind: "builder"`).

## Consequences

- v1 is cheap: ~1 sprint of UI, 0 parser dependencies. Tests are
  snapshots of `render()` output per IR fixture.
- Adding a builder node type (e.g., subquery) is an IR variant + a
  render branch + a right-rail editor. No core changes.
- Users who want round-trip get a one-line answer: "edit the SQL or
  start from the builder, not both."
- If demand for round-trip materializes, the follow-up is a *new*
  ADR introducing `pgsql-ast-parser` (or equivalent) + an
  `irFromSql()` function. The IR is the same; only the inverse
  direction is new.
- Engines that don't have FK introspection yet (MySQL, SQLite —
  tracked separately) lose the auto-join inference. The manual
  edge-drawing path still works.

## Alternatives considered

1. **Full round-trip via PG AST parser.** Rejected for v1: maintenance
   burden on a moving dialect, complex layout preservation, and the
   risk of bait-and-switch when the parser silently drops constructs.
2. **No builder at all — keep punting.** Rejected: the ask appears in
   ROADMAP and DBeaver parity. Scaffolding alone moves the needle.
3. **Text-editor-first with inline UI hints (DataSpell-style).**
   Rejected as a parallel project: requires Monaco/SQL plugin work
   and doesn't address the "I don't know the syntax" entry point a
   canvas does.

## Open questions (for follow-up ADRs, not blocking this one)

- Subquery / CTE composition in the IR (probably a `QueryNode.from`
  variant rather than a top-level CTE list).
- Window functions: render-only or first-class IR item?
- Save / share builder state alongside saved queries.

## Related

- ADR-0001 — Postgres-first engine coverage. The dialect knob in the
  renderer encodes the same priority order.
- `src/components/schema-relationship-map.tsx` — reuses the same
  React Flow + edge-routing primitives.
- ROADMAP.md §5 — the tracked debt this resolves at v1.
