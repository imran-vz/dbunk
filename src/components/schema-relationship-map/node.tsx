import {
  IconArrowUpRight,
  IconBolt,
  IconExternalLink,
} from "@tabler/icons-react";
import { Handle, type Node, type NodeProps, Position } from "@xyflow/react";

import type {
  SchemaGraphNodeData,
  SchemaTableTrigger,
} from "@/lib/schema-graph";
import { cn } from "@/lib/utils";

export type SchemaMapNodeData = SchemaGraphNodeData & {
  hasMultipleSchemas: boolean;
  isDimmed: boolean;
  onOpenTable?: (schema: string, table: string) => void;
};

const triggerSummary = (triggers: SchemaTableTrigger[]): string =>
  triggers
    .map(
      (trigger) =>
        `${trigger.name}: ${trigger.timing} ${trigger.events.join(
          " OR ",
        )} FOR EACH ${trigger.orientation} -> ${trigger.functionName}${
          trigger.enabled ? "" : " (disabled)"
        }`,
    )
    .join("\n");

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

export function SchemaTableNode({ data }: NodeProps<Node<SchemaMapNodeData>>) {
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
            title="Junction Table Card - this table joins a many-to-many relationship"
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
