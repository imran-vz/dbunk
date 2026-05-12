import { useEffect, useMemo, useRef, useState } from "react";
import ReactFlow, {
  Background,
  Controls,
  Handle,
  type Node,
  type NodeMouseHandler,
  type NodeProps,
  Position,
} from "reactflow";
import { relationalPolicy, storageClassFor } from "@/lib/engine-policy";
import {
  buildSchemaGraph,
  type SchemaGraphNodeData,
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

const visibleColumnLimit = 18;

const typeGlyph = (dataType: string): string => {
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
  const visibleColumns = data.columns.slice(0, visibleColumnLimit);
  const hiddenColumnCount = Math.max(
    0,
    data.columnCount - visibleColumns.length,
  );

  return (
    <div
      data-active={data.isActive ? "true" : "false"}
      className={cn(
        "group/schema-node w-52 overflow-hidden rounded-md border bg-card text-[0.625rem] text-card-foreground shadow-sm ring-1 ring-background/80",
        data.isActive
          ? "border-primary shadow-primary/20"
          : data.isExternal
            ? "border-dashed border-primary/50"
            : "border-primary/80",
      )}
    >
      <Handle
        type="target"
        position={Position.Left}
        className="!size-2 !border-primary !bg-primary/70"
      />
      <Handle
        type="source"
        position={Position.Right}
        className="!size-2 !border-primary !bg-primary/70"
      />
      <div className="flex items-center justify-center gap-1.5 border-b border-primary/70 bg-muted/70 px-2 py-1 text-center text-[0.68rem] font-medium">
        <span className="text-primary">tbl</span>
        <span className="truncate">{data.label}</span>
      </div>
      <div className="max-h-72 overflow-auto bg-card">
        {visibleColumns.length > 0 ? (
          visibleColumns.map((column) => (
            <div
              key={column.name}
              className={cn(
                "grid grid-cols-[2rem_minmax(0,1fr)] items-center border-b border-border/50 px-1.5 py-0.5 last:border-b-0",
                column.isPrimaryKey && "bg-primary/10",
              )}
            >
              <span className="text-[0.56rem] leading-none text-primary">
                {column.isPrimaryKey ? "PK" : typeGlyph(column.dataType)}
              </span>
              <span
                className={cn(
                  "truncate",
                  column.nullable && "text-muted-foreground",
                )}
              >
                {column.name}
              </span>
            </div>
          ))
        ) : (
          <div className="px-2 py-1.5 text-muted-foreground">
            {data.isExternal ? "External table" : `${data.columnCount} columns`}
          </div>
        )}
        {hiddenColumnCount > 0 ? (
          <div className="border-t border-primary/40 px-2 py-1 text-muted-foreground">
            + {hiddenColumnCount} more
          </div>
        ) : null}
      </div>
    </div>
  );
}

const nodeTypes = {
  schemaTable: SchemaTableNode,
};

export function SchemaRelationshipMap({
  connectionId,
  schema,
  activeTable,
  isClient = true,
}: SchemaRelationshipMapProps) {
  const {
    schemaRelationships,
    schemaRelationshipsStatus,
    loadSchemaRelationships,
    focusTableInSchemaMap,
    connections,
  } = useAppStore();
  const engine = connections.find(
    (connection) => connection.id === connectionId,
  )?.engine;

  const key = schemaRelationshipsKey(connectionId, schema);
  const relationships = schemaRelationships[key];
  const status = schemaRelationshipsStatus[key];

  // Eager-load on mount (and on key change). The store action is a no-op
  // outside Tauri and is idempotent at the connection level, so it is safe
  // to call here.
  useEffect(() => {
    if (connectionId && schema) {
      void loadSchemaRelationships(connectionId, schema);
    }
  }, [connectionId, schema, loadSchemaRelationships]);

  const graph = useMemo(() => {
    if (!relationships) {
      return { nodes: [], edges: [] };
    }
    return buildSchemaGraph(
      relationships.tables,
      relationships.foreignKeys,
      activeTable,
    );
  }, [relationships, activeTable]);

  // Decorate nodes with classes that highlight the active one. We keep the
  // `data-active` attribute on the wrapper element so downstream tests and
  // styling can target it without relying on internal class strings.
  const styledNodes = useMemo(
    () =>
      graph.nodes.map((node) => ({
        ...node,
        type: "schemaTable",
        style: { width: 208 },
      })),
    [graph.nodes],
  );

  const onNodeClick: NodeMouseHandler = (_event, node) => {
    const data = (node as Node<SchemaGraphNodeData>).data;
    if (!data) {
      return;
    }
    focusTableInSchemaMap(connectionId, data.schema, data.table);
  };

  const hasNodes = graph.nodes.length > 0;
  const errorMessage = status?.state === "error" ? status.error : null;
  const flowContainerRef = useRef<HTMLDivElement>(null);
  const [flowSizeKey, setFlowSizeKey] = useState("initial");

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

  // Engine policy carries the "no foreign keys" banner copy. Only
  // engines with hasForeignKeys=false populate it (CH today). The
  // schema-relationship map is relational-only so we narrow safely;
  // Redis connections never render this component.
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
          nodes={styledNodes}
          edges={graph.edges}
          nodeTypes={nodeTypes}
          fitView
          defaultEdgeOptions={{
            style: { stroke: "var(--primary)", strokeWidth: 1.25 },
          }}
          fitViewOptions={{ padding: 0.16, maxZoom: 1 }}
          nodesConnectable={false}
          onNodeClick={onNodeClick}
          proOptions={{ hideAttribution: true }}
        >
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
}
