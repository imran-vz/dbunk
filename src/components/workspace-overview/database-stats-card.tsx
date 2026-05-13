import { IconArrowRight, IconRefresh } from "@tabler/icons-react";
import type { ReactNode } from "react";

import { Button } from "@/components/ui/button";
import {
  Card,
  CardAction,
  CardContent,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";
import type { DatabaseOverviewStatsStatus } from "@/lib/store";
import { cn } from "@/lib/utils";

type DatabaseStatsCardProps = {
  tableCount: number;
  schemaCount: number;
  databaseSize: string;
  indexes: string;
  connections: string;
  rows: string;
  statsStatus?: DatabaseOverviewStatsStatus;
  /**
   * "estimate" — PG planner estimate from `pg_class.reltuples`.
   * "exact"    — ClickHouse aggregate from `system.parts.rows`.
   *
   * Drives the "Rows" label so users know what kind of number they're
   * looking at (relevant when the same dashboard is rendered for
   * different engines).
   */
  rowCountKind: "estimate" | "exact";
  onViewAll: () => void;
};

export function DatabaseStatsCard({
  tableCount,
  schemaCount,
  databaseSize,
  indexes,
  connections,
  rows,
  statsStatus,
  rowCountKind,
  onViewAll,
}: DatabaseStatsCardProps) {
  const rowsLabel = rowCountKind === "estimate" ? "Rows (≈)" : "Rows";
  const metrics: Array<[string, ReactNode]> = [
    ["Tables", tableCount.toLocaleString()],
    ["Schemas", schemaCount.toLocaleString()],
    [rowsLabel, rows],
    ["Size", databaseSize],
    ["Indexes", indexes],
    ["Connections", connections],
  ];
  const refreshing = statsStatus?.state === "loading";

  return (
    <Card>
      <CardHeader>
        <CardTitle>Database Stats</CardTitle>
        <CardAction>
          <Button size="sm" variant="ghost" disabled={refreshing}>
            <IconRefresh
              className={cn("size-3.5", refreshing && "animate-spin")}
            />
            {refreshing ? "Refreshing…" : "Updated just now"}
          </Button>
        </CardAction>
      </CardHeader>
      <CardContent className="grid grid-cols-3 gap-x-4 gap-y-4 text-xs">
        {metrics.map(([label, value]) => (
          <div key={label}>
            <div className="text-[0.625rem] font-medium uppercase tracking-[0.12em] text-text-muted">
              {label}
            </div>
            <div className="mt-1 text-lg font-semibold tabular-nums text-foreground">
              {value}
            </div>
          </div>
        ))}
      </CardContent>
      <div className="px-4 pt-1">
        <Button
          size="sm"
          variant="ghost"
          className="px-1 text-text-muted"
          onClick={onViewAll}
        >
          View all metrics
          <IconArrowRight className="size-3" />
        </Button>
      </div>
    </Card>
  );
}
