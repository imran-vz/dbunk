/**
 * Global command palette (Cmd/Ctrl+K). Sources items from the live
 * store — connections, tables across loaded schemas, saved queries,
 * plus a small set of primary actions. Selecting an item routes
 * through the relevant store action (`connectConnection`,
 * `openTableTab`, `openWorkspaceTab` for saved queries, etc.).
 *
 * The palette is intentionally light: no fuzzy ranking magic, just
 * substring match via `cmdk`'s built-in filter. Capped at a few
 * hundred items to keep the dropdown snappy on dense schemas.
 */

import {
  IconBookmark,
  IconDatabase,
  IconPlus,
  IconSettings,
  IconTable,
} from "@tabler/icons-react";
import { Command } from "cmdk";
import { useEffect, useMemo, useState } from "react";

import { useAppStore } from "@/lib/store";
import { cn } from "@/lib/utils";

const MAX_TABLES = 300;
const MAX_SAVED_QUERIES = 100;

/* oxlint-disable jsx-a11y/prefer-tag-over-role -- This custom modal overlay relies on application-managed open and backdrop behavior. */
export function CommandPalette() {
  const [open, setOpen] = useState(false);
  const [query, setQuery] = useState("");
  const connections = useAppStore((state) => state.connections);
  const schemaExplorer = useAppStore((state) => state.schemaExplorer);
  const savedQueries = useAppStore((state) => state.savedQueries);
  const openSettings = useAppStore((state) => state.openSettings);
  const connectConnection = useAppStore((state) => state.connectConnection);
  const openTableTab = useAppStore((state) => state.openTableTab);
  const openWorkspaceTab = useAppStore((state) => state.openWorkspaceTab);
  const createNewQueryTab = useAppStore((state) => state.createNewQueryTab);
  const activeConnectionId = useAppStore((state) => state.activeConnectionId);

  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      const isPaletteKey =
        (event.metaKey || event.ctrlKey) && event.key.toLowerCase() === "k";
      if (isPaletteKey) {
        event.preventDefault();
        setOpen((prev) => !prev);
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);

  useEffect(() => {
    if (!open) setQuery("");
  }, [open]);

  // Listen for an explicit "open command palette" CustomEvent so the
  // header search field can dispatch it without prop-drilling.
  useEffect(() => {
    const onCustom = () => setOpen(true);
    window.addEventListener("dbunk:open-command-palette", onCustom);
    return () =>
      window.removeEventListener("dbunk:open-command-palette", onCustom);
  }, []);

  const tableItems = useMemo(() => {
    const items: Array<{
      key: string;
      label: string;
      connectionId: string;
      connectionName: string;
      schema: string;
      table: string;
    }> = [];
    for (const connection of connections) {
      const schemas = schemaExplorer[connection.id] ?? [];
      for (const schema of schemas) {
        for (const table of schema.tables) {
          items.push({
            key: `${connection.id}::${schema.name}::${table}`,
            label: `${schema.name}.${table}`,
            connectionId: connection.id,
            connectionName: connection.name,
            schema: schema.name,
            table,
          });
          if (items.length >= MAX_TABLES) return items;
        }
      }
    }
    return items;
  }, [connections, schemaExplorer]);

  const savedQueryItems = useMemo(
    () => savedQueries.slice(0, MAX_SAVED_QUERIES),
    [savedQueries],
  );

  const close = () => setOpen(false);

  if (!open) return null;

  return (
    /* oxlint-disable-next-line jsx-a11y/no-noninteractive-element-interactions, jsx-a11y/prefer-tag-over-role -- This custom modal overlay closes only when its backdrop receives the pointer event. */
    <div
      role="dialog"
      aria-label="Command palette"
      aria-modal="true"
      className="fixed inset-0 z-50 flex items-start justify-center bg-black/60 p-4 pt-[15vh]"
      onMouseDown={(event) => {
        if (event.target === event.currentTarget) close();
      }}
    >
      <Command
        label="Command palette"
        className="w-full max-w-xl overflow-hidden rounded-lg border border-border-subtle bg-surface-window shadow-2xl"
      >
        <Command.Input
          value={query}
          onValueChange={setQuery}
          placeholder="Type a command, table, or saved query…"
          className="w-full border-b border-border-subtle bg-transparent px-4 py-3 text-sm outline-none placeholder:text-text-muted"
          autoFocus
        />
        <Command.List className="max-h-[60vh] overflow-auto p-1">
          <Command.Empty className="px-4 py-6 text-center text-xs text-text-muted">
            No matches.
          </Command.Empty>

          <Command.Group
            heading="Actions"
            className="px-1 py-1 text-2xs uppercase tracking-wide text-text-muted"
          >
            <PaletteItem
              icon={<IconPlus className="size-3.5" />}
              label="New query tab"
              onSelect={() => {
                createNewQueryTab();
                close();
              }}
            />
            <PaletteItem
              icon={<IconDatabase className="size-3.5" />}
              label="Open Connections settings"
              onSelect={() => {
                openSettings("connections");
                close();
              }}
            />
            <PaletteItem
              icon={<IconSettings className="size-3.5" />}
              label="Open Settings"
              onSelect={() => {
                openSettings();
                close();
              }}
            />
          </Command.Group>

          {connections.length > 0 ? (
            <Command.Group
              heading="Connections"
              className="mt-1 px-1 py-1 text-2xs uppercase tracking-wide text-text-muted"
            >
              {connections.map((connection) => (
                <PaletteItem
                  key={connection.id}
                  icon={<IconDatabase className="size-3.5" />}
                  label={connection.name}
                  description={`${connection.engine}${
                    connection.id === activeConnectionId ? " · active" : ""
                  }`}
                  onSelect={() => {
                    void connectConnection(connection.id);
                    close();
                  }}
                />
              ))}
            </Command.Group>
          ) : null}

          {tableItems.length > 0 ? (
            <Command.Group
              heading="Tables"
              className="mt-1 px-1 py-1 text-2xs uppercase tracking-wide text-text-muted"
            >
              {tableItems.map((item) => (
                <PaletteItem
                  key={item.key}
                  icon={<IconTable className="size-3.5" />}
                  label={item.label}
                  description={item.connectionName}
                  onSelect={() => {
                    if (item.connectionId !== activeConnectionId) {
                      useAppStore.setState({
                        activeConnectionId: item.connectionId,
                      });
                    }
                    openTableTab(item.schema, item.table);
                    close();
                  }}
                />
              ))}
            </Command.Group>
          ) : null}

          {savedQueryItems.length > 0 ? (
            <Command.Group
              heading="Saved queries"
              className="mt-1 px-1 py-1 text-2xs uppercase tracking-wide text-text-muted"
            >
              {savedQueryItems.map((saved) => (
                <PaletteItem
                  key={saved.id}
                  icon={<IconBookmark className="size-3.5" />}
                  label={saved.name}
                  description={
                    saved.body.length > 60
                      ? `${saved.body.slice(0, 60)}…`
                      : saved.body
                  }
                  onSelect={() => {
                    openWorkspaceTab({
                      kind: "query",
                      label: `${saved.name}.sql`,
                      connectionId:
                        saved.connectionId ?? activeConnectionId ?? "",
                      schema: "public",
                      query: saved.body,
                    });
                    close();
                  }}
                />
              ))}
            </Command.Group>
          ) : null}
        </Command.List>
      </Command>
    </div>
  );
}

/* oxlint-enable jsx-a11y/prefer-tag-over-role */
function PaletteItem({
  icon,
  label,
  description,
  onSelect,
}: {
  icon: React.ReactNode;
  label: string;
  description?: string;
  onSelect: () => void;
}) {
  return (
    <Command.Item
      value={`${label} ${description ?? ""}`.trim()}
      onSelect={onSelect}
      className={cn(
        "flex items-center gap-2 rounded-md px-2 py-1.5 text-xs text-text-secondary",
        "data-[selected=true]:bg-surface-panel-elevated data-[selected=true]:text-foreground",
      )}
    >
      <span className="text-text-muted">{icon}</span>
      <span className="truncate">{label}</span>
      {description ? (
        <span className="ml-auto truncate text-2xs text-text-muted">
          {description}
        </span>
      ) : null}
    </Command.Item>
  );
}
