import {
  IconCheck,
  IconClock,
  IconCopy,
  IconExternalLink,
  IconSearch,
  IconTerminal2,
} from "@tabler/icons-react";
import { useMemo, useState } from "react";

import { Button } from "@/components/ui/button";
import {
  Card,
  CardAction,
  CardContent,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import type { Connection, QueryHistoryEntry } from "@/lib/store";
import { cn } from "@/lib/utils";

type StatusFilter = "all" | "success" | "error";

export function QueryHistoryTab({
  activeConnection,
  queryHistory,
  onReopenEntry,
}: {
  activeConnection: Connection;
  queryHistory: QueryHistoryEntry[];
  onReopenEntry: (entry: QueryHistoryEntry) => void;
}) {
  const [showAll, setShowAll] = useState(false);
  const [searchText, setSearchText] = useState("");
  const [statusFilter, setStatusFilter] = useState<StatusFilter>("all");
  const [copiedId, setCopiedId] = useState<string | null>(null);

  const filtered = useMemo(() => {
    const needle = searchText.trim().toLowerCase();
    return queryHistory.filter((entry) => {
      if (!showAll && entry.connectionId !== activeConnection.id) {
        return false;
      }
      if (statusFilter !== "all" && entry.status !== statusFilter) {
        return false;
      }
      if (needle && !entry.sql.toLowerCase().includes(needle)) {
        return false;
      }
      return true;
    });
  }, [queryHistory, showAll, statusFilter, searchText, activeConnection.id]);

  const handleCopy = async (entry: QueryHistoryEntry) => {
    // oxlint-disable-next-line anti-slop/no-runtime-typeof -- The value is handled at a typed library or domain boundary here.
    if (typeof navigator === "undefined" || !navigator.clipboard) {
      return;
    }
    try {
      await navigator.clipboard.writeText(entry.sql);
      setCopiedId(entry.id);
      window.setTimeout(() => {
        setCopiedId((current) => (current === entry.id ? null : current));
      }, 1200);
    } catch (error) {
      console.error("Failed to copy SQL to clipboard", error);
    }
  };

  const totalForScope = showAll
    ? queryHistory.length
    : queryHistory.filter((e) => e.connectionId === activeConnection.id).length;

  return (
    <Card>
      <CardHeader>
        <CardTitle>Query history</CardTitle>
        <CardAction>
          <Button
            size="sm"
            variant={showAll ? "default" : "outline"}
            onClick={() => setShowAll((prev) => !prev)}
          >
            {showAll ? "Showing all connections" : "Showing this connection"}
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
              placeholder="Search SQL…"
              className="pl-7"
            />
          </div>
          <StatusFilterChip
            value="all"
            label="All"
            current={statusFilter}
            onSelect={setStatusFilter}
          />
          <StatusFilterChip
            value="success"
            label="Success"
            current={statusFilter}
            onSelect={setStatusFilter}
          />
          <StatusFilterChip
            value="error"
            label="Errors"
            current={statusFilter}
            onSelect={setStatusFilter}
          />
        </div>

        <div className="text-2xs text-text-muted">
          Showing {filtered.length} of {totalForScope}
          {showAll ? " entries across all connections" : " entries"}.
        </div>

        {filtered.length === 0 ? (
          <EmptyState hasAny={queryHistory.length > 0} />
        ) : (
          <ul className="flex flex-col gap-1.5">
            {filtered.map((entry) => (
              <li key={entry.id}>
                <HistoryRow
                  entry={entry}
                  isCopied={copiedId === entry.id}
                  onReopen={() => onReopenEntry(entry)}
                  onCopy={() => void handleCopy(entry)}
                />
              </li>
            ))}
          </ul>
        )}
      </CardContent>
    </Card>
  );
}

function StatusFilterChip({
  value,
  label,
  current,
  onSelect,
}: {
  value: StatusFilter;
  label: string;
  current: StatusFilter;
  onSelect: (value: StatusFilter) => void;
}) {
  const isActive = value === current;
  return (
    <button
      type="button"
      onClick={() => onSelect(value)}
      className={cn(
        "rounded-md border px-2.5 py-1 text-2xs font-medium transition-colors",
        isActive
          ? "border-accent/40 bg-accent/10 text-accent-hover"
          : "border-border-subtle text-text-muted hover:bg-surface-panel hover:text-foreground",
      )}
    >
      {label}
    </button>
  );
}

function EmptyState({ hasAny }: { hasAny: boolean }) {
  return (
    <div className="flex flex-col items-center justify-center gap-2 rounded-md border border-dashed border-border-subtle px-3 py-8 text-text-muted">
      <IconTerminal2 className="size-5 opacity-60" />
      <span>
        {hasAny
          ? "No queries match the current filters."
          : "No queries yet — open the editor and run one."}
      </span>
    </div>
  );
}

function HistoryRow({
  entry,
  isCopied,
  onReopen,
  onCopy,
}: {
  entry: QueryHistoryEntry;
  isCopied: boolean;
  onReopen: () => void;
  onCopy: () => void;
}) {
  return (
    <div className="flex w-full min-w-0 items-center gap-3 rounded-md border border-border-subtle bg-surface-panel-elevated px-3 py-2">
      <span
        className={cn(
          "flex size-7 shrink-0 items-center justify-center rounded-md",
          entry.status === "success"
            ? "bg-accent/10 text-accent-hover"
            : "bg-danger/10 text-danger",
        )}
      >
        <IconTerminal2 className="size-3.5" />
      </span>
      <div className="flex min-w-0 flex-1 flex-col">
        <div className="truncate font-mono text-xs text-foreground">
          {entry.sql}
        </div>
        <div className="mt-0.5 flex min-w-0 items-center gap-1.5 text-2xs text-text-muted">
          <IconClock className="size-2.5 shrink-0" />
          <span className="shrink-0 whitespace-nowrap">{entry.startedAt}</span>
          <span className="shrink-0">·</span>
          <span className="min-w-0 truncate">{entry.connectionName}</span>
          {entry.errorMessage ? (
            <>
              <span className="shrink-0">·</span>
              <span className="min-w-0 truncate text-danger">
                {entry.errorMessage}
              </span>
            </>
          ) : null}
        </div>
      </div>
      <span className="shrink-0 rounded-full bg-surface-panel px-2 py-0.5 text-2xs font-medium tabular-nums text-text-muted">
        {entry.runtimeMs} ms
      </span>
      <div className="flex shrink-0 items-center gap-1">
        <Button
          size="sm"
          variant="ghost"
          onClick={onCopy}
          title="Copy SQL"
          aria-label="Copy SQL"
        >
          {isCopied ? (
            <IconCheck className="size-3.5 text-accent-hover" />
          ) : (
            <IconCopy className="size-3.5" />
          )}
        </Button>
        <Button
          size="sm"
          variant="ghost"
          onClick={onReopen}
          title="Open in editor"
          aria-label="Open in editor"
        >
          <IconExternalLink className="size-3.5" />
        </Button>
      </div>
    </div>
  );
}
