import {
  IconAlertCircle,
  IconClockHour3,
  IconDatabase,
  IconDatabaseOff,
  IconDotsVertical,
  IconPencil,
  IconPlus,
  IconSearch,
  IconTrash,
  IconX,
} from "@tabler/icons-react";
import { useMemo, useState } from "react";
import { ConnectionForm } from "@/components/connection-form";
import { DeleteConnectionDialog } from "@/components/delete-connection-dialog";
import { EditConnectionDialog } from "@/components/edit-connection-dialog";
import { Button } from "@/components/ui/button";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { Input } from "@/components/ui/input";
import { HealthPill, type StatusTone } from "@/components/ui/status-dot";
import { type Connection, useAppStore } from "@/lib/store";
import { cn } from "@/lib/utils";

type FilterKind = "all" | "healthy" | "warning" | "error";

const FILTER_TONE: Record<FilterKind, StatusTone | "neutral"> = {
  all: "neutral",
  healthy: "healthy",
  warning: "warning",
  error: "danger",
};

function statusToTone(status: Connection["status"]): StatusTone {
  if (status === "Connected") return "healthy";
  if (status === "Read only") return "warning";
  return "neutral";
}

function classifyForFilter(status: Connection["status"]): FilterKind {
  if (status === "Connected") return "healthy";
  if (status === "Read only") return "warning";
  // Treat Disconnected as "neutral" but countable under both All only.
  return "all";
}

function matchesFilter(connection: Connection, filter: FilterKind): boolean {
  if (filter === "healthy") return connection.status === "Connected";
  if (filter === "warning") return connection.status === "Read only";
  if (filter === "error") return Boolean(connection.errorMessage);
  return true;
}

function matchesSearch(connection: Connection, needle: string): boolean {
  if (!needle) return true;
  return (
    connection.name.toLowerCase().includes(needle) ||
    connection.host.toLowerCase().includes(needle) ||
    connection.database.toLowerCase().includes(needle) ||
    connection.engine.toLowerCase().includes(needle)
  );
}

function derivePillTone(
  status: Connection["status"],
  errorMessage: string | undefined,
): StatusTone {
  if (errorMessage) return "danger";
  const tone = statusToTone(status);
  return tone === "neutral" ? "neutral" : tone;
}

function deriveStatusLabel(
  status: Connection["status"],
  errorMessage: string | undefined,
): string {
  if (errorMessage) return "Error";
  if (status === "Connected") return "Healthy";
  if (status === "Read only") return "Warning";
  return "Idle";
}

export function ConnectionsView() {
  const [editingConnection, setEditingConnection] = useState<Connection | null>(
    null,
  );
  const [deletingConnection, setDeletingConnection] =
    useState<Connection | null>(null);
  const [filter, setFilter] = useState<FilterKind>("all");
  const [search, setSearch] = useState("");
  const [showPanel, setShowPanel] = useState(true);

  const {
    activeConnectionId,
    connections,
    setActiveConnectionId,
    connectConnection,
    disconnectConnection,
  } = useAppStore();

  const filterCounts = useMemo(() => {
    const counts: Record<FilterKind, number> = {
      all: connections.length,
      healthy: 0,
      warning: 0,
      error: 0,
    };
    connections.forEach((c) => {
      if (c.status === "Connected") counts.healthy += 1;
      else if (c.status === "Read only") counts.warning += 1;
      else if (c.errorMessage) counts.error += 1;
    });
    return counts;
  }, [connections]);

  const filteredConnections = useMemo(() => {
    const needle = search.trim().toLowerCase();
    return connections.filter(
      (c) => matchesFilter(c, filter) && matchesSearch(c, needle),
    );
  }, [connections, filter, search]);

  return (
    <div className="flex h-full min-h-0 flex-1 flex-col">
      <header className="flex shrink-0 items-end justify-between gap-3 border-b border-border-subtle bg-surface-window px-6 py-4">
        <div>
          <h1 className="text-xl font-semibold tracking-tight text-foreground">
            Connections
          </h1>
          <p className="mt-1 text-xs text-text-muted">
            Manage your database connections and credentials.
          </p>
        </div>
        <Button
          type="button"
          size="sm"
          variant={showPanel ? "secondary" : "default"}
          onClick={() => setShowPanel((prev) => !prev)}
        >
          <IconPlus className="size-3.5" />
          {showPanel ? "Hide form" : "New Connection"}
        </Button>
      </header>

      <div
        className={cn(
          "grid min-h-0 flex-1",
          showPanel
            ? "grid-cols-[minmax(0,1fr)_22rem]"
            : "grid-cols-[minmax(0,1fr)]",
        )}
      >
        <section className="flex min-h-0 flex-col">
          {/* Filters + search */}
          <div className="flex flex-wrap items-center gap-3 border-b border-border-subtle bg-surface-window px-6 py-3">
            <div
              role="tablist"
              aria-label="Connection filters"
              className="flex items-center gap-1 rounded-md border border-border-subtle bg-surface-panel p-1"
            >
              {(
                [
                  ["all", "All"],
                  ["healthy", "Healthy"],
                  ["warning", "Warning"],
                  ["error", "Error"],
                ] as const
              ).map(([id, label]) => {
                const isActive = filter === id;
                const count = filterCounts[id];
                return (
                  <button
                    key={id}
                    type="button"
                    role="tab"
                    aria-selected={isActive}
                    onClick={() => setFilter(id)}
                    className={cn(
                      "flex h-7 items-center gap-1.5 rounded-md px-2.5 text-xs font-medium transition-colors",
                      isActive
                        ? "bg-surface-panel-elevated text-foreground"
                        : "text-text-muted hover:bg-surface-panel-elevated/60 hover:text-foreground",
                    )}
                  >
                    {label}
                    <span
                      className={cn(
                        "tabular-nums",
                        isActive ? "text-text-secondary" : "text-text-muted",
                      )}
                    >
                      {count}
                    </span>
                    <span className="sr-only">
                      {FILTER_TONE[id as FilterKind] ?? "all"}
                    </span>
                  </button>
                );
              })}
            </div>
            <div className="relative ml-auto min-w-64 max-w-md flex-1">
              <IconSearch className="pointer-events-none absolute left-2.5 top-1/2 size-3.5 -translate-y-1/2 text-text-muted" />
              <Input
                aria-label="Search connections"
                placeholder="Search connections"
                value={search}
                onChange={(event) => setSearch(event.target.value)}
                className="pl-8"
              />
            </div>
          </div>

          {/* Card grid */}
          <div className="min-h-0 flex-1 overflow-auto p-6">
            <div className="grid gap-4 sm:grid-cols-2 xl:grid-cols-3">
              {filteredConnections.map((connection) => (
                <ConnectionCard
                  key={connection.id}
                  connection={connection}
                  isActive={connection.id === activeConnectionId}
                  onSelect={() => setActiveConnectionId(connection.id)}
                  onConnect={() => {
                    setActiveConnectionId(connection.id);
                    void connectConnection(connection.id);
                  }}
                  onDisconnect={() => disconnectConnection(connection.id)}
                  onEdit={() => setEditingConnection(connection)}
                  onDelete={() => setDeletingConnection(connection)}
                />
              ))}
              <button
                type="button"
                onClick={() => setShowPanel(true)}
                className="flex min-h-32 flex-col items-center justify-center gap-2 rounded-lg border border-dashed border-border-strong bg-surface-panel/40 text-xs text-text-muted transition-colors hover:border-accent-green/50 hover:bg-surface-panel hover:text-foreground"
              >
                <span className="flex size-9 items-center justify-center rounded-md border border-border-subtle bg-surface-panel">
                  <IconPlus className="size-4" />
                </span>
                <span className="font-medium">New Connection</span>
                <span className="text-[0.6875rem]">
                  Add a new database connection
                </span>
              </button>
            </div>
          </div>
        </section>

        {showPanel ? (
          <aside className="flex min-h-0 flex-col border-l border-border-subtle bg-surface-window">
            <div className="flex items-center justify-between border-b border-border-subtle px-4 py-3">
              <div>
                <div className="text-sm font-semibold text-foreground">
                  New Connection
                </div>
                <div className="mt-0.5 text-[0.6875rem] text-text-muted">
                  Connect to a Postgres, MySQL, ClickHouse, or SQLite database.
                </div>
              </div>
              <Button
                type="button"
                size="icon-sm"
                variant="ghost"
                aria-label="Close new-connection panel"
                onClick={() => setShowPanel(false)}
                className="size-7"
              >
                <IconX className="size-3.5" />
              </Button>
            </div>
            <div className="flex min-h-0 flex-1 flex-col">
              <ConnectionForm mode="new" onSaved={() => setShowPanel(false)} />
            </div>
          </aside>
        ) : null}
      </div>

      <EditConnectionDialog
        connection={editingConnection}
        open={editingConnection !== null}
        onOpenChange={(open) => {
          if (!open) setEditingConnection(null);
        }}
      />
      <DeleteConnectionDialog
        connection={deletingConnection}
        open={deletingConnection !== null}
        onOpenChange={(open) => {
          if (!open) setDeletingConnection(null);
        }}
      />
    </div>
  );
}

function ConnectionCard({
  connection,
  isActive,
  onSelect,
  onConnect,
  onDisconnect,
  onEdit,
  onDelete,
}: {
  connection: Connection;
  isActive: boolean;
  onSelect: () => void;
  onConnect: () => void;
  onDisconnect: () => void;
  onEdit: () => void;
  onDelete: () => void;
}) {
  const filterKind = classifyForFilter(connection.status);
  const pillTone = derivePillTone(connection.status, connection.errorMessage);
  const pillLabel = deriveStatusLabel(
    connection.status,
    connection.errorMessage,
  );

  return (
    <div
      className={cn(
        "group flex min-h-32 flex-col gap-3 rounded-lg border bg-surface-panel p-4 transition-colors",
        isActive
          ? "border-accent-green/40 bg-accent-green/5"
          : "border-border-subtle hover:border-border-strong",
      )}
    >
      <div className="flex items-start justify-between gap-2">
        <button
          type="button"
          onClick={onSelect}
          className="flex min-w-0 flex-1 items-start gap-3 text-left"
        >
          <span
            className={cn(
              "flex size-9 shrink-0 items-center justify-center rounded-md",
              isActive
                ? "bg-accent-green/15 text-accent-green"
                : "bg-surface-panel-elevated text-text-secondary",
            )}
          >
            <IconDatabase className="size-4" />
          </span>
          <span className="min-w-0 flex-1">
            <span className="flex items-center gap-2">
              <span className="truncate text-sm font-semibold text-foreground">
                {connection.name}
              </span>
              <HealthPill tone={pillTone} label={pillLabel} />
            </span>
            <span className="mt-0.5 block truncate text-[0.6875rem] text-text-muted">
              {connection.engine}
            </span>
          </span>
        </button>
        <DropdownMenu>
          <DropdownMenuTrigger
            aria-label="Connection actions"
            className="rounded-md p-1 text-text-muted hover:bg-surface-panel-elevated hover:text-foreground"
          >
            <IconDotsVertical className="size-3.5" />
          </DropdownMenuTrigger>
          <DropdownMenuContent align="end">
            <DropdownMenuItem onClick={onConnect}>Connect</DropdownMenuItem>
            <DropdownMenuItem
              disabled={connection.status === "Disconnected"}
              onClick={onDisconnect}
            >
              <IconDatabaseOff className="size-3.5" />
              Disconnect
            </DropdownMenuItem>
            <DropdownMenuItem onClick={onEdit}>
              <IconPencil className="size-3.5" />
              Edit…
            </DropdownMenuItem>
            <DropdownMenuItem onClick={onDelete} className="text-danger">
              <IconTrash className="size-3.5" />
              Delete…
            </DropdownMenuItem>
          </DropdownMenuContent>
        </DropdownMenu>
      </div>

      <div className="font-mono text-[0.6875rem] text-text-muted">
        {connection.host || "localhost"}:{connection.port || "—"} /{" "}
        {connection.database || "—"}
      </div>

      <div className="mt-auto flex items-center justify-between gap-2 text-[0.625rem] text-text-muted">
        <span className="flex items-center gap-1.5">
          <IconClockHour3 className="size-3" />
          Last activity {formatLastActivity(connection.lastActivityAt)}
        </span>
        <span className="sr-only">{filterKind}</span>
      </div>

      {connection.errorMessage ? (
        <div
          role="alert"
          className="flex items-start gap-2 rounded-md border border-danger/30 bg-danger/10 px-2 py-1.5 text-xs text-danger"
        >
          <IconAlertCircle className="mt-0.5 size-3.5 shrink-0" />
          <div className="flex-1 wrap-break-word">
            {connection.errorMessage}
          </div>
        </div>
      ) : null}
    </div>
  );
}

function formatLastActivity(value: string | undefined): string {
  if (!value) return "—";
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) return "—";
  const diffMs = Date.now() - date.getTime();
  if (diffMs < 60_000) return "just now";
  if (diffMs < 3_600_000) {
    const minutes = Math.round(diffMs / 60_000);
    return `${minutes}m ago`;
  }
  if (diffMs < 86_400_000) {
    const hours = Math.round(diffMs / 3_600_000);
    return `${hours}h ago`;
  }
  if (diffMs < 30 * 86_400_000) {
    const days = Math.round(diffMs / 86_400_000);
    return `${days}d ago`;
  }
  return date.toLocaleDateString();
}
