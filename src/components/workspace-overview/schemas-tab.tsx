import {
  IconArrowDown,
  IconArrowsSort,
  IconArrowUp,
  IconMap,
  IconRefresh,
} from "@tabler/icons-react";
import { useEffect, useMemo, useState } from "react";

import { Button } from "@/components/ui/button";
import {
  Card,
  CardAction,
  CardContent,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";
import type {
  Connection,
  RelationInfo,
  RelationStatsStatus,
} from "@/lib/store";
import { cn } from "@/lib/utils";

import { formatBytes, formatRowCount } from "./format";

type SortColumn = "schema" | "tables" | "views" | "matviews" | "size";
type SortDirection = "asc" | "desc";

type SchemaRow = {
  schema: string;
  tableCount: number;
  viewCount: number;
  matviewCount: number;
  totalSizeBytes: number;
  rowCountEstimate: number;
};

export function SchemasTab({
  activeConnection,
  relationStats,
  relationStatsStatus,
  onLoadRelationStats,
  onSelectSchema,
  onViewSchemaMap,
}: {
  activeConnection: Connection;
  relationStats: RelationInfo[] | undefined;
  relationStatsStatus: RelationStatsStatus | undefined;
  onLoadRelationStats: (connectionId: string) => Promise<void>;
  onSelectSchema: (schema: string) => void;
  onViewSchemaMap: (schema: string) => void;
}) {
  const [sortColumn, setSortColumn] = useState<SortColumn>("schema");
  const [sortDirection, setSortDirection] = useState<SortDirection>("asc");

  useEffect(() => {
    const status = relationStatsStatus?.state;
    if (status === "loading" || status === "success") {
      return;
    }
    void onLoadRelationStats(activeConnection.id);
  }, [activeConnection.id, onLoadRelationStats, relationStatsStatus?.state]);

  const schemaRows = useMemo<SchemaRow[]>(() => {
    const map = new Map<string, SchemaRow>();
    for (const rel of relationStats ?? []) {
      let row = map.get(rel.schema);
      if (!row) {
        row = {
          schema: rel.schema,
          tableCount: 0,
          viewCount: 0,
          matviewCount: 0,
          totalSizeBytes: 0,
          rowCountEstimate: 0,
        };
        map.set(rel.schema, row);
      }
      if (rel.kind === "table") {
        row.tableCount += 1;
      } else if (rel.kind === "view") {
        row.viewCount += 1;
      } else if (rel.kind === "materialized view") {
        row.matviewCount += 1;
      }
      row.totalSizeBytes += rel.totalSizeBytes;
      row.rowCountEstimate += rel.rowCountEstimate;
    }
    return Array.from(map.values());
  }, [relationStats]);

  const sorted = useMemo(() => {
    const list = [...schemaRows];
    list.sort((a, b) => {
      const factor = sortDirection === "asc" ? 1 : -1;
      if (sortColumn === "schema") {
        return a.schema.localeCompare(b.schema) * factor;
      }
      if (sortColumn === "tables") {
        return (a.tableCount - b.tableCount) * factor;
      }
      if (sortColumn === "views") {
        return (a.viewCount - b.viewCount) * factor;
      }
      if (sortColumn === "matviews") {
        return (a.matviewCount - b.matviewCount) * factor;
      }
      return (a.totalSizeBytes - b.totalSizeBytes) * factor;
    });
    return list;
  }, [schemaRows, sortColumn, sortDirection]);

  const handleSort = (column: SortColumn) => {
    if (column === sortColumn) {
      setSortDirection((dir) => (dir === "asc" ? "desc" : "asc"));
    } else {
      setSortColumn(column);
      setSortDirection(column === "schema" ? "asc" : "desc");
    }
  };

  const refreshing = relationStatsStatus?.state === "loading";
  const hasError = relationStatsStatus?.state === "error";

  return (
    <Card>
      <CardHeader>
        <CardTitle>Schemas</CardTitle>
        <CardAction>
          <Button
            size="sm"
            variant="ghost"
            onClick={() => void onLoadRelationStats(activeConnection.id)}
            disabled={refreshing}
          >
            <IconRefresh
              className={cn("size-3.5", refreshing && "animate-spin")}
            />
            {refreshing ? "Refreshing…" : "Refresh"}
          </Button>
        </CardAction>
      </CardHeader>
      <CardContent className="flex flex-col gap-3 text-xs">
        <p className="text-[0.625rem] text-text-muted">
          Click a schema to jump into the Tables sub-tab pre-filtered.
        </p>

        {hasError ? (
          <div className="rounded-md border border-dashed border-danger/40 bg-danger/5 px-3 py-6 text-center text-danger">
            Failed to load schemas — {relationStatsStatus.error}
          </div>
        ) : sorted.length === 0 ? (
          <div className="rounded-md border border-dashed border-border-subtle px-3 py-6 text-center text-text-muted">
            {refreshing
              ? "Loading schemas…"
              : "No user-visible schemas in this connection."}
          </div>
        ) : (
          <div className="overflow-hidden rounded-md border border-border-subtle">
            <table className="w-full border-collapse text-left text-[0.75rem]">
              <thead className="bg-surface-panel text-[0.625rem] uppercase tracking-[0.08em] text-text-muted">
                <tr>
                  <SortHeader
                    column="schema"
                    label="Schema"
                    sortColumn={sortColumn}
                    sortDirection={sortDirection}
                    onSort={handleSort}
                  />
                  <SortHeader
                    column="tables"
                    label="Tables"
                    align="right"
                    sortColumn={sortColumn}
                    sortDirection={sortDirection}
                    onSort={handleSort}
                  />
                  <SortHeader
                    column="views"
                    label="Views"
                    align="right"
                    sortColumn={sortColumn}
                    sortDirection={sortDirection}
                    onSort={handleSort}
                  />
                  <SortHeader
                    column="matviews"
                    label="Matviews"
                    align="right"
                    sortColumn={sortColumn}
                    sortDirection={sortDirection}
                    onSort={handleSort}
                  />
                  <th className="px-3 py-1.5 text-right font-medium">
                    Rows (≈)
                  </th>
                  <SortHeader
                    column="size"
                    label="Size"
                    align="right"
                    sortColumn={sortColumn}
                    sortDirection={sortDirection}
                    onSort={handleSort}
                  />
                  <th className="w-10 px-3 py-1.5 text-right font-medium">
                    Map
                  </th>
                </tr>
              </thead>
              <tbody>
                {sorted.map((row) => (
                  <tr
                    key={row.schema}
                    onClick={() => onSelectSchema(row.schema)}
                    className="cursor-pointer border-t border-border-subtle transition-colors hover:bg-surface-row-hover"
                  >
                    <td className="px-3 py-1.5 font-mono text-foreground">
                      {row.schema}
                    </td>
                    <td className="px-3 py-1.5 text-right tabular-nums text-text-muted">
                      {row.tableCount.toLocaleString()}
                    </td>
                    <td className="px-3 py-1.5 text-right tabular-nums text-text-muted">
                      {row.viewCount.toLocaleString()}
                    </td>
                    <td className="px-3 py-1.5 text-right tabular-nums text-text-muted">
                      {row.matviewCount.toLocaleString()}
                    </td>
                    <td className="px-3 py-1.5 text-right tabular-nums text-text-muted">
                      {formatRowCount(row.rowCountEstimate, {
                        state: "success",
                      })}
                    </td>
                    <td className="px-3 py-1.5 text-right tabular-nums text-text-muted">
                      {formatBytes(row.totalSizeBytes)}
                    </td>
                    <td className="px-2 py-1 text-right">
                      <Button
                        type="button"
                        size="icon-xs"
                        variant="ghost"
                        aria-label={`View ${row.schema} schema map`}
                        title="View schema map"
                        onClick={(event) => {
                          event.stopPropagation();
                          onViewSchemaMap(row.schema);
                        }}
                      >
                        <IconMap className="size-3" />
                      </Button>
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        )}
      </CardContent>
    </Card>
  );
}

function SortHeader({
  column,
  label,
  align,
  sortColumn,
  sortDirection,
  onSort,
}: {
  column: SortColumn;
  label: string;
  align?: "left" | "right";
  sortColumn: SortColumn;
  sortDirection: SortDirection;
  onSort: (column: SortColumn) => void;
}) {
  const isActive = sortColumn === column;
  return (
    <th
      className={cn(
        "px-3 py-1.5 font-medium",
        align === "right" ? "text-right" : "text-left",
      )}
    >
      <button
        type="button"
        onClick={() => onSort(column)}
        className="inline-flex items-center gap-1 hover:text-foreground"
      >
        {label}
        {isActive ? (
          sortDirection === "asc" ? (
            <IconArrowUp className="size-3" />
          ) : (
            <IconArrowDown className="size-3" />
          )
        ) : (
          <IconArrowsSort className="size-3 opacity-40" />
        )}
      </button>
    </th>
  );
}
