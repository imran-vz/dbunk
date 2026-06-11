import {
  IconArrowUpRight,
  IconBolt,
  IconExternalLink,
} from "@tabler/icons-react";
import {
  applyNodeChanges,
  Background,
  Controls,
  type EdgeMouseHandler,
  Handle,
  type Node,
  type NodeChange,
  type NodeMouseHandler,
  type NodeProps,
  type OnNodeDrag,
  Position,
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
import { downloadDataUrl } from "@/lib/download";
import { relationalPolicy, storageClassFor } from "@/lib/engine-policy";
import {
  buildSchemaGraph,
  DEFAULT_SCHEMA_MAP_PREFS,
  filterSchemaRelationshipsForTable,
  isAllSchemas,
  type SchemaForeignKey,
  type SchemaGraphEdge,
  type SchemaGraphNodeData,
  type SchemaMapPosition,
  type SchemaRelationships,
  type SchemaTableNode as SchemaTableNodeData,
  type SchemaTableTrigger,
  schemaRelationshipsKey,
  tableSchemaMapScope,
} from "@/lib/schema-graph";
import { type SchemaRelationshipsStatus, useAppStore } from "@/lib/store";
import { cn } from "@/lib/utils";

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
    options?: Record<string, unknown>,
  ) => Promise<string>;
  toSvg: (
    node: HTMLElement,
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

export const typeGlyph = (dataType: string): string => {
  const normalized = dataType.toLowerCase();
  if (
    normalized.includes("int") ||
    normalized.includes("numeric") ||
    normalized.includes("decimal") ||
    normalized.includes("real") ||
    normalized.includes("double")
  ) {
    return "123";
  }
  if (normalized.includes("bool")) {
    return "T/F";
  }
  if (
    normalized.includes("date") ||
    normalized.includes("time") ||
    normalized.includes("timestamp")
  ) {
    return "time";
  }
  if (normalized.includes("json")) {
    return "{}";
  }
  return "A-Z";
};

const triggerSummary = (triggers: SchemaTableTrigger[]): string =>
  triggers
    .map(
      (trigger) =>
        `${trigger.name}: ${trigger.timing} ${trigger.events.join(
          " OR ",
        )} FOR EACH ${trigger.orientation} → ${trigger.functionName}${
          trigger.enabled ? "" : " (disabled)"
        }`,
    )
    .join("\n");

function SchemaTableNode({ data }: NodeProps<Node<SchemaGraphNodeData>>) {
  const fkColumnNames = new Set(data.fkColumnNames);
  const hiddenColumnCount =
    data.prefs.attrMode === "none" ? data.columnCount : 0;
  const showSchemaPrefix = data.hasMultipleSchemas || data.isExternal;
  const columnTriggers = new Map<string, SchemaTableTrigger[]>();
  for (const trigger of data.triggers) {
    for (const column of trigger.columns ?? []) {
      const existing = columnTriggers.get(column) ?? [];
      existing.push(trigger);
      columnTriggers.set(column, existing);
    }
  }

  return (
    <div
      data-active={data.isActive ? "true" : "false"}
      data-external={data.isExternal ? "true" : "false"}
      data-dimmed={data.isDimmed ? "true" : "false"}
      className={cn(
        "group/schema-node w-[220px] overflow-hidden rounded-md border bg-card text-[0.625rem] text-card-foreground shadow-sm ring-1 ring-background/80 transition-opacity",
        data.isActive
          ? "border-primary shadow-primary/20"
          : data.isExternal
            ? "border-dashed border-warning/60 bg-card/80"
            : "border-primary/80",
        data.isDimmed && "opacity-30",
      )}
    >
      <div
        className={cn(
          "flex items-center gap-1.5 border-b px-2 py-1 text-[0.68rem] font-medium",
          data.isExternal
            ? "border-warning/40 bg-warning/10"
            : "border-primary/70 bg-muted/70",
        )}
      >
        {data.isExternal ? (
          <IconExternalLink
            className="size-2.5 shrink-0 text-warning"
            aria-hidden
          />
        ) : (
          <span className="text-primary">tbl</span>
        )}
        <span
          className="min-w-0 flex-1 truncate"
          title={`${data.schema}.${data.table}`}
        >
          {showSchemaPrefix ? (
            <span className="text-muted-foreground">{data.schema}.</span>
          ) : null}
          {data.label}
        </span>
        {data.isJunctionTable ? (
          <span
            data-testid={`junction-table-indicator-${data.tableId}`}
            title="Junction Table Card — this table joins a many-to-many relationship"
            className="shrink-0 rounded-sm border border-primary/40 bg-primary/15 px-1 py-px text-[0.5rem] uppercase tracking-wide text-primary"
          >
            M:N
          </span>
        ) : null}
        {data.triggers.length > 0 ? (
          <span
            data-testid={`trigger-indicator-table-${data.tableId}`}
            title={triggerSummary(data.triggers)}
            className="flex shrink-0 items-center gap-0.5 rounded-sm border border-warning/40 bg-warning/10 px-1 py-px text-[0.5rem] text-warning"
          >
            <IconBolt className="size-2.5" aria-hidden />
            {data.triggers.length}
          </span>
        ) : null}
        {data.isExternal ? (
          <span className="shrink-0 rounded-sm border border-warning/40 bg-warning/15 px-1 py-px text-[0.5rem] uppercase tracking-wide text-warning">
            external
          </span>
        ) : null}
        {!data.isExternal && data.onOpenTable ? (
          <button
            type="button"
            aria-label={`Open table ${data.schema}.${data.table}`}
            title="Open table"
            className="shrink-0 rounded-sm p-0.5 text-muted-foreground hover:bg-muted hover:text-foreground"
            onClick={(event) => {
              event.stopPropagation();
              data.onOpenTable?.(data.schema, data.table);
            }}
          >
            <IconArrowUpRight className="size-3" aria-hidden />
          </button>
        ) : null}
      </div>
      <div className="max-h-80 overflow-auto bg-card">
        {data.columns.length > 0 ? (
          data.columns.map((column) => {
            const hasHandle = fkColumnNames.has(column.name);
            const glyph = column.isPrimaryKey
              ? "PK"
              : hasHandle
                ? "FK"
                : data.prefs.showTypes
                  ? typeGlyph(column.dataType)
                  : "";
            return (
              <div
                key={column.name}
                className={cn(
                  "relative grid grid-cols-[2.25rem_minmax(0,1fr)_auto] items-center gap-1 border-b border-border/50 px-1.5 py-1 last:border-b-0",
                  column.isPrimaryKey && "bg-primary/10",
                )}
              >
                {hasHandle ? (
                  <>
                    <Handle
                      id={`${data.tableId}.${column.name}.left`}
                      type="target"
                      position={Position.Left}
                      className="!-left-1 !size-2 !border-primary !bg-card"
                    />
                    <Handle
                      id={`${data.tableId}.${column.name}.right`}
                      type="source"
                      position={Position.Right}
                      className="!-right-1 !size-2 !border-primary !bg-card"
                    />
                  </>
                ) : null}
                <span
                  className={cn(
                    "flex items-center gap-1 text-[0.56rem] leading-none text-primary",
                    !glyph && "text-transparent",
                  )}
                >
                  {hasHandle ? (
                    <span className="size-1.5 rounded-full bg-primary/80" />
                  ) : null}
                  {glyph || "-"}
                </span>
                <span
                  className={cn(
                    "min-w-0 truncate",
                    column.nullable && "text-muted-foreground",
                  )}
                >
                  <span className="flex items-center gap-1">
                    <span className="truncate">{column.name}</span>
                    {columnTriggers.has(column.name) ? (
                      <span
                        data-testid={`trigger-indicator-column-${data.tableId}.${column.name}`}
                        title={triggerSummary(
                          columnTriggers.get(column.name) ?? [],
                        )}
                        className="shrink-0 text-warning"
                      >
                        <IconBolt className="size-2.5" aria-hidden />
                      </span>
                    ) : null}
                  </span>
                  {data.prefs.showComments && column.comment ? (
                    <span
                      className="block truncate text-[0.56rem] text-muted-foreground"
                      title={column.comment}
                    >
                      {column.comment}
                    </span>
                  ) : null}
                </span>
                {data.prefs.showNulls ? (
                  <span className="rounded-sm border border-border/70 px-1 text-[0.52rem] leading-4 text-muted-foreground">
                    {column.nullable ? "?" : "NN"}
                  </span>
                ) : null}
              </div>
            );
          })
        ) : (
          <div className="px-2 py-1.5 text-muted-foreground">
            {data.isExternal
              ? "External table"
              : `${hiddenColumnCount || data.columnCount} columns`}
          </div>
        )}
      </div>
    </div>
  );
}

const nodeTypes = {
  schemaTable: SchemaTableNode,
};

function CrowsFootMarkers() {
  return (
    <svg
      aria-hidden="true"
      className="pointer-events-none absolute size-0 overflow-hidden"
    >
      <defs>
        <marker
          id="crowsfoot-one"
          markerHeight="16"
          markerUnits="strokeWidth"
          markerWidth="16"
          orient="auto"
          refX="8"
          refY="8"
          viewBox="0 0 16 16"
        >
          <path
            d="M5 2.5v11M9 2.5v11"
            fill="none"
            stroke="var(--primary)"
            strokeLinecap="round"
            strokeWidth="1.8"
          />
        </marker>
        <marker
          id="crowsfoot-zero-or-one"
          markerHeight="18"
          markerUnits="strokeWidth"
          markerWidth="20"
          orient="auto"
          refX="10"
          refY="9"
          viewBox="0 0 20 18"
        >
          <circle
            cx="6"
            cy="9"
            fill="var(--card)"
            r="3.2"
            stroke="var(--primary)"
            strokeWidth="1.6"
          />
          <path
            d="M12 3.5v11"
            fill="none"
            stroke="var(--primary)"
            strokeLinecap="round"
            strokeWidth="1.8"
          />
        </marker>
        <marker
          id="crowsfoot-many"
          markerHeight="20"
          markerUnits="strokeWidth"
          markerWidth="24"
          orient="auto"
          refX="12"
          refY="10"
          viewBox="0 0 24 20"
        >
          <circle
            cx="6"
            cy="10"
            fill="var(--card)"
            r="3"
            stroke="var(--primary)"
            strokeWidth="1.5"
          />
          <path
            d="M12 10l8-6M12 10l8 6M12 10h8"
            fill="none"
            stroke="var(--primary)"
            strokeLinecap="round"
            strokeLinejoin="round"
            strokeWidth="1.8"
          />
        </marker>
        <marker
          id="crowsfoot-unknown"
          markerHeight="18"
          markerUnits="strokeWidth"
          markerWidth="18"
          orient="auto"
          refX="9"
          refY="9"
          viewBox="0 0 18 18"
        >
          <rect
            fill="var(--card)"
            height="6.4"
            stroke="var(--primary)"
            strokeWidth="1.5"
            transform="rotate(45 9 9)"
            width="6.4"
            x="5.8"
            y="5.8"
          />
        </marker>
        {/* Start-marker variants: a start marker with plain
            orient="auto" renders mirrored, pointing the glyph away
            from its Table Card. */}
        <marker
          id="crowsfoot-one-start"
          markerHeight="16"
          markerUnits="strokeWidth"
          markerWidth="16"
          orient="auto-start-reverse"
          refX="8"
          refY="8"
          viewBox="0 0 16 16"
        >
          <path
            d="M5 2.5v11M9 2.5v11"
            fill="none"
            stroke="var(--primary)"
            strokeLinecap="round"
            strokeWidth="1.8"
          />
        </marker>
        <marker
          id="crowsfoot-many-start"
          markerHeight="20"
          markerUnits="strokeWidth"
          markerWidth="24"
          orient="auto-start-reverse"
          refX="12"
          refY="10"
          viewBox="0 0 24 20"
        >
          <circle
            cx="6"
            cy="10"
            fill="var(--card)"
            r="3"
            stroke="var(--primary)"
            strokeWidth="1.5"
          />
          <path
            d="M12 10l8-6M12 10l8 6M12 10h8"
            fill="none"
            stroke="var(--primary)"
            strokeLinecap="round"
            strokeLinejoin="round"
            strokeWidth="1.8"
          />
        </marker>
        <marker
          id="crowsfoot-unknown-start"
          markerHeight="18"
          markerUnits="strokeWidth"
          markerWidth="18"
          orient="auto-start-reverse"
          refX="9"
          refY="9"
          viewBox="0 0 18 18"
        >
          <rect
            fill="var(--card)"
            height="6.4"
            stroke="var(--primary)"
            strokeWidth="1.5"
            transform="rotate(45 9 9)"
            width="6.4"
            x="5.8"
            y="5.8"
          />
        </marker>
      </defs>
    </svg>
  );
}

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
  // The Table-Level Schema Map needs every schema's payload — direct
  // neighbors can live in other schemas (cross-schema FKs in either
  // direction), and incoming FKs only appear in the child's schema
  // payload. The store caches per schema, so this shares data with the
  // global map.
  const databaseSchemas = useMemo(() => {
    if (!isAllMode && !isTableMode) return [];
    const names = new Set(
      (schemaExplorer[connectionId] ?? []).map((entry) => entry.name),
    );
    if (tableScopeSchema) {
      names.add(tableScopeSchema);
    }
    return [...names].sort();
  }, [isAllMode, isTableMode, schemaExplorer, connectionId, tableScopeSchema]);

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
  // and table-level modes — the merge produces a fresh object every
  // call, and feeding that into downstream `useMemo`s without
  // memoising here triggers an infinite render loop via the
  // `setInteractiveNodes(styledNodes)` effect below.
  const relationships = useMemo<SchemaRelationships | undefined>(() => {
    if (!isAllMode && !isTableMode) {
      return schemaRelationships[schemaRelationshipsKey(connectionId, schema)];
    }
    const entries = databaseSchemas.map(
      (name) => schemaRelationships[schemaRelationshipsKey(connectionId, name)],
    );
    if (tableScopeSchema != null && tableScopeTable != null) {
      // Table-Level Schema Map: render as soon as the table's own
      // schema payload exists. Cross-schema neighbors join the map as
      // their schemas load; one slow or failed schema must not blank
      // the whole map.
      const focusEntry =
        schemaRelationships[
          schemaRelationshipsKey(connectionId, tableScopeSchema)
        ];
      if (!focusEntry) return undefined;
      return filterSchemaRelationshipsForTable(
        mergeDefinedSchemaRelationships(entries),
        tableScopeSchema,
        tableScopeTable,
      );
    }
    return mergeSchemaRelationships(entries);
  }, [
    isAllMode,
    isTableMode,
    schemaRelationships,
    schema,
    connectionId,
    databaseSchemas,
    tableScopeSchema,
    tableScopeTable,
  ]);

  const status = useMemo<SchemaRelationshipsStatus | undefined>(() => {
    if (!isAllMode && !isTableMode) {
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
    schema,
    connectionId,
    databaseSchemas,
  ]);

  useEffect(() => {
    if (!connectionId) return;
    if (isAllMode || isTableMode) {
      for (const name of databaseSchemas) {
        if (isTableMode) {
          // The table-level subtab remounts on every activation; an
          // unguarded fan-out would re-run every schema's catalog
          // queries per click. Loaded and in-flight schemas are
          // skipped — the cache drops on disconnect/delete.
          const entryKey = schemaRelationshipsKey(connectionId, name);
          const current = useAppStore.getState();
          if (
            current.schemaRelationships[entryKey] ||
            current.schemaRelationshipsStatus[entryKey]?.state === "loading"
          ) {
            continue;
          }
        }
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
    databaseSchemas,
    loadSchemaRelationships,
    loadSchemaMapPositions,
    loadSchemaMapPrefs,
  ]);

  // Focused Table / Focused Relationship Edge. Cleared when the map
  // identity changes or the empty canvas is clicked.
  const [focus, setFocus] = useState<SchemaMapFocus | null>(null);
  // biome-ignore lint/correctness/useExhaustiveDependencies(key): focus must reset exactly when the map identity (connection + scope) changes
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

  const styledNodes = useMemo<Node<SchemaGraphNodeData>[]>(
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
          data: edge.data ? { ...edge.data, isDimmed, isFocused } : edge.data,
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
    (changes: NodeChange<Node<SchemaGraphNodeData>>[]) => {
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
        className="flex h-full items-center justify-center border border-destructive/40 bg-destructive/10 px-2 py-1 text-[0.65rem] text-destructive"
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
          className="absolute bottom-2 left-2 right-2 z-10 rounded-md border border-destructive/40 bg-destructive/10 px-3 py-1.5 text-[0.65rem] text-destructive shadow-sm"
        >
          Some relationships failed to load: {errorMessage}
        </div>
      ) : null}
      {noFkBanner ? (
        <div
          data-testid="schema-flow-clickhouse-banner"
          className="absolute left-2 right-2 top-2 z-10 rounded-md border border-border/60 bg-background/90 px-3 py-1.5 text-[0.65rem] text-muted-foreground shadow-sm"
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
