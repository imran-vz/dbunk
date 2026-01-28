import {
  IconPencil,
  IconPlus,
  IconSearch,
  IconTrash,
} from "@tabler/icons-react";
import { useMemo, useState } from "react";
import { DeleteConnectionDialog } from "@/components/delete-connection-dialog";
import { EditConnectionDialog } from "@/components/edit-connection-dialog";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { type Connection, useAppStore } from "@/lib/store";
import { cn } from "@/lib/utils";

export function ConnectionsView() {
  const [editingConnection, setEditingConnection] = useState<Connection | null>(
    null,
  );
  const [deletingConnection, setDeletingConnection] =
    useState<Connection | null>(null);

  const {
    activeConnectionId,
    connections,
    schemaExplorer,
    setActiveConnectionId,
    connectConnection,
  } = useAppStore();

  const activeConnection = useMemo(
    () =>
      connections.find((connection) => connection.id === activeConnectionId) ??
      connections[0],
    [activeConnectionId, connections],
  );

  const explorerSchemas = schemaExplorer[activeConnectionId] ?? [];
  const totalEntities = explorerSchemas.reduce(
    (count, schema) =>
      count + schema.tables.length + (schema.views?.length ?? 0),
    0,
  );

  return (
    <>
      <header className="flex items-center justify-between border-b px-4 py-3">
        <div>
          <div className="text-sm font-semibold">Connections</div>
          <div className="text-xs text-muted-foreground">
            {activeConnection
              ? `${activeConnection.name} / ${activeConnection.database}`
              : "No active connection"}
          </div>
        </div>
        <div className="flex items-center gap-2">
          <Button size="sm" variant="outline">
            <IconSearch />
            Test connection
          </Button>
          <Button size="sm" variant="secondary">
            <IconPlus />
            Add connection
          </Button>
        </div>
      </header>

      <div className="flex min-h-0 flex-1">
        <div className="grid h-full min-h-0 flex-1 grid-cols-[1fr_320px]">
          <section className="flex min-h-0 flex-col border-r">
            <div className="border-b px-4 py-3 text-xs text-muted-foreground">
              Manage credentials, test connectivity, and browse schema metadata.
            </div>
            <div className="grid gap-3 overflow-auto p-4">
              {connections.map((connection) => (
                <Card
                  key={connection.id}
                  className={cn(
                    "border border-border transition",
                    connection.id === activeConnectionId
                      ? "ring-2 ring-sidebar-ring/30"
                      : "hover:border-sidebar-border",
                  )}
                >
                  <CardHeader className="flex flex-row items-center justify-between">
                    <div>
                      <CardTitle>{connection.name}</CardTitle>
                      <div className="text-xs text-muted-foreground">
                        {connection.engine} · {connection.database}
                      </div>
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
                  </CardHeader>
                  <CardContent className="grid gap-3 text-xs text-muted-foreground">
                    <div className="grid grid-cols-2 gap-2">
                      <div>
                        <div className="text-[0.625rem] uppercase">Host</div>
                        <div className="text-foreground">
                          {connection.host}:{connection.port}
                        </div>
                      </div>
                      <div>
                        <div className="text-[0.625rem] uppercase">Role</div>
                        <div className="text-foreground">{connection.role}</div>
                      </div>
                    </div>
                    <div className="flex items-center justify-between">
                      <div>
                        <div className="text-[0.625rem] uppercase">Latency</div>
                        <div className="text-foreground">
                          {connection.latency}
                        </div>
                      </div>
                      <div className="text-right">
                        <div className="text-[0.625rem] uppercase">
                          Last sync
                        </div>
                        <div className="text-foreground">
                          {connection.lastSync}
                        </div>
                      </div>
                    </div>
                    <div className="flex items-center gap-2">
                      <Button size="sm" variant="outline">
                        <IconSearch />
                        Test
                      </Button>
                      <Button
                        size="sm"
                        variant={
                          connection.status === "Disconnected"
                            ? "secondary"
                            : "ghost"
                        }
                        onClick={() => {
                          setActiveConnectionId(connection.id);
                          if (connection.status === "Disconnected") {
                            connectConnection(connection.id);
                          }
                        }}
                      >
                        {connection.status === "Disconnected"
                          ? "Connect"
                          : "Open"}
                      </Button>
                      <div className="ml-auto flex items-center gap-1">
                        <Button
                          size="icon-sm"
                          variant="ghost"
                          aria-label={`Edit ${connection.name}`}
                          onClick={() => setEditingConnection(connection)}
                        >
                          <IconPencil className="size-3.5" />
                        </Button>
                        <Button
                          size="icon-sm"
                          variant="ghost"
                          aria-label={`Delete ${connection.name}`}
                          onClick={() => setDeletingConnection(connection)}
                        >
                          <IconTrash className="size-3.5" />
                        </Button>
                      </div>
                    </div>
                  </CardContent>
                </Card>
              ))}
            </div>
          </section>

          <aside className="flex min-h-0 flex-col gap-3 overflow-auto bg-muted/20 p-4">
            <Card size="sm" className="border border-border">
              <CardHeader>
                <CardTitle>Active connection</CardTitle>
              </CardHeader>
              <CardContent className="space-y-2 text-xs text-muted-foreground">
                <div className="flex items-center justify-between">
                  <span>Status</span>
                  <Badge variant="secondary" className="text-[0.625rem]">
                    {activeConnection?.status ?? "Unknown"}
                  </Badge>
                </div>
                <div className="flex items-center justify-between">
                  <span>User</span>
                  <span className="text-foreground">
                    {activeConnection?.user ?? "--"}
                  </span>
                </div>
                <div className="flex items-center justify-between">
                  <span>Last sync</span>
                  <span className="text-foreground">
                    {activeConnection?.lastSync ?? "--"}
                  </span>
                </div>
              </CardContent>
            </Card>

            <Card size="sm" className="border border-border">
              <CardHeader>
                <CardTitle>Metadata snapshot</CardTitle>
              </CardHeader>
              <CardContent className="space-y-2 text-xs text-muted-foreground">
                <div className="flex items-center justify-between">
                  <span>Schemas</span>
                  <span className="text-foreground">
                    {explorerSchemas.length}
                  </span>
                </div>
                <div className="flex items-center justify-between">
                  <span>Entities</span>
                  <span className="text-foreground">{totalEntities}</span>
                </div>
                <div className="flex items-center justify-between">
                  <span>Last refreshed</span>
                  <span className="text-foreground">5 min ago</span>
                </div>
              </CardContent>
            </Card>

            <Card size="sm" className="border border-border">
              <CardHeader>
                <CardTitle>Access controls</CardTitle>
              </CardHeader>
              <CardContent className="space-y-2 text-xs text-muted-foreground">
                <div className="flex items-center justify-between">
                  <span>Privileges</span>
                  <span className="text-foreground">Schema admin</span>
                </div>
                <div className="flex items-center justify-between">
                  <span>Audit</span>
                  <span className="text-foreground">Enabled</span>
                </div>
                <div className="flex items-center justify-between">
                  <span>Secrets</span>
                  <span className="text-foreground">Vault managed</span>
                </div>
              </CardContent>
            </Card>
          </aside>
        </div>
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
    </>
  );
}
