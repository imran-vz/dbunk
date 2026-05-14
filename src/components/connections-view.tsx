import {
  IconClockHour3,
  IconDatabase,
  IconPlus,
  IconSearch,
  IconX,
} from "@tabler/icons-react";
import { useMemo, useState } from "react";
import {
  ConnectionActionsDropdown,
  ConnectionErrorAlert,
} from "@/components/connection-actions";
import { ConnectionForm } from "@/components/connection-form";
import { connectionStatusTone } from "@/components/connection-status";
import { DeleteConnectionDialog } from "@/components/delete-connection-dialog";
import { EditConnectionDialog } from "@/components/edit-connection-dialog";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { HealthPill, type StatusTone } from "@/components/ui/status-dot";
import { type Connection, useAppStore } from "@/lib/store";
import { useContainerWidth } from "@/lib/use-resizable-width";
import { cn } from "@/lib/utils";

// Container-width breakpoints. We measure the panel, not the viewport,
// because the connections view sits inside the Settings tab rail and
// the form aside can take its own 22rem slice.
const STACK_BELOW_PX = 720;
const LIST_BELOW_PX = 640;
const CARDS_TWO_COL_PX = 640;
const CARDS_THREE_COL_PX = 960;
const FORM_PANEL_WIDTH_PX = 352;

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
  return connectionStatusTone(status);
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
  const [search, setSearch] = useState("");
  const [showPanel, setShowPanel] = useState(true);
  const [rootRef, rootWidth] = useContainerWidth<HTMLDivElement>();
  const isNarrow = rootWidth > 0 && rootWidth < STACK_BELOW_PX;
  // Cards-area width = whatever's left after the form panel takes its
  // share (only when the panel is side-by-side; when stacked, cards
  // get the full width). Drives the list-vs-grid decision and the
  // card-grid column bucket.
  const cardsAreaWidth =
    showPanel && !isNarrow ? rootWidth - FORM_PANEL_WIDTH_PX : rootWidth;
  const useListView = cardsAreaWidth > 0 && cardsAreaWidth < LIST_BELOW_PX;
  const cardGridCols =
    cardsAreaWidth >= CARDS_THREE_COL_PX
      ? "grid-cols-3"
      : cardsAreaWidth >= CARDS_TWO_COL_PX
        ? "grid-cols-2"
        : "grid-cols-1";

  const {
    activeConnectionId,
    connections,
    setActiveConnectionId,
    connectConnection,
    disconnectConnection,
  } = useAppStore();

  const filteredConnections = useMemo(() => {
    const needle = search.trim().toLowerCase();
    return connections.filter((c) => matchesSearch(c, needle));
  }, [connections, search]);

  return (
    <div ref={rootRef} className="flex h-full min-h-0 flex-1 flex-col">
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
          "min-h-0 flex-1",
          !showPanel && "flex flex-col",
          showPanel && isNarrow && "flex flex-col overflow-auto",
          showPanel && !isNarrow && "grid grid-cols-[minmax(0,1fr)_22rem]",
        )}
      >
        <section className="flex min-h-0 flex-col">
          {/* Search */}
          <div className="border-b border-border-subtle bg-surface-window px-6 py-3">
            <div className="relative">
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

          {/* Card grid — owns its own scroll when side-by-side; the
              outer wraps and scrolls when stacked under a narrow form
              panel. */}
          <div
            className={cn(
              "p-6",
              isNarrow ? "shrink-0" : "min-h-0 flex-1 overflow-auto",
            )}
          >
            {useListView ? (
              <div className="flex flex-col gap-2">
                {filteredConnections.map((connection) => (
                  <ConnectionListRow
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
                  className="flex items-center gap-3 rounded-lg border border-dashed border-border-strong bg-surface-panel/40 px-4 py-3 text-xs text-text-muted transition-colors hover:border-accent-green/50 hover:bg-surface-panel hover:text-foreground"
                >
                  <span className="flex size-8 items-center justify-center rounded-md border border-border-subtle bg-surface-panel">
                    <IconPlus className="size-3.5" />
                  </span>
                  <span className="flex flex-col items-start">
                    <span className="font-medium">New Connection</span>
                    <span className="text-[0.6875rem]">
                      Add a new database connection
                    </span>
                  </span>
                </button>
              </div>
            ) : (
              <div className={cn("grid gap-4", cardGridCols)}>
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
            )}
          </div>
        </section>

        {showPanel ? (
          <aside
            className={cn(
              "flex min-h-0 flex-col bg-surface-window",
              isNarrow
                ? "border-t border-border-subtle"
                : "border-l border-border-subtle",
            )}
          >
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
        <ConnectionActionsDropdown
          connection={connection}
          onConnect={onConnect}
          onDisconnect={onDisconnect}
          onEdit={onEdit}
          onDelete={onDelete}
        />
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
      </div>

      {connection.errorMessage ? (
        <ConnectionErrorAlert message={connection.errorMessage} />
      ) : null}
    </div>
  );
}

function ConnectionListRow({
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
  const pillTone = derivePillTone(connection.status, connection.errorMessage);
  const pillLabel = deriveStatusLabel(
    connection.status,
    connection.errorMessage,
  );

  return (
    <div
      className={cn(
        "group flex flex-col gap-1.5 rounded-lg border bg-surface-panel px-3 py-2.5 transition-colors",
        isActive
          ? "border-accent-green/40 bg-accent-green/5"
          : "border-border-subtle hover:border-border-strong",
      )}
    >
      <div className="flex items-center gap-2">
        <button
          type="button"
          onClick={onSelect}
          className="flex min-w-0 flex-1 items-center gap-2.5 text-left"
        >
          <span
            className={cn(
              "flex size-7 shrink-0 items-center justify-center rounded-md",
              isActive
                ? "bg-accent-green/15 text-accent-green"
                : "bg-surface-panel-elevated text-text-secondary",
            )}
          >
            <IconDatabase className="size-3.5" />
          </span>
          <span className="min-w-0 flex-1">
            <span className="block truncate text-sm font-semibold text-foreground">
              {connection.name}
            </span>
            <span className="mt-0.5 block truncate font-mono text-[0.6875rem] text-text-muted">
              {connection.engine} · {connection.host || "localhost"}:
              {connection.port || "—"} / {connection.database || "—"}
            </span>
          </span>
        </button>
        <HealthPill tone={pillTone} label={pillLabel} />
        <ConnectionActionsDropdown
          connection={connection}
          onConnect={onConnect}
          onDisconnect={onDisconnect}
          onEdit={onEdit}
          onDelete={onDelete}
        />
      </div>
      <div className="flex items-center justify-between gap-2 pl-9 text-[0.625rem] text-text-muted">
        <span className="flex items-center gap-1.5">
          <IconClockHour3 className="size-3" />
          Last activity {formatLastActivity(connection.lastActivityAt)}
        </span>
      </div>
      {connection.errorMessage ? (
        <ConnectionErrorAlert
          message={connection.errorMessage}
          className="ml-9"
        />
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
