import {
  IconChevronDown,
  IconChevronRight,
  IconDatabase,
  IconDatabaseOff,
  IconDots,
  IconFilter,
  IconRefresh,
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
import { type Connection, type SchemaExplorer, useAppStore } from "@/lib/store";
import { errorToMessage, isTauri, tauriInvoke } from "@/lib/tauri";
import { cn } from "@/lib/utils";

function statusTone(status: Connection["status"]): StatusTone {
  if (status === "Connected") return "healthy";
  if (status === "Read only") return "warning";
  return "neutral";
}

type SchemaObjectKey = keyof Pick<
  SchemaExplorer,
  | "tables"
  | "views"
  | "materializedViews"
  | "sequences"
  | "foreignTables"
  | "functions"
  | "procedures"
  | "aggregateFunctions"
  | "types"
  | "domains"
  | "extensions"
  | "eventTriggers"
  | "roles"
  | "tablespaces"
>;

const SCHEMA_OBJECT_KEYS: SchemaObjectKey[] = [
  "tables",
  "views",
  "materializedViews",
  "sequences",
  "foreignTables",
  "functions",
  "procedures",
  "aggregateFunctions",
  "types",
  "domains",
  "extensions",
  "eventTriggers",
  "roles",
  "tablespaces",
];

function schemaObjectCount(schema: SchemaExplorer): number {
  return SCHEMA_OBJECT_KEYS.reduce(
    (total, key) => total + (schema[key]?.length ?? 0),
    0,
  );
}

function sqlString(value: string): string {
  return `'${value.replaceAll("'", "''")}'`;
}

function routineName(signature: string): string {
  const parenIndex = signature.indexOf("(");
  return parenIndex >= 0 ? signature.slice(0, parenIndex) : signature;
}

function objectGroups(schema: SchemaExplorer) {
  return [
    {
      label: "Sequences",
      items: schema.sequences ?? [],
      query: (schemaName: string, name: string) =>
        `select * from ${schemaName}.${name};`,
    },
    {
      label: "Foreign tables",
      items: schema.foreignTables ?? [],
      query: (schemaName: string, name: string) =>
        `select * from ${schemaName}.${name} limit 100;`,
    },
    {
      label: "Functions",
      items: schema.functions ?? [],
      query: (schemaName: string, name: string) =>
        `select pg_get_functiondef(p.oid)\nfrom pg_proc p\njoin pg_namespace n on n.oid = p.pronamespace\nwhere n.nspname = ${sqlString(schemaName)} and p.proname = ${sqlString(routineName(name))};`,
    },
    {
      label: "Procedures",
      items: schema.procedures ?? [],
      query: (schemaName: string, name: string) =>
        `select pg_get_functiondef(p.oid)\nfrom pg_proc p\njoin pg_namespace n on n.oid = p.pronamespace\nwhere n.nspname = ${sqlString(schemaName)} and p.proname = ${sqlString(routineName(name))};`,
    },
    {
      label: "Aggregates",
      items: schema.aggregateFunctions ?? [],
      query: (schemaName: string, name: string) =>
        `select pg_get_functiondef(p.oid)\nfrom pg_proc p\njoin pg_namespace n on n.oid = p.pronamespace\nwhere n.nspname = ${sqlString(schemaName)} and p.proname = ${sqlString(routineName(name))};`,
    },
    {
      label: "Types",
      items: schema.types ?? [],
      query: (schemaName: string, name: string) =>
        `select * from pg_type t\njoin pg_namespace n on n.oid = t.typnamespace\nwhere n.nspname = ${sqlString(schemaName)} and t.typname = ${sqlString(name)};`,
    },
    {
      label: "Domains",
      items: schema.domains ?? [],
      query: (schemaName: string, name: string) =>
        `select * from information_schema.domains\nwhere domain_schema = ${sqlString(schemaName)} and domain_name = ${sqlString(name)};`,
    },
    {
      label: "Extensions",
      items: schema.extensions ?? [],
      query: (_schemaName: string, name: string) =>
        `select * from pg_extension where extname = ${sqlString(name)};`,
    },
    {
      label: "Event triggers",
      items: schema.eventTriggers ?? [],
      query: (_schemaName: string, name: string) =>
        `select * from pg_event_trigger where evtname = ${sqlString(name)};`,
    },
    {
      label: "Roles",
      items: schema.roles ?? [],
      query: (_schemaName: string, name: string) =>
        `select * from pg_roles where rolname = ${sqlString(name)};`,
    },
    {
      label: "Tablespaces",
      items: schema.tablespaces ?? [],
      query: (_schemaName: string, name: string) =>
        `select * from pg_tablespace where spcname = ${sqlString(name)};`,
    },
  ];
}

function tableObjectQueries(schema: string, table: string) {
  return [
    {
      label: "Triggers",
      query: `select * from information_schema.triggers\nwhere event_object_schema = ${sqlString(schema)} and event_object_table = ${sqlString(table)}\norder by trigger_name;`,
    },
    {
      label: "Rules",
      query: `select * from pg_rules\nwhere schemaname = ${sqlString(schema)} and tablename = ${sqlString(table)}\norder by rulename;`,
    },
    {
      label: "Policies",
      query: `select * from pg_policies\nwhere schemaname = ${sqlString(schema)} and tablename = ${sqlString(table)}\norder by policyname;`,
    },
    {
      label: "Partitions",
      query: `select child_ns.nspname as schema, child.relname as partition_name\nfrom pg_inherits i\njoin pg_class parent on parent.oid = i.inhparent\njoin pg_namespace parent_ns on parent_ns.oid = parent.relnamespace\njoin pg_class child on child.oid = i.inhrelid\njoin pg_namespace child_ns on child_ns.oid = child.relnamespace\nwhere parent_ns.nspname = ${sqlString(schema)} and parent.relname = ${sqlString(table)}\norder by child_ns.nspname, child.relname;`,
    },
    {
      label: "Dependencies",
      query: `select classid::regclass::text, objid::regclass::text, refclassid::regclass::text, refobjid::regclass::text, deptype\nfrom pg_depend\nwhere refobjid = ${sqlString(`${schema}.${table}`)}::regclass\norder by classid::regclass::text, objid::regclass::text;`,
    },
    {
      label: "References",
      query: `select conname, conrelid::regclass::text as source_table, confrelid::regclass::text as referenced_table, pg_get_constraintdef(oid, true) as definition\nfrom pg_constraint\nwhere contype = 'f' and (conrelid = ${sqlString(`${schema}.${table}`)}::regclass or confrelid = ${sqlString(`${schema}.${table}`)}::regclass)\norder by conname;`,
    },
  ];
}

export function Sidebar({ className }: { className?: string }) {
  const [editingConnection, setEditingConnection] = useState<Connection | null>(
    null,
  );
  const [deletingConnection, setDeletingConnection] =
    useState<Connection | null>(null);
  const [tableFilter, setTableFilter] = useState("");
  const [expandedTables, setExpandedTables] = useState<string[]>([]);

  const {
    activeConnectionId,
    expandedSchemas,
    connections,
    schemaExplorer,
    setActiveView,
    setActiveConnectionId,
    connectConnection,
    disconnectConnection,
    toggleSchema,
    openWorkspaceTab,
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
        const next = { ...schema };
        for (const key of SCHEMA_OBJECT_KEYS) {
          next[key] = (schema[key] ?? []).filter((name) =>
            name.toLowerCase().includes(needle),
          );
        }
        return next;
      })
      .filter((schema) => schemaObjectCount(schema) > 0);
  }, [explorerSchemas, tableFilter]);

  const openObjectQuery = (
    schema: string,
    _name: string,
    label: string,
    query: string,
  ) => {
    openWorkspaceTab({
      kind: "query",
      label,
      connectionId: activeConnectionId,
      schema,
      query,
    });
  };

  const refreshMaterializedView = async (schema: string, view: string) => {
    if (!isTauri() || !activeConnectionId) {
      return;
    }
    try {
      await tauriInvoke("refresh_materialized_view", {
        payload: {
          connectionId: activeConnectionId,
          schema,
          view,
          concurrently: false,
        },
      });
    } catch (error) {
      window.alert(errorToMessage(error));
    }
  };

  const toggleTableNodes = (schema: string, table: string) => {
    const key = `${activeConnectionId}:${schema}.${table}`;
    setExpandedTables((current) =>
      current.includes(key)
        ? current.filter((item) => item !== key)
        : [...current, key],
    );
  };

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
                    disabled={connection.status === "Disconnected"}
                    onClick={() => disconnectConnection(connection.id)}
                  >
                    <IconDatabaseOff className="size-3.5" />
                    Disconnect
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
          const totalCount = schemaObjectCount(schema);
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
                    <div key={table}>
                      <div className="flex h-7 items-center gap-1 rounded-md pl-7 pr-2 text-[0.8125rem] text-text-secondary transition-colors hover:bg-surface-panel hover:text-foreground">
                        <button
                          type="button"
                          aria-label={`Show table objects for ${table}`}
                          onClick={() => toggleTableNodes(schema.name, table)}
                          className="flex size-4 shrink-0 items-center justify-center rounded text-text-muted"
                        >
                          {expandedTables.includes(
                            `${activeConnectionId}:${schema.name}.${table}`,
                          ) ? (
                            <IconChevronDown className="size-3" />
                          ) : (
                            <IconChevronRight className="size-3" />
                          )}
                        </button>
                        <button
                          type="button"
                          onClick={() => openTableTab(schema.name, table)}
                          className="flex min-w-0 flex-1 items-center gap-2 text-left"
                        >
                          <IconTable className="size-3.5 shrink-0 text-text-muted" />
                          <span className="truncate">{table}</span>
                        </button>
                      </div>
                      {expandedTables.includes(
                        `${activeConnectionId}:${schema.name}.${table}`,
                      ) ? (
                        <div className="ml-10 border-l border-border-subtle pl-2">
                          {tableObjectQueries(schema.name, table).map(
                            (node) => (
                              <button
                                key={node.label}
                                type="button"
                                onClick={() =>
                                  openObjectQuery(
                                    schema.name,
                                    table,
                                    `${table}-${node.label.toLowerCase()}.sql`,
                                    node.query,
                                  )
                                }
                                className="flex h-6 w-full items-center text-left text-[0.75rem] text-text-muted hover:text-foreground"
                              >
                                {node.label}
                              </button>
                            ),
                          )}
                        </div>
                      ) : null}
                    </div>
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
                  {schema.materializedViews?.map((view) => (
                    <div
                      key={`matview:${view}`}
                      className="flex h-7 items-center gap-1 rounded-md pl-7 pr-1 text-[0.8125rem] text-text-secondary transition-colors hover:bg-surface-panel hover:text-foreground"
                    >
                      <button
                        type="button"
                        onClick={() => openViewTab(schema.name, view)}
                        className="flex min-w-0 flex-1 items-center gap-2 text-left"
                      >
                        <IconTable className="size-3.5 shrink-0 text-accent-amber" />
                        <span className="truncate">{view}</span>
                      </button>
                      <button
                        type="button"
                        aria-label={`Refresh materialized view ${view}`}
                        title="Refresh materialized view"
                        onClick={() =>
                          void refreshMaterializedView(schema.name, view)
                        }
                        className="flex size-5 shrink-0 items-center justify-center rounded text-text-muted hover:bg-surface-panel-elevated hover:text-foreground"
                      >
                        <IconRefresh className="size-3" />
                      </button>
                    </div>
                  ))}
                  {objectGroups(schema).map((group) =>
                    group.items.length > 0 ? (
                      <div key={group.label} className="pt-1">
                        <div className="pl-7 pr-2 text-[0.625rem] font-medium uppercase text-text-muted">
                          {group.label}
                        </div>
                        {group.items.map((item) => (
                          <button
                            key={`${group.label}:${item}`}
                            type="button"
                            onClick={() =>
                              openObjectQuery(
                                schema.name,
                                item,
                                `${item}.sql`,
                                group.query(schema.name, item),
                              )
                            }
                            className="flex h-7 w-full items-center gap-2 rounded-md pl-7 pr-2 text-left text-[0.8125rem] text-text-secondary transition-colors hover:bg-surface-panel hover:text-foreground"
                          >
                            <IconTable className="size-3.5 shrink-0 text-text-muted" />
                            <span className="truncate">{item}</span>
                          </button>
                        ))}
                      </div>
                    ) : null,
                  )}
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
