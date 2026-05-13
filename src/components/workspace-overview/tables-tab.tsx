import {
  IconArrowDown,
  IconArrowsSort,
  IconArrowUp,
  IconRefresh,
  IconSearch,
  IconX,
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
import { Input } from "@/components/ui/input";
import type {
  Connection,
  RelationInfo,
  RelationStatsStatus,
  SchemaExplorer,
} from "@/lib/store";
import { cn } from "@/lib/utils";

import { formatBytes, formatRowCount } from "./format";

type SortColumn = "schema" | "name" | "kind" | "rows" | "size";
type SortDirection = "asc" | "desc";

type TableRow = {
  schema: string;
  name: string;
  kind: string;
  rowCountEstimate: number;
  totalSizeBytes: number;
};

export function TablesTab({
  activeConnection,
  schemas,
  relationStats,
  relationStatsStatus,
  schemaFilter,
  onClearSchemaFilter,
  onLoadRelationStats,
  onOpenTable,
}: {
  activeConnection: Connection;
  schemas: SchemaExplorer[];
  relationStats: RelationInfo[] | undefined;
  relationStatsStatus: RelationStatsStatus | undefined;
  schemaFilter: string | null;
  onClearSchemaFilter: () => void;
  onLoadRelationStats: (connectionId: string) => Promise<void>;
  onOpenTable: (schema: string, name: string) => void;
}) {
  const [searchText, setSearchText] = useState("");
  const [sortColumn, setSortColumn] = useState<SortColumn>("schema");
  const [sortDirection, setSortDirection] = useState<SortDirection>("asc");

  const isPostgres = activeConnection.engine === "PostgreSQL";

  // Lazy fetch on first activation; reruns when the cache is missing
  // for the active connection. The store action is idempotent (it
  // sets loading status before issuing the call).
  useEffect(() => {
    if (!isPostgres) {
      return;
    }
    const status = relationStatsStatus?.state;
    if (status === "loading" || status === "success") {
      return;
    }
    void onLoadRelationStats(activeConnection.id);
  }, [
    activeConnection.id,
    isPostgres,
    onLoadRelationStats,
    relationStatsStatus?.state,
  ]);

  const rows = useMemo<TableRow[]>(() => {
    if (isPostgres && relationStats && relationStats.length > 0) {
      return relationStats.map((r) => ({
        schema: r.schema,
        name: r.name,
        kind: r.kind,
        rowCountEstimate: r.rowCountEstimate,
        totalSizeBytes: r.totalSizeBytes,
      }));
    }
    // Fallback: derive from schemaExplorer (kind/rows/size unknown).
    return schemas.flatMap((schema) => {
      const tables = schema.tables.map<TableRow>((name) => ({
        schema: schema.name,
        name,
        kind: "table",
        rowCountEstimate: 0,
        totalSizeBytes: 0,
      }));
      const views = (schema.views ?? []).map<TableRow>((name) => ({
        schema: schema.name,
        name,
        kind: "view",
        rowCountEstimate: 0,
        totalSizeBytes: 0,
      }));
      const materializedViews = (schema.materializedViews ?? []).map<TableRow>(
        (name) => ({
          schema: schema.name,
          name,
          kind: "materialized view",
          rowCountEstimate: 0,
          totalSizeBytes: 0,
        }),
      );
      const foreignTables = (schema.foreignTables ?? []).map<TableRow>(
        (name) => ({
          schema: schema.name,
          name,
          kind: "foreign table",
          rowCountEstimate: 0,
          totalSizeBytes: 0,
        }),
      );
      return [...tables, ...views, ...materializedViews, ...foreignTables];
    });
  }, [isPostgres, relationStats, schemas]);

  const filtered = useMemo(() => {
    const needle = searchText.trim().toLowerCase();
    return rows.filter((row) => {
      if (schemaFilter && row.schema !== schemaFilter) {
        return false;
      }
      if (!needle) {
        return true;
      }
      return (
        row.name.toLowerCase().includes(needle) ||
        row.schema.toLowerCase().includes(needle)
      );
    });
  }, [rows, searchText, schemaFilter]);

  const sorted = useMemo(() => {
    const list = [...filtered];
    list.sort((a, b) => {
      const factor = sortDirection === "asc" ? 1 : -1;
      if (sortColumn === "schema") {
        return (
          a.schema.localeCompare(b.schema) * factor ||
          a.name.localeCompare(b.name)
        );
      }
      if (sortColumn === "name") {
        return a.name.localeCompare(b.name) * factor;
      }
      if (sortColumn === "kind") {
        return a.kind.localeCompare(b.kind) * factor;
      }
      if (sortColumn === "rows") {
        return (a.rowCountEstimate - b.rowCountEstimate) * factor;
      }
      return (a.totalSizeBytes - b.totalSizeBytes) * factor;
    });
    return list;
  }, [filtered, sortColumn, sortDirection]);

  const handleSort = (column: SortColumn) => {
    if (column === sortColumn) {
      setSortDirection((dir) => (dir === "asc" ? "desc" : "asc"));
    } else {
      setSortColumn(column);
      setSortDirection("asc");
    }
  };

  const refreshing = relationStatsStatus?.state === "loading";

  return (
    <Card>
      <CardHeader>
        <CardTitle>Tables</CardTitle>
        <CardAction>
          <Button
            size="sm"
            variant="ghost"
            onClick={() => {
              if (isPostgres) {
                void onLoadRelationStats(activeConnection.id);
              }
            }}
            disabled={!isPostgres || refreshing}
          >
            <IconRefresh
              className={cn("size-3.5", refreshing && "animate-spin")}
            />
            {refreshing ? "Refreshing…" : "Refresh"}
          </Button>
        </CardAction>
      </CardHeader>
      <CardContent className="flex flex-col gap-3 text-xs">
        <div className="flex flex-wrap items-center gap-2">
          <div className="relative min-w-0 flex-1">
            <IconSearch className="pointer-events-none absolute left-2 top-1/2 size-3.5 -translate-y-1/2 text-text-muted" />
            <Input
              value={searchText}
              onChange={(event) => setSearchText(event.currentTarget.value)}
              placeholder="Search tables…"
              className="pl-7"
            />
          </div>
          {schemaFilter ? (
            <button
              type="button"
              onClick={onClearSchemaFilter}
              className="flex items-center gap-1 rounded-md border border-accent-green/40 bg-accent-green/10 px-2.5 py-1 text-[0.6875rem] font-medium text-accent-green-hover transition-colors hover:bg-accent-green/15"
            >
              Schema: {schemaFilter}
              <IconX className="size-3" />
            </button>
          ) : null}
        </div>

        <div className="text-[0.625rem] text-text-muted">
          Showing {sorted.length} of {rows.length} relations
          {schemaFilter ? ` in schema "${schemaFilter}"` : ""}.
        </div>

        {sorted.length === 0 ? (
          <EmptyState
            hasAny={rows.length > 0}
            relationStatsStatus={relationStatsStatus}
            isPostgres={isPostgres}
          />
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
                    column="name"
                    label="Name"
                    sortColumn={sortColumn}
                    sortDirection={sortDirection}
                    onSort={handleSort}
                  />
                  <SortHeader
                    column="kind"
                    label="Kind"
                    sortColumn={sortColumn}
                    sortDirection={sortDirection}
                    onSort={handleSort}
                  />
                  {isPostgres ? (
                    <>
                      <SortHeader
                        column="rows"
                        label="Rows (≈)"
                        align="right"
                        sortColumn={sortColumn}
                        sortDirection={sortDirection}
                        onSort={handleSort}
                      />
                      <SortHeader
                        column="size"
                        label="Size"
                        align="right"
                        sortColumn={sortColumn}
                        sortDirection={sortDirection}
                        onSort={handleSort}
                      />
                    </>
                  ) : null}
                </tr>
              </thead>
              <tbody>
                {sorted.map((row) => (
                  <tr
                    key={`${row.schema}.${row.name}`}
                    onClick={() => onOpenTable(row.schema, row.name)}
                    className="cursor-pointer border-t border-border-subtle transition-colors hover:bg-surface-row-hover"
                  >
                    <td className="px-3 py-1.5 text-text-muted">
                      {row.schema}
                    </td>
                    <td className="px-3 py-1.5 font-mono text-foreground">
                      {row.name}
                    </td>
                    <td className="px-3 py-1.5">
                      <KindBadge kind={row.kind} />
                    </td>
                    {isPostgres ? (
                      <>
                        <td className="px-3 py-1.5 text-right tabular-nums text-text-muted">
                          {row.kind === "view"
                            ? "—"
                            : formatRowCount(row.rowCountEstimate, {
                                state: "success",
                              })}
                        </td>
                        <td className="px-3 py-1.5 text-right tabular-nums text-text-muted">
                          {row.kind === "view"
                            ? "—"
                            : formatBytes(row.totalSizeBytes)}
                        </td>
                      </>
                    ) : null}
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

function KindBadge({ kind }: { kind: string }) {
  const label =
    kind === "materialized view"
      ? "matview"
      : kind === "view"
        ? "view"
        : "table";
  return (
    <span
      className={cn(
        "rounded-full px-2 py-0.5 text-[0.625rem] font-medium",
        label === "table" && "bg-accent-green/10 text-accent-green-hover",
        label === "view" && "bg-accent-blue/10 text-accent-blue-hover",
        label === "matview" && "bg-warning/10 text-warning",
      )}
    >
      {label}
    </span>
  );
}

function EmptyState({
  hasAny,
  relationStatsStatus,
  isPostgres,
}: {
  hasAny: boolean;
  relationStatsStatus: RelationStatsStatus | undefined;
  isPostgres: boolean;
}) {
  if (isPostgres && relationStatsStatus?.state === "error") {
    return (
      <div className="rounded-md border border-dashed border-danger/40 bg-danger/5 px-3 py-6 text-center text-danger">
        Failed to load relations — {relationStatsStatus.error}
      </div>
    );
  }
  if (isPostgres && relationStatsStatus?.state === "loading") {
    return (
      <div className="rounded-md border border-dashed border-border-subtle px-3 py-6 text-center text-text-muted">
        Loading relations…
      </div>
    );
  }
  return (
    <div className="rounded-md border border-dashed border-border-subtle px-3 py-6 text-center text-text-muted">
      {hasAny
        ? "No relations match the current filters."
        : "No relations in this connection yet."}
    </div>
  );
}
