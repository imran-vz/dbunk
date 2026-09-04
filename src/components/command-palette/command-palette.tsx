/**
 * Open Anything palette (DESIGN-SYSTEM §4.10, Plan 010 mock A).
 * `Cmd+K`, one unified surface on the Dialog primitive: a bare query
 * ranks everything the app can navigate to — commands, open tabs,
 * connections (including disconnected ones), schemas, relations,
 * catalog objects, saved queries, and history — through
 * `src/lib/open-anything.ts`;
 * a `>` prefix restricts to commands.
 *
 * Results render as ONE flat ranked list with inline kind badges —
 * never fixed group buckets. cmdk's own filtering is disabled
 * (`shouldFilter={false}`), so DOM order is rank order, and selection
 * is controlled so the top-ranked row always owns Enter as the query
 * changes. Truncated kinds are disclosed in a footer (no silent caps).
 *
 * Commands come from the central shortcut registry (§6.1); selection
 * frecency persists per item key through the P8 UI-state store.
 */

import {
  IconBolt,
  IconBookmark,
  IconDatabase,
  IconFolder,
  IconHistory,
  IconLayoutColumns,
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
import { toast } from "sonner";

import { Dialog, DialogContent } from "@/components/ui/dialog";
import { Kbd } from "@/components/ui/kbd";
import {
  buildOpenAnythingIndex,
  type OpenAnythingItem,
  type OpenAnythingTarget,
  rankOpenAnythingItems,
  type RelationKind,
  resolveSavedQueryTarget,
} from "@/lib/open-anything";
import {
  dispatchShortcut,
  getShortcutRegistryVersion,
  hasShortcutHandler,
  SHORTCUTS,
  subscribeShortcutRegistry,
} from "@/lib/shortcuts";
import type { PgObjectKind } from "@/lib/store";
import { useAppStore } from "@/lib/store";
import { uiGet, uiSet } from "@/lib/ui-state";
import { cn } from "@/lib/utils";

const FRECENCY_KEY = "dbunk.palette.frecency";

type FrecencyMap = Record<string, { count: number; last: number }>;

/** Pre-Open-Anything table keys: `table:<conn>::<schema>::<table>`. */
const LEGACY_TABLE_KEY = /^table:(.+?)::(.+?)::(.+)$/;

/**
 * Migrate persisted `table:` keys onto the `relation:` keys the index
 * emits now, so a user's most-used tables survive the upgrade instead
 * of sitting orphaned in the map until they age out.
 */
const migrateLegacyFrecencyKeys = (map: FrecencyMap): FrecencyMap => {
  let changed = false;
  const next: FrecencyMap = {};
  const merge = (key: string, entry: FrecencyMap[string]) => {
    const existing = next[key];
    next[key] = existing
      ? {
          count: existing.count + entry.count,
          last: Math.max(existing.last, entry.last),
        }
      : entry;
  };
  for (const [key, entry] of Object.entries(map)) {
    const legacy = LEGACY_TABLE_KEY.exec(key);
    if (!legacy) {
      merge(key, entry);
      continue;
    }
    changed = true;
    merge(`relation:${legacy[1]}:${legacy[2]}:${legacy[3]}`, entry);
  }
  return changed ? next : map;
};

const readFrecency = (): FrecencyMap => {
  try {
    const raw = uiGet(FRECENCY_KEY);
    if (!raw) return {};
    // SAFETY: scores are advisory ordering hints; malformed entries fall out below.
    const parsed = JSON.parse(raw) as FrecencyMap;
    // oxlint-disable-next-line anti-slop/no-runtime-typeof -- persisted-value validation.
    return parsed && typeof parsed === "object"
      ? migrateLegacyFrecencyKeys(parsed)
      : {};
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

/** Stored `{count,last}` map → the ranker's decayed-score map. */
const toFrecencyScores = (map: FrecencyMap): ReadonlyMap<string, number> =>
  new Map(
    Object.entries(map).map(([key, entry]) => [key, frecencyScore(entry)]),
  );

const KIND_BADGES = {
  command: { label: "Cmd", className: "text-text-secondary" },
  tab: { label: "Tab", className: "text-text-secondary" },
  connection: { label: "Conn", className: "text-warning" },
  schema: { label: "Schema", className: "text-success" },
  relation: { label: "Object", className: "text-info" },
  object: { label: "Object", className: "text-info" },
  "saved-query": { label: "Saved", className: "text-warning" },
  history: { label: "Hist", className: "text-text-muted" },
} satisfies Record<
  OpenAnythingItem["kind"],
  { label: string; className: string }
>;

/** Relation kinds get their concrete badge instead of "Object". */
const RELATION_BADGES = {
  table: "Table",
  view: "View",
  "materialized-view": "MatView",
  "foreign-table": "Foreign",
} satisfies Record<RelationKind, string>;

const OBJECT_BADGES = {
  schema: "Schema",
  table: "Table",
  view: "View",
  "materialized-view": "MatView",
  "foreign-table": "Foreign",
  sequence: "Seq",
  function: "Fn",
  procedure: "Proc",
  aggregate: "Agg",
  type: "Type",
  domain: "Domain",
  extension: "Ext",
} satisfies Record<PgObjectKind, string>;

const KIND_ICONS = {
  command: <IconBolt className="size-3.5" />,
  tab: <IconLayoutColumns className="size-3.5" />,
  connection: <IconDatabase className="size-3.5" />,
  schema: <IconFolder className="size-3.5" />,
  relation: <IconTable className="size-3.5" />,
  object: <IconDatabase className="size-3.5" />,
  "saved-query": <IconBookmark className="size-3.5" />,
  history: <IconHistory className="size-3.5" />,
} satisfies Record<OpenAnythingItem["kind"], React.ReactNode>;

export function CommandPalette() {
  const [open, setOpen] = useState(false);
  const [query, setQuery] = useState("");
  const [selectedKey, setSelectedKey] = useState("");
  const connections = useAppStore((state) => state.connections);
  const schemaExplorer = useAppStore((state) => state.schemaExplorer);
  const pgObjectCatalog = useAppStore((state) => state.pgObjectCatalog);
  const savedQueries = useAppStore((state) => state.savedQueries);
  const queryHistory = useAppStore((state) => state.queryHistory);
  const workspaceTabs = useAppStore((state) => state.workspaceTabs);

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

  const close = useCallback(() => setOpen(false), []);

  // `>` prefix restricts to commands (§4.10).
  const commandsOnly = query.trimStart().startsWith(">");
  const effectiveQuery = commandsOnly
    ? query.trimStart().slice(1).trimStart()
    : query;

  const commands = useMemo(() => {
    void registryVersion;
    return SHORTCUTS.filter(
      (shortcut) =>
        shortcut.id !== "command-palette" && hasShortcutHandler(shortcut.id),
    ).map((shortcut) => ({
      id: shortcut.id,
      label: shortcut.label,
      description: shortcut.group,
    }));
  }, [registryVersion]);

  const index = useMemo(() => {
    if (!open) return [];
    return buildOpenAnythingIndex({
      connections,
      schemaExplorer,
      objectCatalog: Object.fromEntries(
        Object.entries(pgObjectCatalog).map(([connectionId, state]) => [
          connectionId,
          state.status === "ready" ? state.catalog : undefined,
        ]),
      ),
      savedQueries,
      queryHistory,
      workspaceTabs,
      commands,
    });
  }, [
    open,
    connections,
    schemaExplorer,
    pgObjectCatalog,
    savedQueries,
    queryHistory,
    workspaceTabs,
    commands,
  ]);

  // Frecency snapshot per open — selections while open re-rank on the
  // next open, which keeps typing cheap.
  const frecency = useMemo(() => {
    if (!open) return new Map<string, number>();
    return toFrecencyScores(readFrecency());
  }, [open]);

  const ranked = useMemo(() => {
    const scope = commandsOnly
      ? index.filter((item) => item.kind === "command")
      : index;
    return rankOpenAnythingItems(scope, effectiveQuery, frecency);
  }, [index, commandsOnly, effectiveQuery, frecency]);

  // Controlled selection: the top-ranked row owns Enter whenever the
  // query changes; arrow keys still move it via onValueChange.
  const firstKey = ranked.items[0]?.key ?? "";
  useEffect(() => {
    setSelectedKey(firstKey);
    // eslint-disable-next-line react-hooks/exhaustive-deps -- reset on query change only.
  }, [effectiveQuery, open, firstKey]);

  const truncatedTotal = Object.values(ranked.truncated).reduce(
    (sum, count) => sum + (count ?? 0),
    0,
  );

  const runTarget = useCallback((target: OpenAnythingTarget) => {
    const store = useAppStore.getState();
    switch (target.type) {
      case "command":
        dispatchShortcut(target.commandId);
        return;
      case "new-table":
        if (target.connectionId !== store.activeConnectionId) {
          useAppStore.setState({ activeConnectionId: target.connectionId });
        }
        useAppStore.getState().openTableDesignerTab(target.schema);
        return;
      case "connect":
        useAppStore.setState({ activeConnectionId: target.connectionId });
        void store.connectConnection(target.connectionId);
        return;
      case "activate-tab": {
        const tab = store.workspaceTabs.find(
          (item) => item.id === target.tabId,
        );
        if (!tab) return;
        if (tab.connectionId !== store.activeConnectionId) {
          useAppStore.setState({ activeConnectionId: tab.connectionId });
        }
        store.setActiveTabId(target.tabId);
        return;
      }
      case "open-relation": {
        if (target.connectionId !== store.activeConnectionId) {
          useAppStore.setState({ activeConnectionId: target.connectionId });
        }
        // Views/matviews/foreign tables open as SELECT query tabs —
        // the table-browse contract is tables-only (Plan 010 §2).
        if (target.relationKind === "table") {
          useAppStore.getState().openTableTab(target.schema, target.name);
        } else {
          useAppStore.getState().openViewTab(target.schema, target.name);
        }
        return;
      }
      case "reveal-schema":
        store.revealSchemaInNavigator(target.connectionId, target.schema);
        return;
      case "open-object":
        if (target.connectionId !== store.activeConnectionId) {
          useAppStore.setState({ activeConnectionId: target.connectionId });
        }
        useAppStore.getState().openObjectTab(target.reference);
        return;
      case "open-saved-query": {
        const saved = store.savedQueries.find(
          (item) => item.id === target.savedQueryId,
        );
        if (!saved) return;
        const resolved = resolveSavedQueryTarget(
          saved,
          store.connections,
          store.activeConnectionId,
          store.schemaExplorer,
        );
        if (!resolved.ok) {
          toast.error(resolved.reason);
          return;
        }
        store.openWorkspaceTab({
          kind: "query",
          label: `${saved.name}.sql`,
          connectionId: resolved.connectionId,
          schema: resolved.schema,
          query: saved.body,
        });
        return;
      }
      case "open-history-entry": {
        const entry = store.queryHistory.find(
          (item) => item.id === target.historyId,
        );
        if (!entry) return;
        store.reopenHistoryEntry({
          sql: entry.sql,
          connectionId: entry.connectionId,
        });
        return;
      }
      default: {
        // Compile-time exhaustiveness: a new target type must be
        // handled here, not silently ignored on Enter.
        const unhandled: never = target;
        return unhandled;
      }
    }
  }, []);

  const runItem = useCallback(
    (item: OpenAnythingItem) => {
      recordUse(item.key);
      close();
      runTarget(item.target);
    },
    [close, runTarget],
  );

  const shortcutKeysFor = (item: OpenAnythingItem) => {
    if (item.target.type !== "command") return undefined;
    const commandId = item.target.commandId;
    return SHORTCUTS.find((shortcut) => shortcut.id === commandId)?.keys;
  };

  return (
    <Dialog open={open} onOpenChange={setOpen}>
      <DialogContent
        size="lg"
        aria-label="Open Anything"
        className="top-[15vh] max-h-[70vh] translate-y-0 gap-0 p-0"
      >
        <Command
          label="Open Anything"
          shouldFilter={false}
          value={selectedKey}
          onValueChange={setSelectedKey}
          className="flex min-h-0 flex-col"
        >
          <Command.Input
            value={query}
            onValueChange={setQuery}
            placeholder="Open anything: tables, objects, connections, queries… (> for commands)"
            className="w-full border-b border-border-subtle bg-transparent px-4 py-3 text-sm outline-none placeholder:text-text-muted"
            // oxlint-disable-next-line jsx-a11y/no-autofocus -- palette opens focused by definition.
            autoFocus
          />
          <Command.List className="max-h-[55vh] min-h-0 overflow-auto p-1">
            <Command.Empty className="px-4 py-6 text-center text-xs text-text-muted">
              No matches.
            </Command.Empty>
            {ranked.items.map((item) => (
              <OpenAnythingRow
                key={item.key}
                item={item}
                keys={shortcutKeysFor(item)}
                onSelect={() => runItem(item)}
              />
            ))}
          </Command.List>
          {truncatedTotal > 0 ? (
            <div
              data-testid="palette-truncation"
              className="border-t border-border-subtle px-4 py-2 text-2xs text-text-muted"
            >
              {truncatedTotal} more {truncatedTotal === 1 ? "match" : "matches"}{" "}
              . Keep typing to narrow.
            </div>
          ) : null}
        </Command>
      </DialogContent>
    </Dialog>
  );
}

function badgeFor(item: OpenAnythingItem) {
  if (item.kind === "relation" && item.target.type === "open-relation") {
    return {
      label: RELATION_BADGES[item.target.relationKind],
      className: KIND_BADGES.relation.className,
    };
  }
  if (item.kind === "object" && item.target.type === "open-object") {
    return {
      label: OBJECT_BADGES[item.target.reference.kind],
      className: KIND_BADGES.object.className,
    };
  }
  return KIND_BADGES[item.kind];
}

function OpenAnythingRow({
  item,
  keys,
  onSelect,
}: {
  item: OpenAnythingItem;
  keys?: ReadonlyArray<string>;
  onSelect: () => void;
}) {
  const badge = badgeFor(item);
  return (
    <Command.Item
      value={item.key}
      onSelect={onSelect}
      className={cn(
        "flex items-center gap-2 rounded-md px-2 py-1.5 text-xs text-text-secondary",
        "data-[selected=true]:bg-surface-panel-elevated data-[selected=true]:text-foreground",
      )}
    >
      <span className="text-text-muted">{KIND_ICONS[item.kind]}</span>
      <span
        className={cn(
          "shrink-0 rounded border border-border-subtle bg-surface-panel px-1.5 text-2xs font-semibold uppercase tracking-wide",
          badge.className,
        )}
      >
        {badge.label}
      </span>
      <span className="truncate">{item.label}</span>
      <span className="ml-auto flex shrink-0 items-center gap-2">
        {item.description ? (
          <span className="max-w-48 truncate text-2xs text-text-muted">
            {item.description}
          </span>
        ) : null}
        {keys && keys.length > 0 ? (
          <Kbd bare keys={keys} className="text-text-muted" />
        ) : null}
      </span>
    </Command.Item>
  );
}
