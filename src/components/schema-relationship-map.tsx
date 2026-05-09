import { useEffect, useMemo } from "react";
import ReactFlow, {
  Background,
  Controls,
  type Node,
  type NodeMouseHandler,
} from "reactflow";
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
  } = useAppStore();

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
        style: { width: "auto" },
        className: cn(
          "rounded-md border bg-card px-2 py-1 text-[0.65rem] shadow-sm whitespace-nowrap",
          node.data.isActive
            ? "border-primary bg-primary/10 text-primary"
            : node.data.isExternal
              ? "border-dashed border-border text-muted-foreground"
              : "border-border text-foreground",
        ),
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

  return (
    <div className="relative h-full w-full">
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
          nodes={styledNodes}
          edges={graph.edges}
          fitView
          nodesConnectable={false}
          onNodeClick={onNodeClick}
        >
          <Background gap={16} size={0.5} />
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
