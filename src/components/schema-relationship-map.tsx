import {
  forwardRef,
  useCallback,
  useEffect,
  useImperativeHandle,
  useMemo,
  useRef,
  useState,
} from "react";
import ReactFlow, {
  applyNodeChanges,
  Background,
  Controls,
  Handle,
  type Node,
  type NodeChange,
  type NodeDragHandler,
  type NodeMouseHandler,
  type NodeProps,
  Position,
} from "reactflow";

import { downloadDataUrl } from "@/lib/download";
import { relationalPolicy, storageClassFor } from "@/lib/engine-policy";
import {
  buildSchemaGraph,
  DEFAULT_SCHEMA_MAP_PREFS,
  type SchemaGraphNodeData,
  type SchemaMapPosition,
  schemaRelationshipsKey,
} from "@/lib/schema-graph";
import { useAppStore } from "@/lib/store";
import { cn } from "@/lib/utils";

interface SchemaRelationshipMapProps {
  connectionId: string;
  schema: string;
  activeTable: string | null;
  isClient?: boolean;
}

const EMPTY_POSITIONS: Record<string, SchemaMapPosition> = {};

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

function SchemaTableNode({ data }: NodeProps<SchemaGraphNodeData>) {
  const fkColumnNames = new Set(data.fkColumnNames);
  const hiddenColumnCount =
    data.prefs.attrMode === "none" ? data.columnCount : 0;

  return (
    <div
      data-active={data.isActive ? "true" : "false"}
      className={cn(
        "group/schema-node w-[220px] overflow-hidden rounded-md border bg-card text-[0.625rem] text-card-foreground shadow-sm ring-1 ring-background/80",
        data.isActive
          ? "border-primary shadow-primary/20"
          : data.isExternal
            ? "border-dashed border-primary/50"
            : "border-primary/80",
      )}
    >
      <div className="flex items-center justify-center gap-1.5 border-b border-primary/70 bg-muted/70 px-2 py-1 text-center text-[0.68rem] font-medium">
        <span className="text-primary">tbl</span>
        <span className="truncate">{data.label}</span>
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
                  <span className="truncate">{column.name}</span>
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
      </defs>
    </svg>
  );
}

export const SchemaRelationshipMap = forwardRef<
  SchemaRelationshipMapHandle,
  SchemaRelationshipMapProps
>(function SchemaRelationshipMap(
  { connectionId, schema, activeTable, isClient = true },
  ref,
) {
  const {
    schemaRelationships,
    schemaRelationshipsStatus,
    loadSchemaRelationships,
    focusTableInSchemaMap,
    connections,
    schemaMapPositions,
    loadSchemaMapPositions,
    saveSchemaMapPosition,
    schemaMapPrefs,
    loadSchemaMapPrefs,
  } = useAppStore();
  const engine = connections.find(
    (connection) => connection.id === connectionId,
  )?.engine;

  const key = schemaRelationshipsKey(connectionId, schema);
  const relationships = schemaRelationships[key];
  const status = schemaRelationshipsStatus[key];
  const positions =
    schemaMapPositions[connectionId]?.[schema] ?? EMPTY_POSITIONS;
  const prefs =
    schemaMapPrefs[connectionId]?.[schema] ?? DEFAULT_SCHEMA_MAP_PREFS;

  useEffect(() => {
    if (connectionId && schema) {
      void loadSchemaRelationships(connectionId, schema);
      void loadSchemaMapPositions(connectionId, schema);
      void loadSchemaMapPrefs(connectionId, schema);
    }
  }, [
    connectionId,
    schema,
    loadSchemaRelationships,
    loadSchemaMapPositions,
    loadSchemaMapPrefs,
  ]);

  const graph = useMemo(() => {
    if (!relationships) {
      return { nodes: [], edges: [] };
    }
    return buildSchemaGraph(
      relationships.tables,
      relationships.foreignKeys,
      activeTable,
      { positions, prefs },
    );
  }, [relationships, activeTable, positions, prefs]);

  const styledNodes = useMemo<Node<SchemaGraphNodeData>[]>(
    () =>
      graph.nodes.map((node) => ({
        ...node,
        type: "schemaTable",
        style: { width: 220 },
      })),
    [graph.nodes],
  );
  const [interactiveNodes, setInteractiveNodes] = useState(styledNodes);

  useEffect(() => {
    setInteractiveNodes(styledNodes);
  }, [styledNodes]);

  const onNodesChange = useCallback((changes: NodeChange[]) => {
    setInteractiveNodes((currentNodes) =>
      applyNodeChanges(changes, currentNodes),
    );
  }, []);

  const onNodeClick: NodeMouseHandler = (_event, node) => {
    const data = (node as Node<SchemaGraphNodeData>).data;
    if (!data) {
      return;
    }
    focusTableInSchemaMap(connectionId, data.schema, data.table);
  };

  const onNodeDragStop: NodeDragHandler = useCallback(
    (_event, node) => {
      if (!connectionId || !schema) {
        return;
      }
      void saveSchemaMapPosition(
        connectionId,
        schema,
        node.id,
        node.position.x,
        node.position.y,
      );
    },
    [connectionId, schema, saveSchemaMapPosition],
  );

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

  if (errorMessage) {
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
          edges={graph.edges}
          nodeTypes={nodeTypes}
          fitView
          defaultEdgeOptions={{
            style: { stroke: "var(--primary)", strokeWidth: 1.45 },
          }}
          fitViewOptions={{ padding: 0.18, maxZoom: 1 }}
          nodesConnectable={false}
          onNodesChange={onNodesChange}
          onNodeClick={onNodeClick}
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
