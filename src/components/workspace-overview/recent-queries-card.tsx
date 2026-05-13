import { IconArrowRight, IconClock, IconTerminal2 } from "@tabler/icons-react";

import { Button } from "@/components/ui/button";
import {
  Card,
  CardAction,
  CardContent,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";
import type { QueryHistoryEntry } from "@/lib/store";
import { cn } from "@/lib/utils";

export function RecentQueriesCard({
  queries,
  onViewAll,
}: {
  queries: QueryHistoryEntry[];
  onViewAll: () => void;
}) {
  return (
    <Card>
      <CardHeader>
        <CardTitle>Recent Queries</CardTitle>
        <CardAction>
          <Button size="sm" variant="ghost" onClick={onViewAll}>
            View all
          </Button>
        </CardAction>
      </CardHeader>
      <CardContent className="flex flex-col gap-2 text-xs">
        {queries.length === 0 ? (
          <EmptyState />
        ) : (
          queries.map((query) => <QueryRow key={query.id} query={query} />)
        )}
      </CardContent>
      <div className="px-4 pt-1">
        <Button
          size="sm"
          variant="ghost"
          className="px-1 text-text-muted"
          onClick={onViewAll}
        >
          Open Query History
          <IconArrowRight className="size-3" />
        </Button>
      </div>
    </Card>
  );
}

function EmptyState() {
  return (
    <div className="flex flex-col items-center justify-center gap-2 rounded-md border border-dashed border-border-subtle px-3 py-6 text-text-muted">
      <IconTerminal2 className="size-5 opacity-60" />
      <span>No queries yet — open the editor and run one.</span>
    </div>
  );
}

function QueryRow({ query }: { query: QueryHistoryEntry }) {
  return (
    <div className="flex w-full min-w-0 items-center gap-3 rounded-md border border-border-subtle bg-surface-panel-elevated px-3 py-2">
      <span
        className={cn(
          "flex size-7 shrink-0 items-center justify-center rounded-md",
          query.status === "success"
            ? "bg-accent-green/10 text-accent-green-hover"
            : "bg-danger/10 text-danger",
        )}
      >
        <IconTerminal2 className="size-3.5" />
      </span>
      <div className="flex min-w-0 flex-1 flex-col">
        <div className="truncate font-mono text-[0.75rem] text-foreground">
          {query.sql}
        </div>
        <div className="mt-0.5 flex min-w-0 items-center gap-1.5 text-[0.625rem] text-text-muted">
          <IconClock className="size-2.5 shrink-0" />
          <span className="shrink-0 whitespace-nowrap">{query.startedAt}</span>
          <span className="shrink-0">·</span>
          <span className="min-w-0 flex-1 truncate">
            {query.connectionName}
          </span>
        </div>
      </div>
      <span className="shrink-0 rounded-full bg-accent-green/10 px-2 py-0.5 text-[0.625rem] font-medium tabular-nums text-accent-green-hover">
        {query.runtimeMs} ms
      </span>
    </div>
  );
}
