import { IconArrowRight, IconStar } from "@tabler/icons-react";

import { Button } from "@/components/ui/button";
import {
  Card,
  CardAction,
  CardContent,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";

export function FavoriteTablesCard({
  tables,
  onOpenTable,
  onViewAll,
}: {
  tables: Array<{ schema: string; name: string; rowCount: number | null }>;
  onOpenTable: (schema: string, table: string) => void;
  onViewAll: () => void;
}) {
  return (
    <Card>
      <CardHeader>
        <CardTitle>Favorite Tables</CardTitle>
        <CardAction>
          <Button size="sm" variant="ghost">
            Manage
          </Button>
        </CardAction>
      </CardHeader>
      <CardContent className="flex flex-col gap-1.5 text-xs">
        {tables.length === 0 ? (
          <div className="rounded-md border border-dashed border-border-subtle px-3 py-6 text-center text-text-muted">
            No favorites yet — star a table to pin it here.
          </div>
        ) : (
          tables.map((table) => (
            <button
              key={`${table.schema}.${table.name}`}
              type="button"
              onClick={() => onOpenTable(table.schema, table.name)}
              className="flex items-center gap-3 rounded-md border border-border-subtle bg-surface-panel-elevated px-3 py-2 text-left transition-colors hover:bg-surface-row-hover"
            >
              <IconStar className="size-3.5 text-warning" />
              <span className="min-w-0 flex-1 truncate font-mono text-[0.75rem] text-foreground">
                {table.name}
              </span>
              <span className="text-[0.625rem] text-text-muted">
                {table.schema}
              </span>
              <span
                className="text-[0.625rem] tabular-nums text-text-muted"
                title={
                  table.rowCount === null
                    ? "Row counts load from pg_stat — Postgres only"
                    : "Planner estimate from pg_class.reltuples"
                }
              >
                {table.rowCount === null
                  ? "—"
                  : `≈${table.rowCount.toLocaleString()}`}
              </span>
            </button>
          ))
        )}
      </CardContent>
      <div className="px-4 pt-1">
        <Button
          size="sm"
          variant="ghost"
          className="px-1 text-text-muted"
          onClick={onViewAll}
        >
          Browse all tables
          <IconArrowRight className="size-3" />
        </Button>
      </div>
    </Card>
  );
}
