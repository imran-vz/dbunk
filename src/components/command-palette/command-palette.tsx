/**
 * Command palette (DESIGN-SYSTEM §4.10, P7). `Cmd+K`, one unified
 * surface on the Dialog primitive: a bare query matches everything —
 * commands, connections, tables, saved queries — grouped with
 * recents-first ordering; a `>` prefix restricts to commands.
 *
 * Commands come from the central shortcut registry (§6.1): every
 * binding appears with its kbd hint, and rows invoke the handler the
 * owning surface registered. Selection frecency persists per item
 * through the P8 UI-state store.
 */

import {
  IconBolt,
  IconBookmark,
  IconDatabase,
  IconTable,
} from "@tabler/icons-react";
import { Command } from "cmdk";
import {
  useCallback,
  useEffect,
  useMemo,
  useState,
  useSyncExternalStore,
} from "react";

import { Dialog, DialogContent } from "@/components/ui/dialog";
import { Kbd } from "@/components/ui/kbd";
import {
  dispatchShortcut,
  getShortcutRegistryVersion,
  hasShortcutHandler,
  SHORTCUTS,
  subscribeShortcutRegistry,
} from "@/lib/shortcuts";
import { useAppStore } from "@/lib/store";
import { uiGet, uiSet } from "@/lib/ui-state";
import { cn } from "@/lib/utils";

const MAX_TABLES = 300;
const MAX_SAVED_QUERIES = 100;
const FRECENCY_KEY = "dbunk.palette.frecency";
const RECENT_LIMIT = 6;

type FrecencyMap = Record<string, { count: number; last: number }>;

const readFrecency = (): FrecencyMap => {
  try {
    const raw = uiGet(FRECENCY_KEY);
    if (!raw) return {};
    // SAFETY: scores are advisory ordering hints; malformed entries fall out below.
    const parsed = JSON.parse(raw) as FrecencyMap;
    // oxlint-disable-next-line anti-slop/no-runtime-typeof -- persisted-value validation.
    return parsed && typeof parsed === "object" ? parsed : {};
  } catch {
    return {};
  }
};

const recordUse = (key: string) => {
  try {
    const map = readFrecency();
    const entry = map[key];
    map[key] = { count: (entry?.count ?? 0) + 1, last: Date.now() };
    // Cap the map so it can't grow unbounded.
    const entries = Object.entries(map)
      .sort((a, b) => b[1].last - a[1].last)
      .slice(0, 200);
    uiSet(FRECENCY_KEY, JSON.stringify(Object.fromEntries(entries)));
  } catch {
    // Best-effort ranking only.
  }
};

const frecencyScore = (entry: { count: number; last: number }): number => {
  const ageDays = (Date.now() - entry.last) / 86_400_000;
  return entry.count / (1 + ageDays);
};

export function CommandPalette() {
  const [open, setOpen] = useState(false);
  const [query, setQuery] = useState("");
  const connections = useAppStore((state) => state.connections);
  const schemaExplorer = useAppStore((state) => state.schemaExplorer);
  const savedQueries = useAppStore((state) => state.savedQueries);
  const connectConnection = useAppStore((state) => state.connectConnection);
  const openTableTab = useAppStore((state) => state.openTableTab);
  const openWorkspaceTab = useAppStore((state) => state.openWorkspaceTab);
  const activeConnectionId = useAppStore((state) => state.activeConnectionId);

  // Re-render when surfaces register/deregister command handlers.
  const registryVersion = useSyncExternalStore(
    subscribeShortcutRegistry,
    getShortcutRegistryVersion,
    () => 0,
  );

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

  const close = useCallback(() => setOpen(false), []);

  const runItem = useCallback(
    (key: string, action: () => void) => {
      recordUse(key);
      close();
      action();
    },
    [close],
  );

  // `>` prefix restricts to commands (§4.10).
  const commandsOnly = query.trimStart().startsWith(">");
  const effectiveQuery = commandsOnly
    ? query.trimStart().slice(1).trimStart()
    : query;

  const commandItems = useMemo(() => {
    void registryVersion;
    return SHORTCUTS.filter(
      (shortcut) =>
        shortcut.id !== "command-palette" && hasShortcutHandler(shortcut.id),
    ).map((shortcut) => ({
      key: `command:${shortcut.id}`,
      shortcut,
    }));
  }, [registryVersion]);

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
            key: `table:${connection.id}::${schema.name}::${table}`,
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

  // Recents-first (§4.10): with an empty query, surface the top frecent
  // items across every kind before the grouped lists.
  const recentKeys = useMemo(() => {
    if (!open) return [];
    const map = readFrecency();
    return Object.entries(map)
      .sort((a, b) => frecencyScore(b[1]) - frecencyScore(a[1]))
      .map(([key]) => key)
      .slice(0, RECENT_LIMIT);
  }, [open]);

  const actionsByKey = useMemo(() => {
    const actions = new Map<
      string,
      {
        label: string;
        description?: string;
        icon: React.ReactNode;
        run: () => void;
        keys?: ReadonlyArray<string>;
      }
    >();
    for (const { key, shortcut } of commandItems) {
      actions.set(key, {
        label: shortcut.label,
        description: shortcut.group,
        icon: <IconBolt className="size-3.5" />,
        keys: shortcut.keys,
        run: () => dispatchShortcut(shortcut.id),
      });
    }
    for (const connection of connections) {
      actions.set(`connection:${connection.id}`, {
        label: connection.name,
        description: connection.engine,
        icon: <IconDatabase className="size-3.5" />,
        run: () => void connectConnection(connection.id),
      });
    }
    for (const item of tableItems) {
      actions.set(item.key, {
        label: item.label,
        description: item.connectionName,
        icon: <IconTable className="size-3.5" />,
        run: () => {
          if (item.connectionId !== activeConnectionId) {
            useAppStore.setState({ activeConnectionId: item.connectionId });
          }
          openTableTab(item.schema, item.table);
        },
      });
    }
    for (const saved of savedQueryItems) {
      actions.set(`saved:${saved.id}`, {
        label: saved.name,
        description: "Saved query",
        icon: <IconBookmark className="size-3.5" />,
        run: () =>
          openWorkspaceTab({
            kind: "query",
            label: `${saved.name}.sql`,
            connectionId: saved.connectionId ?? activeConnectionId ?? "",
            schema: "public",
            query: saved.body,
          }),
      });
    }
    return actions;
  }, [
    commandItems,
    connections,
    tableItems,
    savedQueryItems,
    activeConnectionId,
    connectConnection,
    openTableTab,
    openWorkspaceTab,
  ]);

  const recentEntries = recentKeys
    .map((key) => {
      const action = actionsByKey.get(key);
      return action ? { key, action } : null;
    })
    .filter((entry): entry is NonNullable<typeof entry> => entry !== null);

  return (
    <Dialog open={open} onOpenChange={setOpen}>
      <DialogContent
        size="lg"
        aria-label="Command palette"
        className="top-[15vh] max-h-[70vh] translate-y-0 gap-0 p-0"
      >
        <Command label="Command palette" className="flex min-h-0 flex-col">
          <Command.Input
            value={query}
            onValueChange={setQuery}
            placeholder="Search commands, tables, connections… (> for commands)"
            className="w-full border-b border-border-subtle bg-transparent px-4 py-3 text-sm outline-none placeholder:text-text-muted"
            // oxlint-disable-next-line jsx-a11y/no-autofocus -- palette opens focused by definition.
            autoFocus
          />
          <Command.List className="max-h-[55vh] min-h-0 overflow-auto p-1">
            <Command.Empty className="px-4 py-6 text-center text-xs text-text-muted">
              No matches.
            </Command.Empty>

            {!commandsOnly &&
            effectiveQuery === "" &&
            recentEntries.length > 0 ? (
              <Command.Group
                heading="Recent"
                className="px-1 py-1 text-2xs uppercase tracking-wide text-text-muted"
              >
                {recentEntries.map(({ key, action }) => (
                  <PaletteRow
                    key={`recent-${key}`}
                    icon={action.icon}
                    label={action.label}
                    description={action.description}
                    keys={action.keys}
                    onSelect={() => runItem(key, action.run)}
                  />
                ))}
              </Command.Group>
            ) : null}

            <Command.Group
              heading="Commands"
              className="px-1 py-1 text-2xs uppercase tracking-wide text-text-muted"
            >
              {commandItems.map(({ key, shortcut }) => (
                <PaletteRow
                  key={key}
                  icon={<IconBolt className="size-3.5" />}
                  label={shortcut.label}
                  description={shortcut.group}
                  keys={shortcut.keys}
                  onSelect={() =>
                    runItem(key, () => dispatchShortcut(shortcut.id))
                  }
                />
              ))}
            </Command.Group>

            {!commandsOnly && connections.length > 0 ? (
              <Command.Group
                heading="Connections"
                className="mt-1 px-1 py-1 text-2xs uppercase tracking-wide text-text-muted"
              >
                {connections.map((connection) => (
                  <PaletteRow
                    key={connection.id}
                    icon={<IconDatabase className="size-3.5" />}
                    label={connection.name}
                    description={`${connection.engine}${
                      connection.id === activeConnectionId ? " · active" : ""
                    }`}
                    onSelect={() =>
                      runItem(
                        `connection:${connection.id}`,
                        () => void connectConnection(connection.id),
                      )
                    }
                  />
                ))}
              </Command.Group>
            ) : null}

            {!commandsOnly && tableItems.length > 0 ? (
              <Command.Group
                heading="Tables"
                className="mt-1 px-1 py-1 text-2xs uppercase tracking-wide text-text-muted"
              >
                {tableItems.map((item) => {
                  const action = actionsByKey.get(item.key);
                  return (
                    <PaletteRow
                      key={item.key}
                      icon={<IconTable className="size-3.5" />}
                      label={item.label}
                      description={item.connectionName}
                      onSelect={() => {
                        if (action) runItem(item.key, action.run);
                      }}
                    />
                  );
                })}
              </Command.Group>
            ) : null}

            {!commandsOnly && savedQueryItems.length > 0 ? (
              <Command.Group
                heading="Saved queries"
                className="mt-1 px-1 py-1 text-2xs uppercase tracking-wide text-text-muted"
              >
                {savedQueryItems.map((saved) => {
                  const key = `saved:${saved.id}`;
                  const action = actionsByKey.get(key);
                  return (
                    <PaletteRow
                      key={key}
                      icon={<IconBookmark className="size-3.5" />}
                      label={saved.name}
                      description={
                        saved.body.length > 60
                          ? `${saved.body.slice(0, 60)}…`
                          : saved.body
                      }
                      onSelect={() => {
                        if (action) runItem(key, action.run);
                      }}
                    />
                  );
                })}
              </Command.Group>
            ) : null}
          </Command.List>
        </Command>
      </DialogContent>
    </Dialog>
  );
}

function PaletteRow({
  icon,
  label,
  description,
  keys,
  onSelect,
}: {
  icon: React.ReactNode;
  label: string;
  description?: string;
  keys?: ReadonlyArray<string>;
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
      <span className="ml-auto flex shrink-0 items-center gap-2">
        {description ? (
          <span className="max-w-48 truncate text-2xs text-text-muted">
            {description}
          </span>
        ) : null}
        {keys && keys.length > 0 ? (
          <Kbd bare keys={keys} className="text-text-muted" />
        ) : null}
      </span>
    </Command.Item>
  );
}
