import { describe, expect, it } from "vitest";

import {
  buildOpenAnythingIndex,
  type OpenAnythingItem,
  type OpenAnythingSnapshot,
  rankOpenAnythingItems,
  resolveSavedQueryTarget,
} from "./open-anything";
import type {
  Connection,
  PgObjectCatalog,
  QueryHistoryEntry,
  SavedQuery,
} from "./store";

const NO_FRECENCY: ReadonlyMap<string, number> = new Map();

function pgConnection(
  id: string,
  name: string,
  status: Connection["status"],
): Connection {
  return {
    engine: "PostgreSQL",
    id,
    name,
    database: "postgres",
    host: "localhost",
    port: 5432,
    user: "postgres",
    password: "",
    role: "read/write",
    ssl: true,
    status,
    latency: "",
  };
}

function savedQuery(
  id: string,
  name: string,
  connectionId: string | null,
): SavedQuery {
  return {
    id,
    name,
    body: "SELECT 1",
    connectionId,
    isFavorite: false,
    ownerId: null,
    createdAt: "2026-08-24T00:00:00Z",
    updatedAt: "2026-08-24T00:00:00Z",
  };
}

function historyEntry(id: string, sql: string): QueryHistoryEntry {
  return {
    id,
    sql,
    connectionId: "conn-a",
    connectionName: "Primary",
    database: "postgres",
    engine: "PostgreSQL",
    status: "success",
    runtimeMs: 12,
    startedAt: "2026-08-24T00:00:00Z",
  };
}

function snapshot(
  overrides: Partial<OpenAnythingSnapshot> = {},
): OpenAnythingSnapshot {
  return {
    connections: [
      pgConnection("conn-a", "Primary", "Connected"),
      pgConnection("conn-b", "Analytics", "Disconnected"),
    ],
    schemaExplorer: {
      "conn-a": [
        {
          name: "public",
          tables: ["users", "orders"],
          views: ["active_users"],
          materializedViews: ["daily_totals"],
          foreignTables: ["remote_events"],
          functions: ["compute_totals"],
          sequences: ["users_id_seq"],
        },
      ],
      // Present but must be ignored: conn-b is disconnected.
      "conn-b": [{ name: "reporting", tables: ["facts"] }],
    },
    objectCatalog: { "conn-a": objectCatalog() },
    savedQueries: [savedQuery("sq-1", "Slow queries", "conn-a")],
    queryHistory: [historyEntry("h-1", "SELECT * FROM users LIMIT 10")],
    workspaceTabs: [
      {
        id: "tab-1",
        kind: "query",
        label: "query_1.sql",
        connectionId: "conn-a",
        schema: "public",
      },
      {
        id: "tab-redis",
        kind: "key",
        label: "cache:user:1",
        connectionId: "conn-c",
        schema: "",
      },
      {
        id: "tab-object",
        kind: "object",
        label: "public.users_id_seq",
        connectionId: "conn-a",
        schema: "public",
        objectRef: {
          kind: "sequence",
          schema: "public",
          name: "users_id_seq",
          identityArgs: null,
        },
      },
    ],
    commands: [
      { id: "new-query", label: "New query tab", description: "Editor" },
    ],
    ...overrides,
  };
}

function objectCatalog(): PgObjectCatalog {
  return {
    schemas: [
      {
        name: "public",
        tables: [{ name: "users" }],
        views: [{ name: "active_users" }],
        materializedViews: [{ name: "daily_totals" }],
        foreignTables: [{ name: "remote_events" }],
        sequences: [{ name: "users_id_seq" }],
        functions: [
          { name: "compute_totals", identityArgs: "integer" },
          { name: "compute_totals", identityArgs: "numeric" },
        ],
        procedures: [{ name: "archive_orders", identityArgs: "date" }],
        aggregates: [{ name: "median", identityArgs: "numeric" }],
        types: [{ name: "order_status", typeClass: "enum" }],
        domains: [{ name: "positive_amount" }],
        extensions: [{ name: "pgcrypto" }],
      },
    ],
    eventTriggers: [{ name: "audit_ddl" }],
    roles: [{ name: "app_writer" }],
    tablespaces: [{ name: "fast_disk" }],
    truncated: [],
  };
}

const byKey = (items: OpenAnythingItem[], key: string) =>
  items.find((item) => item.key === key);

describe("buildOpenAnythingIndex", () => {
  it("emits every navigable kind with stable keys", () => {
    const items = buildOpenAnythingIndex(snapshot());
    expect(byKey(items, "command:new-query")?.kind).toBe("command");
    expect(byKey(items, "tab:tab-1")?.kind).toBe("tab");
    expect(byKey(items, "tab:tab-object")?.target).toEqual({
      type: "activate-tab",
      tabId: "tab-object",
    });
    expect(byKey(items, "connection:conn-a")?.kind).toBe("connection");
    expect(byKey(items, "schema:conn-a:public")?.kind).toBe("schema");
    expect(byKey(items, "relation:conn-a:public:users")?.target).toEqual({
      type: "open-relation",
      connectionId: "conn-a",
      schema: "public",
      name: "users",
      relationKind: "table",
    });
    expect(
      byKey(items, "relation:conn-a:public:active_users")?.target,
    ).toMatchObject({ relationKind: "view" });
    expect(
      byKey(items, "relation:conn-a:public:daily_totals")?.target,
    ).toMatchObject({ relationKind: "materialized-view" });
    expect(
      byKey(items, "relation:conn-a:public:remote_events")?.target,
    ).toMatchObject({ relationKind: "foreign-table" });
    expect(byKey(items, "saved:sq-1")?.kind).toBe("saved-query");
    expect(byKey(items, "history:h-1")?.kind).toBe("history");
  });

  it("emits overload-safe object targets with stable frecency keys", () => {
    const items = buildOpenAnythingIndex(snapshot());
    const functionKey =
      'object:conn-a:["function","public","compute_totals","integer"]';
    expect(byKey(items, functionKey)).toEqual(
      expect.objectContaining({
        kind: "object",
        label: "compute_totals(integer)",
        description: "Fn · public · Primary",
        target: {
          type: "open-object",
          connectionId: "conn-a",
          reference: {
            kind: "function",
            schema: "public",
            name: "compute_totals",
            identityArgs: "integer",
          },
        },
      }),
    );
    expect(
      byKey(
        items,
        'object:conn-a:["function","public","compute_totals","numeric"]',
      ),
    ).toBeDefined();
  });

  it("skips non-workspace tabs and list-only catalog kinds", () => {
    const items = buildOpenAnythingIndex(snapshot());
    expect(byKey(items, "tab:tab-redis")).toBeUndefined();
    expect(items.some((item) => item.keywords.includes("audit_ddl"))).toBe(
      false,
    );
    expect(items.some((item) => item.keywords.includes("app_writer"))).toBe(
      false,
    );
    expect(items.some((item) => item.keywords.includes("fast_disk"))).toBe(
      false,
    );
    expect(byKey(items, "tab:tab-object")?.keywords).toContain("users_id_seq");
  });

  it("keeps views on the relation path without double-indexing them", () => {
    const items = buildOpenAnythingIndex(snapshot());
    expect(items.filter((item) => item.label === "active_users")).toHaveLength(
      1,
    );
    expect(byKey(items, "relation:conn-a:public:active_users")?.target).toEqual(
      {
        type: "open-relation",
        connectionId: "conn-a",
        schema: "public",
        name: "active_users",
        relationKind: "view",
      },
    );
  });

  it("includes disconnected connections as connect targets without their objects", () => {
    const items = buildOpenAnythingIndex(snapshot());
    const analytics = byKey(items, "connection:conn-b");
    expect(analytics?.target).toEqual({
      type: "connect",
      connectionId: "conn-b",
    });
    expect(analytics?.description).toContain("connect");
    expect(byKey(items, "schema:conn-b:reporting")).toBeUndefined();
    expect(byKey(items, "relation:conn-b:reporting:facts")).toBeUndefined();
  });

  it("caps indexed history at the 50 most recent entries", () => {
    const entries = Array.from({ length: 60 }, (_, index) =>
      historyEntry(`h-${index}`, `SELECT ${index}`),
    );
    const items = buildOpenAnythingIndex(snapshot({ queryHistory: entries }));
    expect(items.filter((item) => item.kind === "history")).toHaveLength(50);
    expect(byKey(items, "history:h-0")).toBeDefined();
    expect(byKey(items, "history:h-59")).toBeUndefined();
  });
});

describe("rankOpenAnythingItems", () => {
  it("orders exact > prefix > word-boundary > substring > subsequence", () => {
    const item = (
      key: string,
      label: string,
      keywords = label.toLowerCase(),
    ): OpenAnythingItem => ({
      key,
      kind: "relation",
      label,
      keywords,
      target: { type: "command", commandId: "x" },
    });
    const items = [
      item("sub", "reorders_queue", "zz"), // subsequence of label only
      item("substr", "zzz", "big_reorders_audit_xordersx"),
      item("boundary", "zzz", "public.orders extras"),
      item("prefix", "orders_archive"),
      item("exact", "orders"),
    ];
    const ranked = rankOpenAnythingItems(items, "orders", NO_FRECENCY);
    expect(ranked.items.map((entry) => entry.key)).toEqual([
      "exact",
      "prefix",
      "boundary",
      "substr",
      "sub",
    ]);
  });

  it("requires every token to match (AND semantics)", () => {
    const items = buildOpenAnythingIndex(snapshot());
    const ranked = rankOpenAnythingItems(items, "users primary", NO_FRECENCY);
    expect(ranked.items.length).toBeGreaterThan(0);
    for (const entry of ranked.items) {
      expect(entry.keywords).toContain("primary");
    }
    expect(
      rankOpenAnythingItems(items, "users zzzzzz", NO_FRECENCY).items,
    ).toHaveLength(0);
  });

  it("applies kind caps after ranking so the best match always survives", () => {
    const tables = Array.from({ length: 250 }, (_, index) => `noise_${index}`);
    // The only exact match sits last in input order — position must not matter.
    tables.push("target");
    const items = buildOpenAnythingIndex(
      snapshot({
        schemaExplorer: { "conn-a": [{ name: "public", tables }] },
      }),
    );
    const ranked = rankOpenAnythingItems(items, "t", NO_FRECENCY);
    const relations = ranked.items.filter((item) => item.kind === "relation");
    expect(relations).toHaveLength(200);
    expect(relations[0].label).toBe("target");
    expect(ranked.truncated.relation).toBe(51);
  });

  it("applies one shared cap across object kinds", () => {
    const catalog = objectCatalog();
    const schema = catalog.schemas[0];
    if (!schema) throw new Error("Object catalog fixture has no schema.");
    schema.functions = Array.from({ length: 150 }, (_, index) => ({
      name: `shared_${index}`,
      identityArgs: "integer",
    }));
    schema.sequences = Array.from({ length: 100 }, (_, index) => ({
      name: `shared_seq_${index}`,
    }));
    const items = buildOpenAnythingIndex(
      snapshot({ objectCatalog: { "conn-a": catalog } }),
    );

    const ranked = rankOpenAnythingItems(items, "shared", NO_FRECENCY);
    expect(ranked.items.filter((item) => item.kind === "object")).toHaveLength(
      200,
    );
    expect(ranked.truncated.object).toBe(50);
  });

  it("boosts frecent items and serves recents for the empty query", () => {
    const items = buildOpenAnythingIndex(snapshot());
    const frecency = new Map([["relation:conn-a:public:orders", 500]]);

    const ranked = rankOpenAnythingItems(items, "or", frecency);
    expect(ranked.items[0].key).toBe("relation:conn-a:public:orders");

    // The boost is capped below a scoring tier: an exact label match
    // must beat a weaker match no matter how frecent the latter is.
    const capped = rankOpenAnythingItems(
      items,
      "users",
      new Map([["relation:conn-a:public:active_users", 100_000]]),
    );
    expect(capped.items[0].key).toBe("relation:conn-a:public:users");

    const empty = rankOpenAnythingItems(items, "", frecency);
    expect(empty.items[0].key).toBe("relation:conn-a:public:orders");
    const rest = empty.items.slice(1);
    expect(rest.length).toBeGreaterThan(0);
    for (const entry of rest) {
      expect(["tab", "command"]).toContain(entry.kind);
    }
    expect(empty.truncated).toEqual({});
  });
});

describe("resolveSavedQueryTarget", () => {
  const connections = [pgConnection("conn-a", "Primary", "Connected")];

  it("prefers the pinned connection while it exists", () => {
    expect(
      resolveSavedQueryTarget(savedQuery("sq", "Q", "conn-a"), connections, ""),
    ).toEqual({ ok: true, connectionId: "conn-a", schema: "public" });
  });

  it("falls back to the active connection when the pin is stale", () => {
    expect(
      resolveSavedQueryTarget(
        savedQuery("sq", "Q", "conn-gone"),
        connections,
        "conn-a",
      ),
    ).toEqual({ ok: true, connectionId: "conn-a", schema: "public" });
  });

  it("refuses instead of fabricating an empty connection id", () => {
    const result = resolveSavedQueryTarget(
      savedQuery("sq", "Q", null),
      connections,
      "",
    );
    expect(result.ok).toBe(false);
    const stale = resolveSavedQueryTarget(
      savedQuery("sq", "Q", "conn-gone"),
      connections,
      "conn-also-gone",
    );
    expect(stale.ok).toBe(false);
  });
});
