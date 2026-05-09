import type { Edge, Node } from "reactflow";

export type SchemaTableNode = {
  schema: string;
  name: string;
  columnCount: number;
};

export type SchemaForeignKey = {
  constraintName: string;
  fromSchema: string;
  fromTable: string;
  fromColumns: string[];
  toSchema: string;
  toTable: string;
  toColumns: string[];
};

export type SchemaRelationships = {
  tables: SchemaTableNode[];
  foreignKeys: SchemaForeignKey[];
};

export type SchemaGraphNodeData = {
  label: string;
  schema: string;
  table: string;
  isActive: boolean;
  isExternal: boolean;
};

export type SchemaGraphNode = Node<SchemaGraphNodeData>;
export type SchemaGraphEdge = Edge;

export type SchemaGraph = {
  nodes: SchemaGraphNode[];
  edges: SchemaGraphEdge[];
};

export const schemaRelationshipsKey = (
  connectionId: string,
  schema: string,
): string => `${connectionId}::${schema}`;

const tableNodeId = (schema: string, table: string): string =>
  `${schema}.${table}`;

// A simple deterministic grid layout. dagre would give a nicer DAG layout but
// would add a runtime dependency we don't currently ship; a stable grid is
// good enough for v1 and keeps tests trivial to assert against.
const NODE_WIDTH = 200;
const NODE_HEIGHT = 90;
const COLUMN_GAP = 60;
const ROW_GAP = 40;
const COLUMNS_PER_ROW = 4;

const positionFor = (index: number): { x: number; y: number } => {
  const column = index % COLUMNS_PER_ROW;
  const row = Math.floor(index / COLUMNS_PER_ROW);
  return {
    x: column * (NODE_WIDTH + COLUMN_GAP),
    y: row * (NODE_HEIGHT + ROW_GAP),
  };
};

const fkLabel = (foreignKey: SchemaForeignKey): string => {
  const left = foreignKey.fromColumns.join(", ");
  const right = foreignKey.toColumns.join(", ");
  return `${left} → ${right}`;
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
): SchemaGraph => {
  // First, collect the unique node ids in deterministic order: tables in the
  // order they were supplied, then any external FK targets in the order they
  // first appear. This gives a stable layout across renders.
  const orderedIds: string[] = [];
  const tableMeta = new Map<
    string,
    { schema: string; name: string; isExternal: boolean }
  >();

  for (const table of tables) {
    const id = tableNodeId(table.schema, table.name);
    if (!tableMeta.has(id)) {
      orderedIds.push(id);
      tableMeta.set(id, {
        schema: table.schema,
        name: table.name,
        isExternal: false,
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
        isExternal: true,
      });
    }
  }

  const nodes: SchemaGraphNode[] = orderedIds.map((id, index) => {
    const meta = tableMeta.get(id);
    if (!meta) {
      // Not reachable: ids come from tableMeta keys.
      throw new Error(`Missing metadata for node ${id}`);
    }
    return {
      id,
      position: positionFor(index),
      data: {
        label: meta.name,
        schema: meta.schema,
        table: meta.name,
        isActive: activeTable != null && meta.name === activeTable,
        isExternal: meta.isExternal,
      },
    };
  });

  const edges: SchemaGraphEdge[] = foreignKeys.map((fk) => ({
    id: `fk:${fk.constraintName}`,
    source: tableNodeId(fk.fromSchema, fk.fromTable),
    target: tableNodeId(fk.toSchema, fk.toTable),
    label: fkLabel(fk),
    type: "default",
  }));

  return { nodes, edges };
};
