import {
  applyNodeChanges,
  Background,
  Controls,
  type EdgeMouseHandler,
  type Node,
  type NodeChange,
  type NodeMouseHandler,
  type OnNodeDrag,
  ReactFlow,
} from "@xyflow/react";
import {
  forwardRef,
  useCallback,
  useEffect,
  useImperativeHandle,
  useMemo,
  useRef,
  useState,
} from "react";

import { RelationshipDetailPopover } from "@/components/relationship-detail-popover";
import { CrowsFootMarkers } from "@/components/schema-relationship-map/crows-foot-markers";
import {
  type SchemaMapNodeData,
  SchemaTableNode,
  typeGlyph,
} from "@/components/schema-relationship-map/node";
import { downloadDataUrl } from "@/lib/download";
import { relationalPolicy, storageClassFor } from "@/lib/engine-policy";
import {
  buildSchemaGraph,
  DEFAULT_SCHEMA_MAP_PREFS,
  isAllSchemas,
  type SchemaForeignKey,
  type SchemaGraphEdge,
  type SchemaGraphNodeData,
  type SchemaMapPosition,
  type SchemaRelationships,
  type SchemaTableNode as SchemaTableNodeData,
  schemaRelationshipsKey,
  tableSchemaMapScope,
} from "@/lib/schema-graph";
import { type SchemaRelationshipsStatus, useAppStore } from "@/lib/store";

export { typeGlyph };

interface SchemaRelationshipMapProps {
  connectionId: string;
  schema: string;
  activeTable: string | null;
  isClient?: boolean;
  /**
   * Table-Level Schema Map mode: scope the graph to this table's
   * direct incoming/outgoing relationships. Positions and prefs
   * persist under a dedicated `(connection, table scope)` key.
   */
  tableScope?: { schema: string; table: string };
}

/** Focused Table or Focused Relationship Edge. */
type SchemaMapFocus =
  | { kind: "table"; tableId: string }
  | { kind: "edge"; edgeId: string };

const EMPTY_POSITIONS: Record<string, SchemaMapPosition> = {};

/**
 * Combine per-schema relationship payloads into one. Tables are
 * deduplicated by `(schema, name)`; FKs by their referencing table
 * plus constraint name — PostgreSQL constraint names are only unique
 * per table, so two tables (or two schemas) may legitimately reuse the
 * same FK name and both edges must survive the merge.
 */
function mergeSchemaRelationships(
  entries: ReadonlyArray<SchemaRelationships | undefined>,
): SchemaRelationships | undefined {
  if (entries.length === 0) return undefined;
  if (entries.some((entry) => entry === undefined)) return undefined;
  return mergeDefinedSchemaRelationships(entries);
}

/** The merge itself, skipping undefined entries instead of bailing. */
function mergeDefinedSchemaRelationships(
  entries: ReadonlyArray<SchemaRelationships | undefined>,
): SchemaRelationships {
  const tables = new Map<string, SchemaTableNodeData>();
  const foreignKeys = new Map<string, SchemaForeignKey>();
  for (const entry of entries) {
    if (!entry) continue;
    for (const table of entry.tables) {
      tables.set(`${table.schema}.${table.name}`, table);
    }
    for (const fk of entry.foreignKeys) {
      foreignKeys.set(
        `${fk.fromSchema}.${fk.fromTable}::${fk.constraintName}`,
        fk,
      );
    }
  }
  return {
    tables: [...tables.values()],
    foreignKeys: [...foreignKeys.values()],
  };
}

/**
 * Roll several per-schema statuses up into a single status for the
 * aggregated map: any loading bubbles up first, then errors, then
 * success only when every entry succeeded.
 */
function aggregateStatus(
  statuses: ReadonlyArray<SchemaRelationshipsStatus | undefined>,
): SchemaRelationshipsStatus | undefined {
  if (statuses.length === 0) return undefined;
  if (statuses.some((s) => s?.state === "loading")) return { state: "loading" };
  const firstError = statuses.find((s) => s?.state === "error");
  if (firstError?.state === "error") return firstError;
  if (statuses.every((s) => s?.state === "success"))
    return { state: "success" };
  return undefined;
}

export type SchemaMapExportFormat = "png" | "svg";

export interface SchemaRelationshipMapHandle {
  exportImage: (
    format: SchemaMapExportFormat,
    filename: string,
  ) => Promise<void>;
}

type HtmlToImageApi = {
  toPng: (
    node: HTMLElement,
    // oxlint-disable-next-line anti-slop/no-unsafe-dictionary-type -- The value is handled at a typed library or domain boundary here.
    options?: Record<string, unknown>,
  ) => Promise<string>;
  toSvg: (
    node: HTMLElement,
    // oxlint-disable-next-line anti-slop/no-unsafe-dictionary-type -- The value is handled at a typed library or domain boundary here.
    options?: Record<string, unknown>,
  ) => Promise<string>;
};

export async function exportSchemaMapImage(
  container: HTMLElement | null,
  format: SchemaMapExportFormat,
  filename: string,
  imageApi?: HtmlToImageApi,
): Promise<void> {
  if (!container) {
    throw new Error("Schema map renderer is not mounted.");
  }
  const renderer =
    container.querySelector<HTMLElement>(".react-flow__renderer") ??
    container.querySelector<HTMLElement>(".react-flow");
  if (!renderer) {
    throw new Error("Schema map renderer was not found.");
  }

  const api = imageApi ?? (await import("html-to-image"));
  renderer.classList.add("export-light");
  try {
    const options = {
      backgroundColor: "#ffffff",
      cacheBust: true,
      pixelRatio: format === "png" ? 2 : 1,
    };
    const dataUrl =
      format === "png"
        ? await api.toPng(renderer, options)
        : await api.toSvg(renderer, options);
    downloadDataUrl(filename, dataUrl);
  } finally {
    renderer.classList.remove("export-light");
  }
}

const nodeTypes = {
  schemaTable: SchemaTableNode,
};

export const SchemaRelationshipMap = forwardRef<
  SchemaRelationshipMapHandle,
  SchemaRelationshipMapProps
>(function SchemaRelationshipMap(
  { connectionId, schema, activeTable, isClient = true, tableScope },
  ref,
) {
  const {
    schemaRelationships,
    schemaRelationshipsStatus,
    loadSchemaRelationships,
    loadTableSchemaRelationships,
    focusTableInSchemaMap,
    connections,
    schemaExplorer,
    schemaMapPositions,
    loadSchemaMapPositions,
    saveSchemaMapPosition,
    schemaMapPrefs,
    loadSchemaMapPrefs,
  } = useAppStore();
  const engine = connections.find(
    (connection) => connection.id === connectionId,
  )?.engine;

  const isAllMode = isAllSchemas(schema);
  // Depend on the scope's primitives, not the `tableScope` object —
  // callers typically pass a fresh object literal per render, and an
  // identity-sensitive effect would refetch in a loop.
  const tableScopeSchema = tableScope?.schema;
  const tableScopeTable = tableScope?.table;
  const isTableMode = tableScopeTable != null;
  const databaseSchemas = useMemo(() => {
    if (!isAllMode) return [];
    const names = new Set(
      (schemaExplorer[connectionId] ?? []).map((entry) => entry.name),
    );
    return [...names].sort();
  }, [isAllMode, schemaExplorer, connectionId]);

  // Positions and prefs persist per `(connection, map scope)` — a real
  // schema name, the all-schemas sentinel, or the table-level scope.
  const mapScope =
    tableScopeSchema != null && tableScopeTable != null
      ? tableSchemaMapScope(tableScopeSchema, tableScopeTable)
      : schema;
  const key = schemaRelationshipsKey(connectionId, mapScope);
  const positions =
    schemaMapPositions[connectionId]?.[mapScope] ?? EMPTY_POSITIONS;
  const prefs =
    schemaMapPrefs[connectionId]?.[mapScope] ?? DEFAULT_SCHEMA_MAP_PREFS;

  // Memoise the merged relationships + rolled-up status in "Database"
  // mode — the merge produces a fresh object every call, and feeding
  // that into downstream `useMemo`s without memoising here triggers an
  // infinite render loop via the `setInteractiveNodes(styledNodes)`
  // effect below.
  const relationships = useMemo<SchemaRelationships | undefined>(() => {
    if (isTableMode) {
      return schemaRelationships[key];
    }
    if (!isAllMode) {
      return schemaRelationships[schemaRelationshipsKey(connectionId, schema)];
    }
    const entries = databaseSchemas.map(
      (name) => schemaRelationships[schemaRelationshipsKey(connectionId, name)],
    );
    return mergeSchemaRelationships(entries);
  }, [
    isAllMode,
    isTableMode,
    schemaRelationships,
    key,
    schema,
    connectionId,
    databaseSchemas,
  ]);

  const status = useMemo<SchemaRelationshipsStatus | undefined>(() => {
    if (isTableMode) {
      return schemaRelationshipsStatus[key];
    }
    if (!isAllMode) {
      return schemaRelationshipsStatus[
        schemaRelationshipsKey(connectionId, schema)
      ];
    }
    const statuses = databaseSchemas.map(
      (name) =>
        schemaRelationshipsStatus[schemaRelationshipsKey(connectionId, name)],
    );
    return aggregateStatus(statuses);
  }, [
    isAllMode,
    isTableMode,
    schemaRelationshipsStatus,
    key,
    schema,
    connectionId,
    databaseSchemas,
  ]);

  useEffect(() => {
    if (!connectionId) return;
    if (isTableMode && tableScopeSchema != null && tableScopeTable != null) {
      void loadTableSchemaRelationships(
        connectionId,
        tableScopeSchema,
        tableScopeTable,
      );
      void loadSchemaMapPositions(connectionId, mapScope);
      void loadSchemaMapPrefs(connectionId, mapScope);
      return;
    }
    if (isAllMode) {
      for (const name of databaseSchemas) {
        void loadSchemaRelationships(connectionId, name);
      }
      void loadSchemaMapPositions(connectionId, mapScope);
      void loadSchemaMapPrefs(connectionId, mapScope);
      return;
    }
    if (schema) {
      void loadSchemaRelationships(connectionId, schema);
      void loadSchemaMapPositions(connectionId, mapScope);
      void loadSchemaMapPrefs(connectionId, mapScope);
    }
  }, [
    connectionId,
    schema,
    mapScope,
    isAllMode,
    isTableMode,
    tableScopeSchema,
    tableScopeTable,
    databaseSchemas,
    loadSchemaRelationships,
    loadTableSchemaRelationships,
    loadSchemaMapPositions,
    loadSchemaMapPrefs,
  ]);

  // Focused Table / Focused Relationship Edge. Cleared when the map
  // identity changes or the empty canvas is clicked.
  const [focus, setFocus] = useState<SchemaMapFocus | null>(null);
  useEffect(() => {
    setFocus(null);
  }, [key]);

  const graph = useMemo(() => {
    if (!relationships) {
      return { nodes: [], edges: [] };
    }
    return buildSchemaGraph(
      relationships.tables,
      relationships.foreignKeys,
      activeTable,
      {
        positions,
        prefs,
        // Table mode highlights exactly the scoped table — name-only
        // matching would also flag same-named tables in other schemas.
        activeTableId:
          tableScopeSchema != null && tableScopeTable != null
            ? `${tableScopeSchema}.${tableScopeTable}`
            : undefined,
      },
    );
  }, [
    relationships,
    activeTable,
    positions,
    prefs,
    tableScopeSchema,
    tableScopeTable,
  ]);

  const hasMultipleSchemas = useMemo(
    () => new Set(graph.nodes.map((node) => node.data.schema)).size > 1,
    [graph.nodes],
  );

  // Directly related graph elements for the current focus. `null`
  // means no focus — nothing dims. A focused element that no longer
  // exists after a graph reload also means no dimming, never an
  // all-dimmed map.
  const emphasis = useMemo(() => {
    if (!focus) return null;
    const tables = new Set<string>();
    const edges = new Set<string>();
    if (focus.kind === "table") {
      if (!graph.nodes.some((node) => node.id === focus.tableId)) {
        return null;
      }
      tables.add(focus.tableId);
      for (const edge of graph.edges) {
        if (edge.source === focus.tableId || edge.target === focus.tableId) {
          edges.add(edge.id);
          tables.add(edge.source);
          tables.add(edge.target);
        }
      }
      return { tables, edges };
    }
    const focusedEdge = graph.edges.find((edge) => edge.id === focus.edgeId);
    if (!focusedEdge) return null;
    edges.add(focusedEdge.id);
    tables.add(focusedEdge.source);
    tables.add(focusedEdge.target);
    return { tables, edges };
  }, [focus, graph.nodes, graph.edges]);

  const handleOpenTable = useCallback(
    (tableSchema: string, table: string) => {
      focusTableInSchemaMap(connectionId, tableSchema, table);
    },
    [connectionId, focusTableInSchemaMap],
  );

  const styledNodes = useMemo<Node<SchemaMapNodeData>[]>(
    () =>
      graph.nodes.map((node) => ({
        ...node,
        type: "schemaTable",
        style: { width: 220 },
        data: {
          ...node.data,
          hasMultipleSchemas,
          isDimmed: emphasis ? !emphasis.tables.has(node.id) : false,
          onOpenTable: handleOpenTable,
        },
      })),
    [graph.nodes, hasMultipleSchemas, emphasis, handleOpenTable],
  );

  const styledEdges = useMemo<SchemaGraphEdge[]>(
    () =>
      graph.edges.map((edge) => {
        const isFocused = focus?.kind === "edge" && focus.edgeId === edge.id;
        const isDimmed = emphasis ? !emphasis.edges.has(edge.id) : false;
        return {
          ...edge,
          selected: isFocused,
          style: {
            ...edge.style,
            strokeWidth: isFocused ? 2.4 : 1.45,
            opacity: isDimmed ? 0.15 : 1,
          },
          labelStyle: {
            ...edge.labelStyle,
            opacity: isDimmed ? 0.15 : 1,
          },
          labelBgStyle: {
            ...edge.labelBgStyle,
            fillOpacity: isDimmed ? 0.15 : 0.96,
          },
        };
      }),
    [graph.edges, focus, emphasis],
  );

  const [interactiveNodes, setInteractiveNodes] = useState(styledNodes);

  useEffect(() => {
    setInteractiveNodes(styledNodes);
  }, [styledNodes]);

  const onNodesChange = useCallback(
    (changes: NodeChange<Node<SchemaMapNodeData>>[]) => {
      setInteractiveNodes((currentNodes) =>
        applyNodeChanges(changes, currentNodes),
      );
    },
    [],
  );

  // Single click only focuses the Table Card; opening a table is the
  // explicit header action or a double-click.
  const onNodeClick: NodeMouseHandler = useCallback((_event, node) => {
    setFocus({ kind: "table", tableId: node.id });
  }, []);

  const onNodeDoubleClick: NodeMouseHandler = useCallback(
    (_event, node) => {
      // SAFETY: The value is constrained by the typed component or library contract at this boundary.
      const data = (node as Node<SchemaGraphNodeData>).data;
      if (!data) {
        return;
      }
      focusTableInSchemaMap(connectionId, data.schema, data.table);
    },
    [connectionId, focusTableInSchemaMap],
  );

  const onEdgeClick: EdgeMouseHandler<SchemaGraphEdge> = useCallback(
    (_event, edge) => {
      setFocus({ kind: "edge", edgeId: edge.id });
    },
    [],
  );

  const onPaneClick = useCallback(() => {
    setFocus(null);
  }, []);

  const onNodeDragStop: OnNodeDrag = useCallback(
    (_event, node) => {
      if (!connectionId || !mapScope) {
        return;
      }
      void saveSchemaMapPosition(
        connectionId,
        mapScope,
        node.id,
        node.position.x,
        node.position.y,
      );
    },
    [connectionId, mapScope, saveSchemaMapPosition],
  );

  const focusedForeignKey: SchemaForeignKey | undefined =
    focus?.kind === "edge"
      ? graph.edges.find((edge) => edge.id === focus.edgeId)?.data?.foreignKey
      : undefined;

  const hasNodes = graph.nodes.length > 0;
  const errorMessage = status?.state === "error" ? status.error : null;
  const flowContainerRef = useRef<HTMLDivElement>(null);
  const [flowSizeKey, setFlowSizeKey] = useState("initial");

  useImperativeHandle(
    ref,
    () => ({
      exportImage: (format, filename) =>
        exportSchemaMapImage(flowContainerRef.current, format, filename),
    }),
    [],
  );

  useEffect(() => {
    // oxlint-disable-next-line anti-slop/no-runtime-typeof -- The value is handled at a typed library or domain boundary here.
    if (!isClient || !hasNodes || typeof ResizeObserver === "undefined") {
      return;
    }

    const container = flowContainerRef.current;
    if (!container) {
      return;
    }

    let frame = 0;
    const observer = new ResizeObserver(([entry]) => {
      window.cancelAnimationFrame(frame);
      frame = window.requestAnimationFrame(() => {
        const width = Math.round(entry.contentRect.width / 24);
        const height = Math.round(entry.contentRect.height / 24);
        setFlowSizeKey(`${width}x${height}`);
      });
    });

    observer.observe(container);

    return () => {
      window.cancelAnimationFrame(frame);
      observer.disconnect();
    };
  }, [isClient, hasNodes]);

  // A load error only replaces the map when there is nothing to draw.
  // With renderable (partial or cached) data, the map stays up and the
  // failure surfaces as a non-blocking banner instead.
  if (errorMessage && !hasNodes) {
    return (
      <div
        data-testid="schema-flow-error"
        role="alert"
        className="flex h-full items-center justify-center border border-danger/40 bg-danger/10 px-2 py-1 text-2xs text-danger"
      >
        Failed to load relationships: {errorMessage}
      </div>
    );
  }

  const policy =
    engine && storageClassFor(engine) === "relational"
      ? relationalPolicy(engine)
      : null;
  const noFkBanner =
    policy?.schemaMapNoForeignKeysCopy &&
    hasNodes &&
    (relationships?.foreignKeys.length ?? 0) === 0
      ? policy.schemaMapNoForeignKeysCopy
      : null;

  return (
    <div ref={flowContainerRef} className="relative h-full w-full">
      {focusedForeignKey ? (
        <RelationshipDetailPopover
          foreignKey={focusedForeignKey}
          onClose={() => setFocus(null)}
        />
      ) : null}
      {errorMessage && hasNodes ? (
        <div
          data-testid="schema-flow-partial-error"
          role="alert"
          className="absolute bottom-2 left-2 right-2 z-10 rounded-md border border-danger/40 bg-danger/10 px-3 py-1.5 text-2xs text-danger shadow-sm"
        >
          Some relationships failed to load: {errorMessage}
        </div>
      ) : null}
      {noFkBanner ? (
        <div
          data-testid="schema-flow-clickhouse-banner"
          className="absolute left-2 right-2 top-2 z-10 rounded-md border border-border/60 bg-background/90 px-3 py-1.5 text-2xs text-muted-foreground shadow-sm"
        >
          {noFkBanner}
        </div>
      ) : null}
      {!hasNodes ? (
        <div
          data-testid="schema-flow-empty"
          className="flex h-full items-center justify-center text-xs text-muted-foreground"
        >
          {status?.state === "loading"
            ? "Loading relationships..."
            : "No relationships to display"}
        </div>
      ) : isClient ? (
        <ReactFlow
          key={`${key}:${flowSizeKey}`}
          nodes={interactiveNodes}
          edges={styledEdges}
          nodeTypes={nodeTypes}
          fitView
          defaultEdgeOptions={{
            style: { stroke: "var(--primary)", strokeWidth: 1.45 },
          }}
          fitViewOptions={{ padding: 0.18, maxZoom: 1 }}
          nodesConnectable={false}
          onNodesChange={onNodesChange}
          onNodeClick={onNodeClick}
          onNodeDoubleClick={onNodeDoubleClick}
          onEdgeClick={onEdgeClick}
          onPaneClick={onPaneClick}
          onNodeDragStop={onNodeDragStop}
          proOptions={{ hideAttribution: true }}
        >
          <CrowsFootMarkers />
          <Background gap={18} size={0.6} color="var(--border)" />
          <Controls showInteractive={false} />
        </ReactFlow>
      ) : (
        <div className="flex h-full items-center justify-center text-xs text-muted-foreground">
          Loading schema map...
        </div>
      )}
    </div>
  );
});
