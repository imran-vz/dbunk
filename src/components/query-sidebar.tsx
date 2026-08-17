import {
  IconChevronRight,
  IconCircleFilled,
  IconPlus,
  IconStar,
  IconStarFilled,
  IconTrash,
} from "@tabler/icons-react";
import { useMemo, useState } from "react";

import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import {
  type QueryHistoryEntry,
  type SavedQuery,
  useAppStore,
  type WorkspaceTab,
} from "@/lib/store";
import { cn } from "@/lib/utils";

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

function generateSavedId(): string {
  if (typeof crypto !== "undefined" && "randomUUID" in crypto) {
    return crypto.randomUUID();
  }
  return `saved-${Date.now()}-${Math.random().toString(36).slice(2)}`;
}

export function QuerySidebar({ tab }: QuerySidebarProps) {
  const queryHistory = useAppStore((state) => state.queryHistory);
  const savedQueries = useAppStore((state) => state.savedQueries);
  const reopenHistoryEntry = useAppStore((state) => state.reopenHistoryEntry);
  const updateQuery = useAppStore((state) => state.updateQuery);
  const saveSavedQuery = useAppStore((state) => state.saveSavedQuery);
  const deleteSavedQuery = useAppStore((state) => state.deleteSavedQuery);

  const [newName, setNewName] = useState("");
  const [isCreatingNew, setIsCreatingNew] = useState(false);

  const visibleHistory = useMemo(
    () => queryHistory.slice(0, 8),
    [queryHistory],
  );

  // Favorites first, then most-recently-updated.
  const sortedSaved = useMemo(() => {
    return [...savedQueries].sort((a, b) => {
      if (a.isFavorite !== b.isFavorite) return a.isFavorite ? -1 : 1;
      return b.updatedAt.localeCompare(a.updatedAt);
    });
  }, [savedQueries]);

  const handleSaveCurrent = async () => {
    if (!newName.trim() || !tab.query) return;
    await saveSavedQuery({
      id: generateSavedId(),
      name: newName.trim(),
      body: tab.query,
      connectionId: tab.connectionId || null,
      isFavorite: false,
    });
    setNewName("");
    setIsCreatingNew(false);
  };

  const handleToggleFavorite = async (saved: SavedQuery) => {
    await saveSavedQuery({
      ...saved,
      isFavorite: !saved.isFavorite,
    });
  };

  const handleOpenSaved = (saved: SavedQuery) => {
    updateQuery(tab.id, saved.body);
  };

  return (
    <div className="flex h-full min-h-0 flex-col gap-3 overflow-auto">
      <Card size="sm" className="min-h-0">
        <CardHeader>
          <CardTitle>Query History</CardTitle>
          <Button size="xs" variant="ghost" className="text-text-muted">
            Clear
          </Button>
        </CardHeader>
        <CardContent className="flex flex-col gap-1.5 text-xs">
          {visibleHistory.length > 0 ? (
            visibleHistory.map((entry: QueryHistoryEntry, index) => {
              const isCurrent = index === 0;
              return (
                <button
                  key={entry.id}
                  type="button"
                  onClick={() => reopenHistoryEntry(entry)}
                  className="flex w-full min-w-0 flex-col gap-1 rounded-md border border-border-subtle bg-surface-panel-elevated px-2.5 py-2 text-left transition-colors hover:border-accent/40 hover:bg-surface-row-hover"
                  data-testid="query-history-entry"
                >
                  <div className="flex w-full min-w-0 items-center gap-2">
                    <span className="min-w-0 flex-1 truncate font-mono text-[0.75rem] text-foreground">
                      {truncateSql(entry.sql, 48)}
                    </span>
                    {isCurrent ? (
                      <IconCircleFilled className="size-2 shrink-0 text-accent" />
                    ) : null}
                    <span className="sr-only">
                      {entry.status === "success" ? "OK" : "Error"}
                    </span>
                  </div>
                  <div className="flex w-full min-w-0 items-center gap-2 text-[0.625rem] text-text-muted">
                    <span className="min-w-0 flex-1 truncate">
                      {entry.connectionName || entry.connectionId}
                    </span>
                    <span className="shrink-0 tabular-nums">
                      {entry.runtimeMs}ms · {formatTimestamp(entry.startedAt)}
                    </span>
                  </div>
                </button>
              );
            })
          ) : (
            <div className="rounded-md border border-dashed border-border-subtle px-3 py-4 text-center text-text-muted">
              No recent queries
            </div>
          )}
        </CardContent>
        <div className="px-3 pt-1">
          <Button size="sm" variant="ghost" className="px-1 text-text-muted">
            View all history
            <IconChevronRight className="size-3" />
          </Button>
        </div>
      </Card>

      <Card size="sm">
        <CardHeader>
          <CardTitle>Saved Queries</CardTitle>
          <Button
            size="xs"
            variant="ghost"
            className="text-text-muted"
            onClick={() => setIsCreatingNew((prev) => !prev)}
          >
            {isCreatingNew ? "Cancel" : "Manage"}
          </Button>
        </CardHeader>
        <CardContent className="flex flex-col gap-1.5 text-xs">
          {isCreatingNew ? (
            <div className="flex flex-col gap-1.5 rounded-md border border-accent/30 bg-surface-panel-elevated p-2">
              <div className="text-[0.625rem] font-medium uppercase tracking-[0.12em] text-text-muted">
                Save current query
              </div>
              <Input
                autoFocus
                aria-label="Saved query name"
                placeholder="e.g. Top customers"
                value={newName}
                onChange={(event) => setNewName(event.target.value)}
                onKeyDown={(event) => {
                  if (event.key === "Enter") {
                    void handleSaveCurrent();
                  }
                  if (event.key === "Escape") {
                    setNewName("");
                    setIsCreatingNew(false);
                  }
                }}
                className="h-7 text-xs"
              />
              <div className="flex items-center justify-end gap-1.5">
                <Button
                  size="xs"
                  variant="ghost"
                  onClick={() => {
                    setNewName("");
                    setIsCreatingNew(false);
                  }}
                >
                  Cancel
                </Button>
                <Button
                  size="xs"
                  disabled={!newName.trim() || !tab.query?.trim()}
                  onClick={() => {
                    void handleSaveCurrent();
                  }}
                >
                  Save
                </Button>
              </div>
            </div>
          ) : null}

          {sortedSaved.length === 0 && !isCreatingNew ? (
            <div className="rounded-md border border-dashed border-border-subtle px-3 py-4 text-center text-text-muted">
              No saved queries yet
            </div>
          ) : null}

          {sortedSaved.map((saved) => (
            <div
              key={saved.id}
              className="group flex items-start gap-2 rounded-md border border-border-subtle bg-surface-panel-elevated p-2 transition-colors hover:border-accent/40"
            >
              <button
                type="button"
                aria-label={`Toggle favorite for ${saved.name}`}
                onClick={() => {
                  void handleToggleFavorite(saved);
                }}
                className="mt-0.5"
              >
                {saved.isFavorite ? (
                  <IconStarFilled className="size-3 text-warning" />
                ) : (
                  <IconStar className="size-3 text-text-muted hover:text-warning" />
                )}
              </button>
              <button
                type="button"
                onClick={() => handleOpenSaved(saved)}
                className="flex min-w-0 flex-1 flex-col gap-0.5 text-left"
              >
                <div className="flex items-center justify-between gap-2">
                  <span className="truncate text-[0.75rem] font-medium text-foreground">
                    {saved.name}
                  </span>
                  <Badge variant="secondary" className="h-4 text-[0.5625rem]">
                    SQL
                  </Badge>
                </div>
                <div className="truncate font-mono text-[0.625rem] text-text-muted">
                  {truncateSql(saved.body, 56)}
                </div>
              </button>
              <button
                type="button"
                aria-label={`Delete saved query ${saved.name}`}
                onClick={() => {
                  void deleteSavedQuery(saved.id);
                }}
                className={cn(
                  "shrink-0 rounded-md p-1 text-text-muted opacity-0 hover:bg-surface-row-hover hover:text-danger group-hover:opacity-100",
                )}
              >
                <IconTrash className="size-3" />
              </button>
            </div>
          ))}
        </CardContent>
        <div className="px-3 pt-1">
          <Button
            size="sm"
            variant="ghost"
            className="px-1 text-text-muted"
            onClick={() => setIsCreatingNew(true)}
            disabled={!tab.query?.trim()}
          >
            <IconPlus className="size-3" />
            New Saved Query
          </Button>
        </div>
      </Card>
      <span className="sr-only" aria-hidden="true">
        Editing {tab.label}
      </span>
    </div>
  );
}
