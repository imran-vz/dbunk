import {
  IconChevronDown,
  IconChevronRight,
  IconDatabase,
  IconDots,
  IconFilter,
  IconSettings,
  IconTable,
} from "@tabler/icons-react";
import { useMemo, useState } from "react";
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
import { StatusDot, type StatusTone } from "@/components/ui/status-dot";
import { type Connection, useAppStore } from "@/lib/store";
import { cn } from "@/lib/utils";

function statusTone(status: Connection["status"]): StatusTone {
  if (status === "Connected") return "healthy";
  if (status === "Read only") return "warning";
  return "neutral";
}

export function Sidebar({ className }: { className?: string }) {
  const [editingConnection, setEditingConnection] = useState<Connection | null>(
    null,
  );
  const [deletingConnection, setDeletingConnection] =
    useState<Connection | null>(null);
  const [tableFilter, setTableFilter] = useState("");

  const {
    activeConnectionId,
    expandedSchemas,
    connections,
    schemaExplorer,
    setActiveView,
    setActiveConnectionId,
    connectConnection,
    toggleSchema,
    openTableTab,
    openViewTab,
  } = useAppStore();

  const activeConnection = connections.find(
    (connection) => connection.id === activeConnectionId,
  );
  const activeTone = statusTone(activeConnection?.status ?? "Disconnected");

  const explorerSchemas = schemaExplorer[activeConnectionId] ?? [];
  const filteredSchemas = useMemo(() => {
    const needle = tableFilter.trim().toLowerCase();
    if (!needle) return explorerSchemas;
    return explorerSchemas
      .map((schema) => {
        const tables = schema.tables.filter((t) =>
          t.toLowerCase().includes(needle),
        );
        const views =
          schema.views?.filter((v) => v.toLowerCase().includes(needle)) ?? [];
        return { ...schema, tables, views };
      })
      .filter((schema) => schema.tables.length > 0 || schema.views.length > 0);
  }, [explorerSchemas, tableFilter]);

  return (
    <aside
      className={cn(
        "flex h-full w-full flex-col border-r border-border-subtle bg-surface-sidebar text-foreground",
        className,
      )}
    >
      {/* CONNECTIONS */}
      <div className="flex items-center justify-between px-4 pt-4 pb-2">
        <div className="dbunk-section-title">Connections</div>
        <Button
          size="icon-sm"
          variant="ghost"
          aria-label="Manage connections"
          onClick={() => setActiveView("connections")}
          className="size-6"
        >
          <IconSettings className="size-3.5" />
        </Button>
      </div>

      <div className="flex max-h-[34%] shrink-0 flex-col gap-0.5 overflow-auto px-2 pb-3">
        {connections.map((connection) => {
          const isActive = connection.id === activeConnectionId;
          const tone = statusTone(connection.status);
          return (
            <div
              key={connection.id}
              className={cn(
                "group relative flex items-center gap-2.5 rounded-md px-2.5 py-2 text-left text-xs transition-colors",
                isActive
                  ? "bg-accent-green/10 text-foreground before:absolute before:left-0 before:top-2 before:bottom-2 before:w-0.5 before:rounded-full before:bg-accent-green"
                  : "hover:bg-surface-panel",
              )}
            >
              <button
                type="button"
                onClick={() => setActiveConnectionId(connection.id)}
                onDoubleClick={() => connectConnection(connection.id)}
                className="flex min-w-0 flex-1 items-center gap-2.5 text-left"
              >
                <span
                  className={cn(
                    "flex size-7 shrink-0 items-center justify-center rounded-md border",
                    isActive
                      ? "border-accent-green/40 bg-accent-green/15 text-accent-green"
                      : "border-border-subtle bg-surface-panel text-text-muted",
                  )}
                >
                  <IconDatabase className="size-3.5" />
                </span>
                <span className="min-w-0 flex-1">
                  <span className="flex items-center gap-1.5">
                    <span className="truncate text-[0.8125rem] font-medium leading-tight text-foreground">
                      {connection.name}
                    </span>
                  </span>
                  <span className="mt-0.5 block truncate text-[0.6875rem] text-text-muted">
                    {connection.engine}{" "}
                    {connection.role ? `· ${connection.role}` : ""}
                  </span>
                </span>
              </button>
              <StatusDot tone={tone} />
              <DropdownMenu>
                <DropdownMenuTrigger
                  aria-label={`More actions for ${connection.name}`}
                  className="invisible flex size-6 items-center justify-center rounded-md text-text-muted hover:bg-surface-panel-elevated hover:text-foreground group-hover:visible aria-expanded:visible aria-expanded:bg-surface-panel-elevated"
                >
                  <IconDots className="size-3.5" />
                </DropdownMenuTrigger>
                <DropdownMenuContent align="end" className="w-44">
                  <DropdownMenuItem
                    onClick={() => connectConnection(connection.id)}
                  >
                    Connect
                  </DropdownMenuItem>
                  <DropdownMenuItem
                    onClick={() => setEditingConnection(connection)}
                  >
                    Edit…
                  </DropdownMenuItem>
                  <DropdownMenuItem
                    onClick={() => setDeletingConnection(connection)}
                    className="text-danger"
                  >
                    Delete…
                  </DropdownMenuItem>
                </DropdownMenuContent>
              </DropdownMenu>
            </div>
          );
        })}
      </div>

      {/* TABLES */}
      <div className="flex items-center justify-between px-4 pt-3 pb-2">
        <div className="dbunk-section-title">Tables</div>
        <Button
          size="icon-sm"
          variant="ghost"
          aria-label="Filter tables"
          className="size-6"
        >
          <IconFilter className="size-3.5" />
        </Button>
      </div>

      <div className="px-3 pb-2">
        <Input
          value={tableFilter}
          onChange={(event) => setTableFilter(event.target.value)}
          className="h-8 text-xs"
          placeholder="Filter tables"
          aria-label="Filter tables"
        />
      </div>

      <div className="flex min-h-0 flex-1 flex-col gap-0.5 overflow-auto px-2 pb-3 text-xs">
        {filteredSchemas.length === 0 ? (
          <div className="px-2 py-4 text-center text-[0.6875rem] text-text-muted">
            {explorerSchemas.length === 0
              ? "Connect to load schemas"
              : "No tables match"}
          </div>
        ) : null}
        {filteredSchemas.map((schema) => {
          const schemaId = `${activeConnectionId}:${schema.name}`;
          const isExpanded = expandedSchemas.includes(schemaId);
          const totalCount = schema.tables.length + (schema.views?.length ?? 0);
          const visibleTables = isExpanded ? schema.tables : [];
          return (
            <div key={schemaId} className="px-1">
              <button
                type="button"
                onClick={() => toggleSchema(schema.name)}
                className="flex w-full items-center gap-1.5 rounded-md px-1.5 py-1 text-left transition-colors hover:bg-surface-panel"
              >
                {isExpanded ? (
                  <IconChevronDown className="size-3.5 text-text-muted" />
                ) : (
                  <IconChevronRight className="size-3.5 text-text-muted" />
                )}
                <span className="flex-1 truncate text-[0.8125rem] font-medium text-foreground">
                  {schema.name}
                </span>
                <span className="rounded-md bg-surface-panel px-1.5 text-[0.625rem] tabular-nums text-text-muted">
                  {totalCount}
                </span>
              </button>
              {isExpanded ? (
                <div className="mt-0.5 space-y-0.5">
                  {visibleTables.map((table) => (
                    <button
                      key={table}
                      type="button"
                      onClick={() => openTableTab(schema.name, table)}
                      className="flex h-7 w-full items-center gap-2 rounded-md pl-7 pr-2 text-left text-[0.8125rem] text-text-secondary transition-colors hover:bg-surface-panel hover:text-foreground"
                    >
                      <IconTable className="size-3.5 shrink-0 text-text-muted" />
                      <span className="truncate">{table}</span>
                    </button>
                  ))}
                  {schema.views?.map((view) => (
                    <button
                      key={view}
                      type="button"
                      onClick={() => openViewTab(schema.name, view)}
                      className="flex h-7 w-full items-center gap-2 rounded-md pl-7 pr-2 text-left text-[0.8125rem] text-text-secondary transition-colors hover:bg-surface-panel hover:text-foreground"
                    >
                      <IconTable className="size-3.5 shrink-0 text-info/80" />
                      <span className="truncate">{view}</span>
                    </button>
                  ))}
                </div>
              ) : null}
            </div>
          );
        })}
      </div>

      {/* Footer */}
      <div className="flex items-center justify-between border-t border-border-subtle bg-surface-window/60 px-4 py-3 text-xs">
        <div className="flex min-w-0 items-center gap-2">
          <StatusDot tone={activeTone} />
          <div className="min-w-0">
            <div className="truncate text-[0.8125rem] font-medium text-foreground">
              {activeConnection?.name ?? "No connection"}
            </div>
            <div className="text-[0.6875rem] text-text-muted">
              {activeConnection?.status ?? "Disconnected"}
            </div>
          </div>
        </div>
        <Button
          size="icon-sm"
          variant="ghost"
          aria-label="Connection settings"
          onClick={() => setActiveView("settings")}
          className="size-7"
        >
          <IconSettings className="size-3.5" />
        </Button>
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
    </aside>
  );
}
