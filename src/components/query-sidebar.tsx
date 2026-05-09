import { useMemo } from "react";
import { Badge } from "@/components/ui/badge";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import {
  type QueryHistoryEntry,
  type QueryPreviewData,
  useAppStore,
  type WorkspaceTab,
} from "@/lib/store";

interface QuerySidebarProps {
  tab: WorkspaceTab;
}

const truncateSql = (sql: string, max = 80): string => {
  const collapsed = sql.replace(/\s+/g, " ").trim();
  return collapsed.length > max ? `${collapsed.slice(0, max - 1)}…` : collapsed;
};

const formatTimestamp = (iso: string): string => {
  const date = new Date(iso);
  if (Number.isNaN(date.getTime())) {
    return iso;
  }
  return date.toLocaleString(undefined, {
    month: "short",
    day: "numeric",
    hour: "2-digit",
    minute: "2-digit",
  });
};

export function QuerySidebar({ tab }: QuerySidebarProps) {
  const queryPreviews = useAppStore((state) => state.queryPreviews);
  const queryHistory = useAppStore((state) => state.queryHistory);
  const reopenHistoryEntry = useAppStore((state) => state.reopenHistoryEntry);

  const activeQueryPreview: QueryPreviewData | null = useMemo(() => {
    if (tab.kind !== "query") {
      return null;
    }
    return (
      queryPreviews[tab.label] ?? {
        columns: ["column"],
        rows: [],
        runtime: "--",
        rowCount: "0",
        cache: "Cold",
      }
    );
  }, [tab, queryPreviews]);

  const visibleHistory = useMemo(
    () => queryHistory.slice(0, 25),
    [queryHistory],
  );

  return (
    <>
      <Card size="sm" className="border border-border">
        <CardHeader>
          <CardTitle>Query details</CardTitle>
        </CardHeader>
        <CardContent className="space-y-2 text-xs text-muted-foreground">
          <div className="flex items-center justify-between">
            <span>Rows</span>
            <span className="text-foreground">
              {activeQueryPreview?.rowCount ?? "0"}
            </span>
          </div>
          <div className="flex items-center justify-between">
            <span>Runtime</span>
            <span className="text-foreground">
              {activeQueryPreview?.runtime ?? "--"}
            </span>
          </div>
          <div className="flex items-center justify-between">
            <span>Cache</span>
            <Badge variant="secondary" className="text-[0.625rem]">
              {activeQueryPreview?.cache ?? "Cold"}
            </Badge>
          </div>
          <div className="flex items-center justify-between">
            <span>Status</span>
            <span className="text-foreground">
              {tab.isDirty ? "Edited" : "Saved"}
            </span>
          </div>
        </CardContent>
      </Card>

      <Card size="sm" className="border border-border">
        <CardHeader>
          <CardTitle>Recent queries</CardTitle>
        </CardHeader>
        <CardContent className="space-y-2 text-xs">
          {visibleHistory.length > 0 ? (
            visibleHistory.map((entry: QueryHistoryEntry) => (
              <button
                key={entry.id}
                type="button"
                onClick={() => reopenHistoryEntry(entry)}
                className="block w-full rounded-md border px-2 py-1.5 text-left transition-colors hover:bg-muted/50"
                data-testid="query-history-entry"
              >
                <div className="flex items-center justify-between gap-2">
                  <span className="truncate font-mono text-foreground">
                    {truncateSql(entry.sql)}
                  </span>
                  <Badge
                    variant={
                      entry.status === "success" ? "secondary" : "destructive"
                    }
                    className="text-[0.625rem] shrink-0"
                  >
                    {entry.status === "success" ? "OK" : "Error"}
                  </Badge>
                </div>
                <div className="mt-1 flex items-center justify-between gap-2 text-muted-foreground">
                  <span className="truncate">
                    {entry.connectionName || entry.connectionId}
                  </span>
                  <span className="shrink-0">
                    {entry.runtimeMs}ms &middot;{" "}
                    {formatTimestamp(entry.startedAt)}
                  </span>
                </div>
              </button>
            ))
          ) : (
            <div className="text-muted-foreground">No recent queries</div>
          )}
        </CardContent>
      </Card>

      <Card size="sm" className="border border-border">
        <CardHeader>
          <CardTitle>Guardrails</CardTitle>
        </CardHeader>
        <CardContent className="space-y-2 text-xs text-muted-foreground">
          <div className="flex items-center justify-between">
            <span>Timeout</span>
            <span className="text-foreground">30s</span>
          </div>
          <div className="flex items-center justify-between">
            <span>Max rows</span>
            <span className="text-foreground">10,000</span>
          </div>
          <div className="flex items-center justify-between">
            <span>Export</span>
            <span className="text-foreground">Enabled</span>
          </div>
        </CardContent>
      </Card>
    </>
  );
}
