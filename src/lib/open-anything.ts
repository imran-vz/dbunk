/**
 * Open Anything index + ranking (Plan 009, PAR-005).
 *
 * Pure module: everything arrives through an injected snapshot and the
 * result is plain data, so the palette (Plan 010) stays a thin renderer
 * and every ranking rule is unit-testable without a store.
 *
 * Deliberate boundaries:
 * - Relations keep their browse/query targets, while catalog kinds with an
 *   Object Viewer use overload-safe `open-object` targets.
 * - Caps are applied per kind AFTER ranking, and the cut counts are
 *   reported via `truncated` so the UI can disclose them (no silent
 *   caps — register requirement).
 * - Item `key` doubles as the frecency key and must stay stable across
 *   releases (`saved:<id>` keys predate this module).
 */

import { canonicalPgObjectRefKey } from "@/lib/pg-object-ref";
import {
  type Connection,
  isConnectedStatus,
  type PgCatalogEntry,
  type PgObjectCatalog,
  type PgObjectKind,
  type PgObjectRef,
  type PgSchemaObjects,
  type QueryHistoryEntry,
  type SavedQuery,
  type SchemaExplorer,
  type WorkspaceTab,
} from "@/lib/store";

export type RelationKind =
  | "table"
  | "view"
  | "materialized-view"
  | "foreign-table";

export type OpenAnythingTarget =
  | { type: "command"; commandId: string }
  | {
      type: "new-table";
      connectionId: string;
      schema: string;
    }
  | { type: "connect"; connectionId: string }
  | { type: "activate-tab"; tabId: string }
  | {
      type: "open-relation";
      connectionId: string;
      schema: string;
      name: string;
      relationKind: RelationKind;
    }
  | { type: "reveal-schema"; connectionId: string; schema: string }
  | {
      type: "open-object";
      connectionId: string;
      reference: PgObjectRef;
    }
  | { type: "open-saved-query"; savedQueryId: string }
  | { type: "open-history-entry"; historyId: string };

export type OpenAnythingKind =
  | "command"
  | "tab"
  | "connection"
  | "schema"
  | "relation"
  | "object"
  | "saved-query"
  | "history";

export type OpenAnythingItem = {
  /** Stable identity; doubles as the frecency key. */
  key: string;
  kind: OpenAnythingKind;
  label: string;
  description?: string;
  /** Lowercase haystack the ranker scores in addition to the label. */
  keywords: string;
  target: OpenAnythingTarget;
};

export type OpenAnythingCommand = {
  id: string;
  label: string;
  description?: string;
};

export type OpenAnythingSnapshot = {
  connections: Connection[];
  schemaExplorer: Record<string, SchemaExplorer[] | undefined>;
  objectCatalog: Record<string, PgObjectCatalog | undefined>;
  savedQueries: SavedQuery[];
  queryHistory: QueryHistoryEntry[];
  workspaceTabs: WorkspaceTab[];
  commands: OpenAnythingCommand[];
};

export type RankedOpenAnything = {
  /** Ranked best-first across kinds; regroup in the UI as needed. */
  items: OpenAnythingItem[];
  /** Per-kind count of ranked matches cut by the kind cap. */
  truncated: Partial<Record<OpenAnythingKind, number>>;
};

/** Per-kind result caps, applied after ranking. */
const KIND_CAPS = new Map<OpenAnythingKind, number>([
  ["connection", 20],
  ["schema", 20],
  ["relation", 200],
  ["object", 200],
  ["saved-query", 50],
  ["history", 25],
]);

/** History entries considered at index time (most recent first). */
const HISTORY_INDEX_LIMIT = 50;

/** Items surfaced for the empty query from frecency. */
const RECENT_LIMIT = 6;

/**
 * Ceiling on the per-item frecency boost inside a non-empty query.
 * Kept below the smallest gap between scoring tiers (subsequence 40 →
 * substring 100 → word-boundary 200 → prefix 300 → exact 400) so
 * frecency reorders within a relevance tier but can never lift a
 * weaker match above a stronger one. Raw frecency values stay
 * unbounded for empty-query recents ordering, where relevance doesn't
 * apply.
 */
const FRECENCY_BOOST_CAP = 50;

const RELATION_SOURCES: ReadonlyArray<{
  field: keyof Pick<
    SchemaExplorer,
    "tables" | "views" | "materializedViews" | "foreignTables"
  >;
  relationKind: RelationKind;
  badge: string;
}> = [
  { field: "tables", relationKind: "table", badge: "Table" },
  { field: "views", relationKind: "view", badge: "View" },
  {
    field: "materializedViews",
    relationKind: "materialized-view",
    badge: "Materialized view",
  },
  {
    field: "foreignTables",
    relationKind: "foreign-table",
    badge: "Foreign table",
  },
];

type PaletteObjectKind = Extract<
  PgObjectKind,
  | "sequence"
  | "function"
  | "procedure"
  | "aggregate"
  | "type"
  | "domain"
  | "extension"
>;

const OBJECT_SOURCES: ReadonlyArray<{
  field: keyof Pick<
    PgSchemaObjects,
    | "sequences"
    | "functions"
    | "procedures"
    | "aggregates"
    | "types"
    | "domains"
    | "extensions"
  >;
  objectKind: PaletteObjectKind;
  badge: string;
}> = [
  { field: "functions", objectKind: "function", badge: "Fn" },
  { field: "procedures", objectKind: "procedure", badge: "Proc" },
  { field: "aggregates", objectKind: "aggregate", badge: "Agg" },
  { field: "sequences", objectKind: "sequence", badge: "Seq" },
  { field: "types", objectKind: "type", badge: "Type" },
  { field: "domains", objectKind: "domain", badge: "Domain" },
  { field: "extensions", objectKind: "extension", badge: "Ext" },
];

const paletteObjectRef = (
  kind: PaletteObjectKind,
  schema: string,
  entry: PgCatalogEntry,
): PgObjectRef => {
  if (kind === "function" || kind === "procedure" || kind === "aggregate") {
    return {
      kind,
      schema,
      name: entry.name,
      identityArgs: entry.identityArgs ?? "",
    };
  }
  return { kind, schema, name: entry.name, identityArgs: null };
};

function firstLine(sql: string, max: number): string {
  const line = sql.trim().split("\n", 1)[0].trim();
  return line.length > max ? `${line.slice(0, max - 1)}…` : line;
}

export function buildOpenAnythingIndex(
  snapshot: OpenAnythingSnapshot,
): OpenAnythingItem[] {
  const items: OpenAnythingItem[] = [];
  const connectionNames = new Map(
    snapshot.connections.map((connection) => [connection.id, connection.name]),
  );

  for (const command of snapshot.commands) {
    items.push({
      key: `command:${command.id}`,
      kind: "command",
      label: command.label,
      description: command.description,
      keywords: `${command.label} ${command.description ?? ""}`.toLowerCase(),
      target: { type: "command", commandId: command.id },
    });
  }

  for (const tab of snapshot.workspaceTabs) {
    if (
      tab.kind !== "query" &&
      tab.kind !== "table" &&
      tab.kind !== "table-designer" &&
      tab.kind !== "object"
    ) {
      continue;
    }
    const connectionName = connectionNames.get(tab.connectionId) ?? "";
    items.push({
      key: `tab:${tab.id}`,
      kind: "tab",
      label: tab.label,
      description: connectionName ? `Open tab · ${connectionName}` : "Open tab",
      keywords:
        `${tab.label} ${tab.table ?? ""} ${tab.objectRef?.kind ?? ""} ${tab.objectRef?.schema ?? ""} ${tab.objectRef?.name ?? ""} ${connectionName}`.toLowerCase(),
      target: { type: "activate-tab", tabId: tab.id },
    });
  }

  for (const connection of snapshot.connections) {
    items.push({
      key: `connection:${connection.id}`,
      kind: "connection",
      label: connection.name,
      description: isConnectedStatus(connection.status)
        ? `${connection.engine} · connected`
        : `${connection.engine} · connect`,
      keywords:
        `${connection.name} ${connection.engine} ${connection.host} ${connection.database}`.toLowerCase(),
      target: { type: "connect", connectionId: connection.id },
    });

    if (!isConnectedStatus(connection.status)) continue;
    const schemas = snapshot.schemaExplorer[connection.id] ?? [];
    for (const schema of schemas) {
      if (connection.engine === "PostgreSQL") {
        items.push({
          key: `command:new-table:${connection.id}:${schema.name}`,
          kind: "command",
          label: `New table in ${schema.name}`,
          keywords:
            `create new table ${schema.name} ${connection.name}`.toLowerCase(),
          target: {
            type: "new-table",
            connectionId: connection.id,
            schema: schema.name,
          },
        });
      }
      items.push({
        key: `schema:${connection.id}:${schema.name}`,
        kind: "schema",
        label: schema.name,
        description: `Schema · ${connection.name}`,
        keywords: `${schema.name} ${connection.name} schema`.toLowerCase(),
        target: {
          type: "reveal-schema",
          connectionId: connection.id,
          schema: schema.name,
        },
      });
      for (const source of RELATION_SOURCES) {
        for (const name of schema[source.field] ?? []) {
          items.push({
            key: `relation:${connection.id}:${schema.name}:${name}`,
            kind: "relation",
            label: name,
            description: `${source.badge} · ${schema.name} · ${connection.name}`,
            keywords:
              `${schema.name}.${name} ${source.badge} ${connection.name}`.toLowerCase(),
            target: {
              type: "open-relation",
              connectionId: connection.id,
              schema: schema.name,
              name,
              relationKind: source.relationKind,
            },
          });
        }
      }
    }

    const catalog = snapshot.objectCatalog[connection.id];
    for (const schema of catalog?.schemas ?? []) {
      for (const source of OBJECT_SOURCES) {
        for (const entry of schema[source.field]) {
          const reference = paletteObjectRef(
            source.objectKind,
            schema.name,
            entry,
          );
          items.push({
            key: `object:${connection.id}:${canonicalPgObjectRefKey(reference)}`,
            kind: "object",
            label:
              reference.identityArgs === null
                ? reference.name
                : `${reference.name}(${reference.identityArgs})`,
            description: `${source.badge} · ${schema.name} · ${connection.name}`,
            keywords:
              `${schema.name}.${reference.name} ${reference.identityArgs ?? ""} ${source.badge} ${source.objectKind} ${connection.name}`.toLowerCase(),
            target: {
              type: "open-object",
              connectionId: connection.id,
              reference,
            },
          });
        }
      }
    }
  }

  for (const saved of snapshot.savedQueries) {
    const connectionName = saved.connectionId
      ? (connectionNames.get(saved.connectionId) ?? "")
      : "";
    items.push({
      key: `saved:${saved.id}`,
      kind: "saved-query",
      label: saved.name,
      description: connectionName
        ? `Saved query · ${connectionName}`
        : "Saved query",
      keywords:
        `${saved.name} ${firstLine(saved.body, 120)} ${connectionName}`.toLowerCase(),
      target: { type: "open-saved-query", savedQueryId: saved.id },
    });
  }

  for (const entry of snapshot.queryHistory.slice(0, HISTORY_INDEX_LIMIT)) {
    items.push({
      key: `history:${entry.id}`,
      kind: "history",
      label: firstLine(entry.sql, 80),
      description: `History · ${entry.connectionName} · ${entry.status}`,
      keywords:
        `${entry.sql.slice(0, 400)} ${entry.connectionName}`.toLowerCase(),
      target: { type: "open-history-entry", historyId: entry.id },
    });
  }

  return items;
}

function isWordBoundaryMatch(haystack: string, needle: string): boolean {
  let from = 0;
  while (true) {
    const at = haystack.indexOf(needle, from);
    if (at < 0) return false;
    if (at === 0) return true;
    const before = haystack[at - 1];
    if (!/[a-z0-9]/.test(before)) return true;
    from = at + 1;
  }
}

function isSubsequence(haystack: string, needle: string): boolean {
  let position = 0;
  for (const char of needle) {
    position = haystack.indexOf(char, position);
    if (position < 0) return false;
    position += 1;
  }
  return true;
}

/** Score one query token against an item; 0 = no match. */
function scoreToken(item: OpenAnythingItem, token: string): number {
  const label = item.label.toLowerCase();
  if (label === token) return 400;
  if (label.startsWith(token)) return 300;
  if (isWordBoundaryMatch(item.keywords, token)) return 200;
  if (item.keywords.includes(token)) return 100;
  if (isSubsequence(label, token)) return 40;
  return 0;
}

/**
 * Rank items for a query. Every whitespace-separated token must match
 * (AND semantics); scores sum across tokens plus a frecency boost.
 * With an empty query, returns frecency recents plus open tabs and
 * commands — the "just opened ⌘K" view.
 */
export function rankOpenAnythingItems(
  items: OpenAnythingItem[],
  query: string,
  frecency: ReadonlyMap<string, number>,
): RankedOpenAnything {
  const tokens = query.toLowerCase().split(/\s+/).filter(Boolean);

  if (tokens.length === 0) {
    const recents = items
      .filter((item) => (frecency.get(item.key) ?? 0) > 0)
      .sort((a, b) => (frecency.get(b.key) ?? 0) - (frecency.get(a.key) ?? 0))
      .slice(0, RECENT_LIMIT);
    const recentKeys = new Set(recents.map((item) => item.key));
    const ambient = items.filter(
      (item) =>
        (item.kind === "tab" || item.kind === "command") &&
        !recentKeys.has(item.key),
    );
    return { items: [...recents, ...ambient], truncated: {} };
  }

  const scored: Array<{ item: OpenAnythingItem; score: number }> = [];
  for (const item of items) {
    let score = 0;
    let matched = true;
    for (const token of tokens) {
      const tokenScore = scoreToken(item, token);
      if (tokenScore === 0) {
        matched = false;
        break;
      }
      score += tokenScore;
    }
    if (!matched) continue;
    score += Math.min(frecency.get(item.key) ?? 0, FRECENCY_BOOST_CAP);
    scored.push({ item, score });
  }
  scored.sort(
    (a, b) => b.score - a.score || a.item.label.localeCompare(b.item.label),
  );

  const kindCounts = new Map<OpenAnythingKind, number>();
  const truncated: Partial<Record<OpenAnythingKind, number>> = {};
  const ranked: OpenAnythingItem[] = [];
  for (const { item } of scored) {
    const cap = KIND_CAPS.get(item.kind);
    const seen = kindCounts.get(item.kind) ?? 0;
    if (cap !== undefined && seen >= cap) {
      truncated[item.kind] = (truncated[item.kind] ?? 0) + 1;
      continue;
    }
    kindCounts.set(item.kind, seen + 1);
    ranked.push(item);
  }
  return { items: ranked, truncated };
}

export type SavedQueryOpenTarget =
  | { ok: true; connectionId: string; schema: string }
  | { ok: false; reason: string };

/**
 * Resolve where a saved query should open. Fixes the historical defect
 * where a stale `saved.connectionId` or an empty active connection
 * produced a tab with an empty connection id and a hardcoded schema:
 * the pinned connection wins only while it still exists, the active
 * connection is the fallback, and anything else is a typed refusal the
 * UI can surface instead of fabricating a broken tab. The tab's schema
 * is the target connection's first explored schema (the same rule
 * `createNewQueryTab` uses), falling back to `public` only when
 * nothing has been explored yet.
 */
export function resolveSavedQueryTarget(
  saved: SavedQuery,
  connections: Connection[],
  activeConnectionId: string,
  schemaExplorer: OpenAnythingSnapshot["schemaExplorer"] = {},
): SavedQueryOpenTarget {
  const exists = (id: string | null): id is string =>
    id !== null && connections.some((connection) => connection.id === id);
  const schemaFor = (connectionId: string): string =>
    schemaExplorer[connectionId]?.[0]?.name ?? "public";

  if (exists(saved.connectionId)) {
    return {
      ok: true,
      connectionId: saved.connectionId,
      schema: schemaFor(saved.connectionId),
    };
  }
  if (activeConnectionId && exists(activeConnectionId)) {
    return {
      ok: true,
      connectionId: activeConnectionId,
      schema: schemaFor(activeConnectionId),
    };
  }
  return {
    ok: false,
    reason:
      "This saved query isn't pinned to an existing connection — select a connection first.",
  };
}
