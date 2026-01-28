import { useMemo } from "react";
import { Badge } from "@/components/ui/badge";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import {
  type QueryPreviewData,
  useAppStore,
  type WorkspaceTab,
} from "@/lib/store";

interface QuerySidebarProps {
  tab: WorkspaceTab;
}

export function QuerySidebar({ tab }: QuerySidebarProps) {
  const { queryPreviews, recentQueries } = useAppStore();

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
          {recentQueries.length > 0 ? (
            recentQueries.map((query, index) => (
              <div
                key={`query-${
                  // biome-ignore lint/suspicious/noArrayIndexKey: the query does not have reliable id
                  index
                }`}
                className="rounded-md border px-2 py-1"
              >
                <div className="truncate text-muted-foreground">{query}</div>
              </div>
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
