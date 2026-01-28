import {
  IconChevronDown,
  IconChevronRight,
  IconDatabase,
  IconSettings,
  IconTable,
  IconTerminal2,
} from "@tabler/icons-react";
import { useState } from "react";
import { DeleteConnectionDialog } from "@/components/delete-connection-dialog";
import { EditConnectionDialog } from "@/components/edit-connection-dialog";
import { NewConnectionDialog } from "@/components/new-connection-dialog";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Separator } from "@/components/ui/separator";
import { type Connection, useAppStore } from "@/lib/store";
import { cn } from "@/lib/utils";

type IconType = React.ComponentType<{ className?: string }>;

type NavItem = {
  id: string;
  label: string;
  icon: IconType;
};

const primaryNav: NavItem[] = [
  { id: "workspace", label: "Workspace", icon: IconTerminal2 },
  { id: "connections", label: "Connections", icon: IconDatabase },
];

export function Sidebar() {
  const [newConnectionOpen, setNewConnectionOpen] = useState(false);
  const [editingConnection, setEditingConnection] = useState<Connection | null>(
    null,
  );
  const [deletingConnection, setDeletingConnection] =
    useState<Connection | null>(null);

  const {
    activeView,
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

  const explorerSchemas = schemaExplorer[activeConnectionId] ?? [];
  const totalEntities = explorerSchemas.reduce(
    (count, schema) =>
      count + schema.tables.length + (schema.views?.length ?? 0),
    0,
  );

  return (
    <aside className="flex h-full w-65 flex-col border-r border-sidebar-border bg-sidebar text-sidebar-foreground">
      <div className="flex items-center justify-between px-4 py-3">
        <div className="flex items-center gap-2">
          <div className="flex size-8 items-center justify-center rounded-md bg-sidebar-primary text-sidebar-primary-foreground">
            <IconDatabase className="size-4" />
          </div>
          <div>
            <div className="text-sm font-semibold">Dbunk</div>
            <div className="text-xs text-muted-foreground">Gateway Console</div>
          </div>
        </div>
        <Button size="icon-sm" variant="ghost" aria-label="Open settings">
          <IconSettings />
        </Button>
      </div>

      <div className="px-4 pb-3">
        <Input placeholder="Search tables, schemas, queries" />
      </div>

      <div className="px-3 pb-3">
        <div className="flex flex-col gap-1">
          {primaryNav.map((item) => {
            const isActive = activeView === item.id;
            const NavIcon = item.icon;
            return (
              <Button
                key={item.id}
                size="sm"
                variant={isActive ? "secondary" : "ghost"}
                className="justify-start"
                onClick={() =>
                  setActiveView(item.id as "workspace" | "connections")
                }
              >
                <NavIcon />
                {item.label}
              </Button>
            );
          })}
        </div>
      </div>

      <Separator />

      <div className="flex items-center justify-between px-4 py-3">
        <div className="text-xs font-semibold uppercase text-muted-foreground">
          Connections
        </div>
        <NewConnectionDialog
          open={newConnectionOpen}
          onOpenChange={setNewConnectionOpen}
        />
      </div>

      <div className="flex flex-1 flex-col gap-2 overflow-auto px-3 pb-4">
        {connections.map((connection) => {
          const isActive = connection.id === activeConnectionId;
          return (
            <div
              key={connection.id}
              className={cn(
                "group relative flex flex-col gap-1 rounded-md border px-3 py-2 text-left text-xs transition",
                isActive
                  ? "border-sidebar-accent bg-sidebar-accent/10"
                  : "border-transparent hover:border-sidebar-border hover:bg-sidebar-accent/5",
              )}
            >
              <button
                type="button"
                onClick={() => setActiveConnectionId(connection.id)}
                onDoubleClick={() => connectConnection(connection.id)}
                className="flex flex-col gap-1 text-left"
              >
                <div className="flex items-center justify-between">
                  <div className="text-sm font-medium text-sidebar-foreground">
                    {connection.name}
                  </div>
                  <Badge
                    variant={
                      connection.status === "Disconnected"
                        ? "secondary"
                        : "default"
                    }
                    className="text-[0.625rem]"
                  >
                    {connection.status}
                  </Badge>
                </div>
                <div className="text-muted-foreground">
                  {connection.database}
                </div>
              </button>
            </div>
          );
        })}
      </div>

      <Separator />

      <div className="flex items-center justify-between px-4 py-3">
        <div className="text-xs font-semibold uppercase text-muted-foreground">
          Explorer
        </div>
        <Badge variant="secondary" className="text-[0.625rem]">
          {totalEntities} objects
        </Badge>
      </div>

      <div className="flex flex-col gap-2 overflow-auto px-3 pb-4 text-xs">
        {explorerSchemas.map((schema) => {
          const schemaId = `${activeConnectionId}:${schema.name}`;
          const isExpanded = expandedSchemas.includes(schemaId);
          return (
            <div key={schemaId} className="rounded-md border px-2 py-2">
              <button
                type="button"
                onClick={() => toggleSchema(schema.name)}
                className="flex w-full items-center justify-between gap-2 text-left"
              >
                <div className="flex items-center gap-2 font-medium">
                  {isExpanded ? (
                    <IconChevronDown className="size-3.5" />
                  ) : (
                    <IconChevronRight className="size-3.5" />
                  )}
                  {schema.name}
                </div>
                <span className="text-muted-foreground">
                  {schema.tables.length + (schema.views?.length ?? 0)}
                </span>
              </button>
              {isExpanded && (
                <div className="mt-2 space-y-1">
                  {schema.tables.map((table) => (
                    <button
                      key={table}
                      type="button"
                      onClick={() => openTableTab(schema.name, table)}
                      className="flex w-full items-center gap-2 rounded-sm pl-5 text-left transition hover:bg-muted/40"
                    >
                      <IconTable className="size-3.5 text-muted-foreground" />
                      <span className="truncate">{table}</span>
                    </button>
                  ))}
                  {schema.views?.map((view) => (
                    <button
                      key={view}
                      type="button"
                      onClick={() => openViewTab(schema.name, view)}
                      className="flex w-full items-center gap-2 rounded-sm pl-5 text-left transition hover:bg-muted/40"
                    >
                      <IconTerminal2 className="size-3.5 text-muted-foreground" />
                      <span className="truncate">{view}</span>
                    </button>
                  ))}
                </div>
              )}
            </div>
          );
        })}
      </div>

      <Separator />

      <div className="px-4 py-3 text-xs text-muted-foreground">
        Connected as {activeConnection?.user ?? "admin"}@
        {activeConnection?.database ?? "primary"}
      </div>

      {/* Edit Connection Dialog */}
      <EditConnectionDialog
        connection={editingConnection}
        open={editingConnection !== null}
        onOpenChange={(open) => {
          if (!open) setEditingConnection(null);
        }}
      />

      {/* Delete Connection Dialog */}
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
