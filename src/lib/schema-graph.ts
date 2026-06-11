import type { Edge, Node } from "@xyflow/react";
import * as dagre from "dagre";

export type SchemaMapRouting = "bezier" | "step";
export type SchemaMapAttrMode = "all" | "keys-only" | "none";

export type SchemaMapPosition = {
  x: number;
  y: number;
};

export type SchemaMapPrefs = {
  routing: SchemaMapRouting;
  attrMode: SchemaMapAttrMode;
  showTypes: boolean;
  showNulls: boolean;
  showComments: boolean;
};

export const DEFAULT_SCHEMA_MAP_PREFS: SchemaMapPrefs = {
  routing: "bezier",
  attrMode: "all",
  showTypes: true,
  showNulls: false,
  showComments: false,
};

/**
 * Compact Trigger Indicator metadata for Table Cards and Column Rows.
 * Full trigger function bodies / DDL deliberately stay out of the
 * Schema Map payload.
 */
export type SchemaTableTrigger = {
  name: string;
  table: string;
  /** Explicitly targeted columns (`UPDATE OF column`); absent/empty
   * when the trigger fires for the whole table. */
  columns?: string[];
  /** `BEFORE` | `AFTER` | `INSTEAD OF`. */
  timing: string;
  /** Any of `INSERT`, `UPDATE`, `DELETE`, `TRUNCATE`. */
  events: string[];
  /** `ROW` | `STATEMENT`. */
  orientation: string;
  enabled: boolean;
  functionName: string;
};

export type SchemaTableNode = {
  schema: string;
  name: string;
  columnCount: number;
  columns?: SchemaTableColumn[];
  /** Junction Table Card marker; absent on engines that don't detect. */
  isJunctionTable?: boolean;
  triggers?: SchemaTableTrigger[];
};

export type SchemaTableColumn = {
  name: string;
  dataType: string;
  nullable: boolean;
  isPrimaryKey: boolean;
  ordinalPosition: number;
  comment?: string | null;
};

/**
 * Backend-provided Relationship Cardinality. `unknown` means the
 * engine could not classify — the UI must not re-infer.
 */
export type SchemaRelationshipCardinality =
  | "one-to-one"
  | "one-to-many"
  | "unknown";

export type SchemaForeignKey = {
  constraintName: string;
  fromSchema: string;
  fromTable: string;
  fromColumns: string[];
  toSchema: string;
  toTable: string;
  toColumns: string[];
  /** `"foreign key"` for FK-backed Relationship Edges. */
  relationshipType?: string;
  cardinality?: SchemaRelationshipCardinality;
  cardinalityReason?: string;
  onUpdate?: string;
  onDelete?: string;
  /** True when any referencing (FK) column is nullable. */
  fkColumnsNullable?: boolean;
  /** True when the FK columns are covered by a unique constraint on
   * the referencing table. */
  fkColumnsUnique?: boolean;
  /** True when this edge participates in a detected junction-table
   * (many-to-many) path. */
  isJunctionParticipant?: boolean;
};

export type SchemaRelationships = {
  tables: SchemaTableNode[];
  foreignKeys: SchemaForeignKey[];
};

export type SchemaGraphNodeData = {
  tableId: string;
  label: string;
  schema: string;
  table: string;
  columnCount: number;
  columns: SchemaTableColumn[];
  fkColumnNames: string[];
  prefs: SchemaMapPrefs;
  isActive: boolean;
  isExternal: boolean;
  /** Junction Table Card marker from the backend payload. */
  isJunctionTable: boolean;
  /** Trigger Indicator metadata for the Table Card / Column Rows. */
  triggers: SchemaTableTrigger[];
  /**
   * Set by the renderer (not the builder) when the graph spans more
   * than one distinct schema, so every node header can carry its
   * `schema.` prefix in a multi-schema map.
   */
  hasMultipleSchemas: boolean;
  /**
   * Set by the renderer when a Focused Table or Focused Relationship
   * Edge exists and this Table Card is not directly related to it.
   */
  isDimmed: boolean;
  /**
   * Renderer-injected explicit table-open action for the Table Card
   * header. Opening a table is never triggered by plain single click.
   */
  onOpenTable?: (schema: string, table: string) => void;
};

export type SchemaGraphEdgeData = {
  /** The authoritative backend relationship metadata for this edge. */
  foreignKey: SchemaForeignKey;
  /** Renderer-set dimming flag mirroring node dimming. */
  isDimmed: boolean;
  /** Renderer-set Focused Relationship Edge flag. */
  isFocused: boolean;
};

export type SchemaGraphNode = Node<SchemaGraphNodeData>;
export type SchemaGraphEdge = Edge<SchemaGraphEdgeData>;

export type SchemaGraph = {
  nodes: SchemaGraphNode[];
  edges: SchemaGraphEdge[];
};

/**
 * Sentinel schema value meaning "render every schema in this connection
 * in one combined map." Picked to be unlikely as a real schema name so we
 * don't have to add a discriminated union throughout the store.
 */
export const ALL_SCHEMAS_SENTINEL = "__dbunk:all-schemas__";

export const isAllSchemas = (schema: string): boolean =>
  schema === ALL_SCHEMAS_SENTINEL;

export const schemaRelationshipsKey = (
  connectionId: string,
  schema: string,
): string => `${connectionId}::${schema}`;

/**
 * Stable persistence scope for a Table-Level Schema Map. Lives in the
 * same `(connection, scope)` keyspace as per-schema scopes and the
 * all-schemas sentinel; the `__dbunk:` prefix keeps it from colliding
 * with real schema names.
 */
export const tableSchemaMapScope = (schema: string, table: string): string =>
  `__dbunk:table-scope__::${schema}.${table}`;

/**
 * Scope a relationships payload down to a Table-Level Schema Map: the
 * current Table Card, its directly referenced and directly referencing
 * Table Cards, and only the Relationship Edges that connect the
 * current table to those direct neighbors. Second-degree neighbors and
 * edges between neighbors are excluded.
 */
export const filterSchemaRelationshipsForTable = (
  relationships: SchemaRelationships,
  schema: string,
  table: string,
): SchemaRelationships => {
  const focusId = tableNodeId(schema, table);
  const incidentFks = relationships.foreignKeys.filter(
    (fk) =>
      tableNodeId(fk.fromSchema, fk.fromTable) === focusId ||
      tableNodeId(fk.toSchema, fk.toTable) === focusId,
  );
  const keptIds = new Set<string>([focusId]);
  for (const fk of incidentFks) {
    keptIds.add(tableNodeId(fk.fromSchema, fk.fromTable));
    keptIds.add(tableNodeId(fk.toSchema, fk.toTable));
  }
  return {
    tables: relationships.tables.filter((entry) =>
      keptIds.has(tableNodeId(entry.schema, entry.name)),
    ),
    foreignKeys: incidentFks,
  };
};

const tableNodeId = (schema: string, table: string): string =>
  `${schema}.${table}`;

const NODE_WIDTH = 220;
const NODE_HEADER_HEIGHT = 42;
const NODE_ROW_HEIGHT = 24;
const NODE_MAX_LAYOUT_ROWS = 18;
const DAGRE_RANK_SEP = 120;
const DAGRE_NODE_SEP = 60;

const fkLabel = (foreignKey: SchemaForeignKey): string => {
  const left = formatColumnList(foreignKey.fromColumns);
  const right = formatColumnList(foreignKey.toColumns);
  return `${left} → ${right}`;
};

const formatColumnList = (columns: string[]): string =>
  columns.length === 1 ? (columns[0] ?? "") : `(${columns.join(", ")})`;

const handleId = (
  schema: string,
  table: string,
  column: string | undefined,
  side: "left" | "right",
): string | undefined =>
  column ? `${tableNodeId(schema, table)}.${column}.${side}` : undefined;

const mergedPrefs = (prefs?: Partial<SchemaMapPrefs>): SchemaMapPrefs => ({
  ...DEFAULT_SCHEMA_MAP_PREFS,
  ...prefs,
});

const nodeHeightFor = (
  columns: SchemaTableColumn[],
  prefs: SchemaMapPrefs,
): number => {
  if (prefs.attrMode === "none") {
    return NODE_HEADER_HEIGHT + NODE_ROW_HEIGHT;
  }
  const rows = Math.min(columns.length, NODE_MAX_LAYOUT_ROWS);
  return NODE_HEADER_HEIGHT + Math.max(1, rows) * NODE_ROW_HEIGHT;
};

const displayedColumnsFor = (
  columns: SchemaTableColumn[],
  fkColumnNames: Set<string>,
  prefs: SchemaMapPrefs,
): SchemaTableColumn[] => {
  if (prefs.attrMode === "none") {
    return [];
  }
  const ordered = [...columns].sort(
    (a, b) => a.ordinalPosition - b.ordinalPosition,
  );
  if (prefs.attrMode === "keys-only") {
    return ordered.filter(
      (column) => column.isPrimaryKey || fkColumnNames.has(column.name),
    );
  }
  return ordered;
};

const buildFkColumnMap = (
  foreignKeys: SchemaForeignKey[],
): Map<string, Set<string>> => {
  const result = new Map<string, Set<string>>();
  const add = (tableId: string, column: string) => {
    const current = result.get(tableId) ?? new Set<string>();
    current.add(column);
    result.set(tableId, current);
  };
  for (const fk of foreignKeys) {
    const fromId = tableNodeId(fk.fromSchema, fk.fromTable);
    const toId = tableNodeId(fk.toSchema, fk.toTable);
    for (const column of fk.fromColumns) {
      add(fromId, column);
    }
    for (const column of fk.toColumns) {
      add(toId, column);
    }
  }
  return result;
};

/**
 * Crow's-foot marker for the referencing/child end of the edge, driven
 * by the backend-provided Relationship Cardinality. Edges without
 * backend cardinality (minimal payloads, unsupported engines) render
 * the explicit unknown marker rather than re-inferring in the UI.
 *
 * Start markers use dedicated `-start` defs with
 * `orient="auto-start-reverse"` — a plain `auto` start marker is
 * mirrored, pointing the crow's foot away from its Table Card.
 */
const childEndMarkerFor = (fk: SchemaForeignKey): string => {
  switch (fk.cardinality) {
    case "one-to-one":
      return "url(#crowsfoot-one-start)";
    case "one-to-many":
      return "url(#crowsfoot-many-start)";
    default:
      return "url(#crowsfoot-unknown-start)";
  }
};

/**
 * Marker for the referenced/parent end: exactly-one when the FK
 * columns are non-null, zero-or-one when nullable. Backend-provided FK
 * nullability wins; column metadata is the fallback for minimal
 * payloads so existing relationship rendering keeps working.
 */
const parentEndMarkerFor = (
  fk: SchemaForeignKey,
  tableMeta: Map<
    string,
    {
      columns: SchemaTableColumn[];
    }
  >,
): string => {
  if (fk.fkColumnsNullable !== undefined) {
    return fk.fkColumnsNullable
      ? "url(#crowsfoot-zero-or-one)"
      : "url(#crowsfoot-one)";
  }
  const meta = tableMeta.get(tableNodeId(fk.fromSchema, fk.fromTable));
  if (!meta) {
    return "url(#crowsfoot-zero-or-one)";
  }
  const byName = new Map(meta.columns.map((column) => [column.name, column]));
  const everyColumnIsNotNull =
    fk.fromColumns.length > 0 &&
    fk.fromColumns.every(
      (columnName) => byName.get(columnName)?.nullable === false,
    );
  return everyColumnIsNotNull
    ? "url(#crowsfoot-one)"
    : "url(#crowsfoot-zero-or-one)";
};

const dagreLayout = (
  orderedIds: string[],
  tableMeta: Map<
    string,
    {
      columns: SchemaTableColumn[];
    }
  >,
  foreignKeys: SchemaForeignKey[],
  prefs: SchemaMapPrefs,
): Record<string, SchemaMapPosition> => {
  const graph = new dagre.graphlib.Graph();
  graph.setDefaultEdgeLabel(() => ({}));
  graph.setGraph({
    rankdir: "LR",
    ranksep: DAGRE_RANK_SEP,
    nodesep: DAGRE_NODE_SEP,
  });

  for (const id of orderedIds) {
    const meta = tableMeta.get(id);
    graph.setNode(id, {
      width: NODE_WIDTH,
      height: nodeHeightFor(meta?.columns ?? [], prefs),
    });
  }

  for (const fk of foreignKeys) {
    const source = tableNodeId(fk.fromSchema, fk.fromTable);
    const target = tableNodeId(fk.toSchema, fk.toTable);
    if (graph.hasNode(source) && graph.hasNode(target)) {
      graph.setEdge(source, target);
    }
  }

  dagre.layout(graph);

  const positions: Record<string, SchemaMapPosition> = {};
  for (const id of orderedIds) {
    const node = graph.node(id);
    positions[id] = {
      x: (node?.x ?? 0) - NODE_WIDTH / 2,
      y: (node?.y ?? 0) - (node?.height ?? NODE_HEADER_HEIGHT) / 2,
    };
  }
  return positions;
};

/**
 * Convert raw foreign-key metadata into a ReactFlow-compatible graph.
 *
 * Tables become nodes; foreign keys become edges (one edge per FK,
 * regardless of how many columns participate). FK targets that are not in
 * the supplied tables list (e.g. cross-schema references) are added as
 * synthetic nodes so the edge has somewhere to land.
 */
export const buildSchemaGraph = (
  tables: SchemaTableNode[],
  foreignKeys: SchemaForeignKey[],
  activeTable?: string | null,
  options?: {
    positions?: Record<string, SchemaMapPosition>;
    prefs?: Partial<SchemaMapPrefs>;
    /**
     * Schema-qualified active node id (`schema.table`). Takes
     * precedence over the name-only `activeTable` match, which is
     * ambiguous across schemas in multi-schema maps.
     */
    activeTableId?: string;
  },
): SchemaGraph => {
  const prefs = mergedPrefs(options?.prefs);
  const fkColumnMap = buildFkColumnMap(foreignKeys);
  // First, collect the unique node ids in deterministic order: tables in the
  // order they were supplied, then any external FK targets in the order they
  // first appear. This gives a stable layout across renders.
  const orderedIds: string[] = [];
  const tableMeta = new Map<
    string,
    {
      schema: string;
      name: string;
      columnCount: number;
      columns: SchemaTableColumn[];
      isExternal: boolean;
      isJunctionTable: boolean;
      triggers: SchemaTableTrigger[];
    }
  >();

  for (const table of tables) {
    const id = tableNodeId(table.schema, table.name);
    if (!tableMeta.has(id)) {
      orderedIds.push(id);
      tableMeta.set(id, {
        schema: table.schema,
        name: table.name,
        columnCount: table.columnCount,
        columns: table.columns ?? [],
        isExternal: false,
        isJunctionTable: table.isJunctionTable ?? false,
        triggers: table.triggers ?? [],
      });
    }
  }

  for (const fk of foreignKeys) {
    const id = tableNodeId(fk.toSchema, fk.toTable);
    if (!tableMeta.has(id)) {
      orderedIds.push(id);
      tableMeta.set(id, {
        schema: fk.toSchema,
        name: fk.toTable,
        columnCount: 0,
        columns: [],
        isExternal: true,
        isJunctionTable: false,
        triggers: [],
      });
    }
  }

  const dagrePositions = dagreLayout(orderedIds, tableMeta, foreignKeys, prefs);

  const nodes: SchemaGraphNode[] = orderedIds.map((id, index) => {
    const meta = tableMeta.get(id);
    if (!meta) {
      // Not reachable: ids come from tableMeta keys.
      throw new Error(`Missing metadata for node ${id}`);
    }
    void index;
    const fkColumnNames = fkColumnMap.get(id) ?? new Set<string>();
    return {
      id,
      position: options?.positions?.[id] ??
        dagrePositions[id] ?? { x: 0, y: 0 },
      data: {
        tableId: id,
        label: meta.name,
        schema: meta.schema,
        table: meta.name,
        columnCount: meta.columnCount,
        columns: displayedColumnsFor(meta.columns, fkColumnNames, prefs),
        fkColumnNames: [...fkColumnNames],
        prefs,
        isActive:
          options?.activeTableId != null
            ? id === options.activeTableId
            : activeTable != null && meta.name === activeTable,
        isExternal: meta.isExternal,
        isJunctionTable: meta.isJunctionTable,
        triggers: meta.triggers,
        hasMultipleSchemas: false,
        isDimmed: false,
      },
    };
  });

  // Edge ids carry the referencing table — PostgreSQL constraint names
  // are only unique per table, so a bare name would collide for
  // same-named FKs on different tables.
  const edges: SchemaGraphEdge[] = foreignKeys.map((fk) => ({
    id: `fk:${tableNodeId(fk.fromSchema, fk.fromTable)}.${fk.constraintName}`,
    source: tableNodeId(fk.fromSchema, fk.fromTable),
    target: tableNodeId(fk.toSchema, fk.toTable),
    sourceHandle:
      prefs.attrMode === "none"
        ? undefined
        : handleId(fk.fromSchema, fk.fromTable, fk.fromColumns[0], "right"),
    targetHandle:
      prefs.attrMode === "none"
        ? undefined
        : handleId(fk.toSchema, fk.toTable, fk.toColumns[0], "left"),
    label: fkLabel(fk),
    labelShowBg: true,
    labelBgStyle: {
      fill: "var(--card)",
      fillOpacity: 0.96,
    },
    labelBgPadding: [6, 3],
    labelBgBorderRadius: 4,
    labelStyle: {
      fill: "var(--foreground)",
      fontSize: 10,
      fontWeight: 600,
    },
    // markerStart sits at the source (referencing/child) end of the
    // path, markerEnd at the target (referenced/parent) end.
    markerStart: childEndMarkerFor(fk),
    markerEnd: parentEndMarkerFor(fk, tableMeta),
    type: prefs.routing === "step" ? "step" : "default",
    style: { stroke: "var(--primary)", strokeWidth: 1.45 },
    data: { foreignKey: fk, isDimmed: false, isFocused: false },
  }));

  return { nodes, edges };
};
