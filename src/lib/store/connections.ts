/**
 * Connections slice — owns Connection records and Active Connection ID.
 *
 * The Connections slice is the entity-owner for the
 * delete-connection cleanup cascade (`store/README.md`). Today
 * `deleteConnection` cleans up the Schema Explorer cache inline; as
 * the per-slice cleanup methods land (relational-tables in commit 7,
 * etc.), entity-owner actions call `get().dropRelationalCachesForConnection(id)`
 * and friends instead of the inline `set` pattern.
 *
 * Helpers `hydrateConnection`, `toStoredConnection`, and
 * `applyConnectionUpdate` are slice-local — only Connections-slice
 * actions create or mutate Connection records.
 */

import type { StateCreator } from "zustand";

import { storageClassFor } from "@/lib/engine-policy";
import { formatLatencyMs } from "@/lib/format";
import { fetchRedisAclSelf } from "@/lib/redis/api";
import { errorToMessage, isTauri, tauriInvoke } from "@/lib/tauri";

import type {
  AppStoreState,
  Connection,
  OverviewTabId,
  RedisCapabilities,
  StoredConnection,
} from "./types";

type ConnectResult = {
  latencyMs: number;
  /** Populated when the target is a Redis server (Phase 1.1+). */
  redisCapabilities?: RedisCapabilities;
};

const hydrateConnection = (connection: StoredConnection): Connection => ({
  ...connection,
  status: "Disconnected",
  latency: "--",
});

/**
 * Strip the runtime fields (`status`, `latency`, `errorMessage`) from a
 * `Connection` to recover the `StoredConnection` wire shape. The
 * variant-specific fields (`ssl`, `useHttps`, etc.) pass through
 * untouched via the spread — TypeScript narrows on `engine` so the
 * returned union member matches the input variant.
 */
const toStoredConnection = (connection: Connection): StoredConnection => {
  const { status, latency, errorMessage, ...stored } = connection;
  // The runtime-field destructure leaves a shape whose engine tag plus
  // per-variant fields exactly satisfy `StoredConnection`; the cast is
  // narrowing the union the destructure widens.
  void status;
  void latency;
  void errorMessage;
  // SAFETY: Removing only runtime fields preserves the complete stored discriminated union variant.
  return stored as StoredConnection;
};

/**
 * Updates to a Connection's runtime fields (`status`, `latency`,
 * `errorMessage`, `lastActivityAt`). Variant-specific fields are
 * excluded because they're part of the Connection's engine identity,
 * not transient state.
 */
type ConnectionRuntimeUpdate = Partial<
  Pick<Connection, "status" | "latency" | "errorMessage" | "lastActivityAt">
>;

const applyConnectionUpdate = (
  connections: Connection[],
  connectionId: string,
  updates: ConnectionRuntimeUpdate,
): Connection[] =>
  connections.map((connection) =>
    connection.id === connectionId
      ? Object.assign({}, connection, updates)
      : connection,
  );

/**
 * The single frontend owner for connection teardown ordering. Session bindings
 * are closed before the backend connection, while local workspace state is
 * removed only after the backend operation succeeds.
 */
const teardownConnectionWorkspace = async (
  state: AppStoreState,
  connectionId: string,
  teardownBackend?: () => Promise<void>,
) => {
  await state.closeQuerySessionsForConnection(connectionId);
  await teardownBackend?.();
  state.dropOpenQueryStateForConnection(connectionId);
  state.dropRelationalCachesForConnection(connectionId);
  state.closeKeyTabsForConnection(connectionId);
  state.closePubSubSessionsForConnection(connectionId);
  state.closeTabsForConnection(connectionId);
};

export type ConnectionsSlice = {
  connections: Connection[];
  activeConnectionId: string;
  /**
   * Active sub-tab inside each connection's Overview surface. Keyed
   * by connection id; missing entries default to `"overview"` at the
   * consumer. Entries are dropped when the connection is disconnected
   * or deleted so the record can't drift away from `connections`.
   */
  connectionOverviewTab: Record<string, OverviewTabId>;
  /**
   * Last schema selected inside the schema-map Overview sub-tab.
   * Kept per connection so returning to a map resumes the graph the
   * user was arranging without leaking across connections.
   */
  connectionSchemaMapSchema: Record<string, string>;

  setActiveConnectionId: (id: string) => void;
  setConnectionOverviewTab: (connectionId: string, tab: OverviewTabId) => void;
  setConnectionSchemaMapSchema: (connectionId: string, schema: string) => void;
  loadConnections: () => Promise<void>;
  addConnection: (connection: Connection) => Promise<void>;
  updateConnection: (connection: Connection) => Promise<void>;
  deleteConnection: (connectionId: string) => Promise<void>;
  connectConnection: (connectionId: string) => Promise<void>;
  disconnectConnection: (connectionId: string) => Promise<void>;
  testConnection: (
    connection: StoredConnection,
  ) => Promise<
    | { ok: true; latencyMs: number; redisCapabilities?: RedisCapabilities }
    | { ok: false; error: string }
  >;
  /**
   * Bump `lastActivityAt` on a connection record. Called from
   * sibling slices (e.g., relational-queries `runQuery` success
   * path) so they don't have to mutate `connections` directly.
   * Owner-slice helper per the cross-slice write rule documented
   * in `store/README.md`.
   */
  applyConnectionActivity: (connectionId: string, at?: string) => void;
  runHealthChecks: () => Promise<void>;
};

export const createConnectionsSlice: StateCreator<
  AppStoreState,
  [],
  [],
  ConnectionsSlice
> = (set, get) => ({
  connections: [],
  activeConnectionId: "",
  connectionOverviewTab: {},
  connectionSchemaMapSchema: {},

  setActiveConnectionId: (id) => set({ activeConnectionId: id }),

  setConnectionOverviewTab: (connectionId, tab) => {
    if (!connectionId) {
      return;
    }
    set((state) => ({
      connectionOverviewTab: {
        ...state.connectionOverviewTab,
        [connectionId]: tab,
      },
    }));
  },

  setConnectionSchemaMapSchema: (connectionId, schema) => {
    if (!connectionId || !schema) {
      return;
    }
    set((state) => ({
      connectionSchemaMapSchema: {
        ...state.connectionSchemaMapSchema,
        [connectionId]: schema,
      },
    }));
  },

  loadConnections: async () => {
    if (!isTauri()) {
      return;
    }
    try {
      const stored = await tauriInvoke<StoredConnection[]>("load_connections");
      const connections = stored.map(hydrateConnection);
      const liveIds = new Set(connections.map((c) => c.id));
      set((state) => ({
        connections,
        activeConnectionId: connections.some(
          (connection) => connection.id === state.activeConnectionId,
        )
          ? state.activeConnectionId
          : (connections[0]?.id ?? ""),
        connectionOverviewTab: Object.fromEntries(
          Object.entries(state.connectionOverviewTab).filter(([key]) =>
            liveIds.has(key),
          ),
        ),
        connectionSchemaMapSchema: Object.fromEntries(
          Object.entries(state.connectionSchemaMapSchema).filter(([key]) =>
            liveIds.has(key),
          ),
        ),
      }));
    } catch (error) {
      console.error("Failed to load connections", error);
    }
  },

  addConnection: async (connection) => {
    if (!isTauri()) {
      set((state) => ({
        connections: [...state.connections, connection],
        activeConnectionId: connection.id,
      }));
      return;
    }
    try {
      const stored = await tauriInvoke<StoredConnection[]>("save_connection", {
        connection: toStoredConnection(connection),
      });
      const connections = stored.map(hydrateConnection);
      set({ connections, activeConnectionId: connection.id });
    } catch (error) {
      console.error("Failed to save connection", error);
      set((state) => ({
        connections: [...state.connections, connection],
        activeConnectionId: connection.id,
      }));
    }
  },

  updateConnection: async (connection) => {
    if (!isTauri()) {
      set((state) => ({
        connections: state.connections.map((c) =>
          c.id === connection.id ? connection : c,
        ),
      }));
      return;
    }
    try {
      const stored = await tauriInvoke<StoredConnection[]>("save_connection", {
        connection: toStoredConnection(connection),
      });
      const connections = stored.map(hydrateConnection);
      // Preserve the connection status for the updated connection
      const currentConnection = get().connections.find(
        (c) => c.id === connection.id,
      );
      if (currentConnection) {
        const updatedConnections = connections.map((c) =>
          c.id === connection.id
            ? {
                ...c,
                status: currentConnection.status,
                latency: currentConnection.latency,
                lastActivityAt: currentConnection.lastActivityAt,
              }
            : c,
        );
        set({ connections: updatedConnections });
      } else {
        set({ connections });
      }
    } catch (error) {
      console.error("Failed to update connection", error);
    }
  },

  deleteConnection: async (connectionId) => {
    const finalize = (connections: ReturnType<typeof get>["connections"]) => {
      set((state) => {
        const newActiveId =
          state.activeConnectionId === connectionId
            ? (connections[0]?.id ?? "")
            : state.activeConnectionId;
        const { [connectionId]: _droppedTab, ...remainingTabs } =
          state.connectionOverviewTab;
        const {
          [connectionId]: _droppedSchemaMapSchema,
          ...remainingSchemaMapSchemas
        } = state.connectionSchemaMapSchema;
        return {
          connections,
          activeConnectionId: newActiveId,
          connectionOverviewTab: remainingTabs,
          connectionSchemaMapSchema: remainingSchemaMapSchemas,
        };
      });
    };

    try {
      let connections = get().connections.filter(
        (connection) => connection.id !== connectionId,
      );
      const teardownBackend = isTauri()
        ? async () => {
            const stored = await tauriInvoke<StoredConnection[]>(
              "delete_connection",
              { payload: { connectionId } },
            );
            connections = stored.map(hydrateConnection);
          }
        : undefined;
      await teardownConnectionWorkspace(get(), connectionId, teardownBackend);
      finalize(connections);
    } catch (error) {
      console.error("Failed to delete connection", error);
    }
  },

  testConnection: async (connection) => {
    if (!isTauri()) {
      // In dev/storybook mode we can't actually connect — pretend it
      // succeeded so the UI flow stays exercised.
      return { ok: true, latencyMs: 0 };
    }
    try {
      // The connection arg is already a `StoredConnection` union member;
      // serde on the Rust side decodes the engine-tagged JSON into the
      // matching `StoredConnection::*` variant (ADR-0010).
      const result = await tauriInvoke<ConnectResult>("test_connection", {
        payload: { connection },
      });
      return {
        ok: true,
        latencyMs: result.latencyMs,
        redisCapabilities: result.redisCapabilities,
      };
    } catch (error) {
      return { ok: false, error: errorToMessage(error) };
    }
  },

  runHealthChecks: async () => {
    if (!isTauri()) {
      return;
    }
    // Only probe connections the user has explicitly connected. Pinging
    // every stored connection on a 30 s tick had the side-effect of auto-
    // connecting Disconnected entries (PG/MySQL/CH/Redis alike) on launch:
    // each "health check" opens a real socket and runs SELECT 1 / PING,
    // which flips the status to Connected even though the user never asked
    // for that engine. See ADR-0002 (revised 2026-05-12).
    const connectionIds = get()
      .connections.filter(
        (c) => c.status === "Connected" || c.status === "Read only",
      )
      .map((c) => c.id);
    if (connectionIds.length === 0) {
      return;
    }
    // Fan out in parallel; per-connection failures are local and
    // shouldn't block siblings.
    const results = await Promise.all(
      connectionIds.map(async (connectionId) => {
        try {
          const result = await tauriInvoke<
            | { state: "healthy"; latencyMs: number }
            | { state: "error"; error: string }
          >("health_check_connection", {
            payload: { connectionId },
          });
          return { connectionId, result };
        } catch (error) {
          return {
            connectionId,
            result: {
              state: "error" as const,
              error: errorToMessage(error),
            },
          };
        }
      }),
    );
    set((state) => {
      const next = state.connections.map((connection) => {
        const found = results.find((r) => r.connectionId === connection.id);
        if (!found) return connection;
        if (found.result.state === "healthy") {
          // Don't downgrade an explicit "Read only" status; the
          // health-check only proves reachability, not write capability.
          const status: Connection["status"] =
            connection.status === "Read only" ? "Read only" : "Connected";
          return {
            ...connection,
            status,
            latency: formatLatencyMs(found.result.latencyMs),
            lastActivityAt: new Date().toISOString(),
            errorMessage: undefined,
          };
        }
        return {
          ...connection,
          status: "Disconnected" as const,
          errorMessage: found.result.error,
        };
      });
      return { connections: next };
    });
  },

  applyConnectionActivity: (connectionId, at) => {
    if (!connectionId) return;
    const stamp = at ?? new Date().toISOString();
    set((state) => ({
      connections: applyConnectionUpdate(state.connections, connectionId, {
        lastActivityAt: stamp,
      }),
    }));
  },

  connectConnection: async (connectionId) => {
    if (!connectionId) {
      return;
    }
    set({ activeConnectionId: connectionId });
    if (!isTauri()) {
      set((state) => ({
        connections: applyConnectionUpdate(state.connections, connectionId, {
          status: "Connected",
          lastActivityAt: new Date().toISOString(),
          errorMessage: undefined,
        }),
      }));
      return;
    }
    const managedServer = get().managedServers.find(
      (server) => server.connectionId === connectionId,
    );
    if (managedServer?.status === "stopped") {
      set((state) => ({
        managedServers: state.managedServers.map((server) =>
          server.id === managedServer.id
            ? { ...server, status: "starting" }
            : server,
        ),
      }));
    }
    try {
      const result = await tauriInvoke<ConnectResult>("connect_connection", {
        payload: { connectionId },
      });
      // Schema introspection is a relational-only concept; the keyvalue
      // dispatch returns `Err("Schema explorer does not apply to Redis…")`
      // and would otherwise sink the whole connect flow back to
      // Disconnected. The keyspace browser loads keys lazily via its own
      // SCAN API, so there's nothing to prefetch for Redis here.
      const target = get().connections.find((c) => c.id === connectionId);
      const isRelational =
        target && storageClassFor(target.engine) === "relational";
      let schema: import("./types").SchemaExplorer[] | undefined;
      if (isRelational) {
        try {
          schema = await tauriInvoke<import("./types").SchemaExplorer[]>(
            "load_schema_explorer",
            { payload: { connectionId } },
          );
        } catch (schemaError) {
          console.error("Failed to load schema explorer", schemaError);
        }
      }
      set((state) => ({
        connections: applyConnectionUpdate(state.connections, connectionId, {
          status: "Connected",
          latency: formatLatencyMs(result.latencyMs),
          lastActivityAt: new Date().toISOString(),
          errorMessage: undefined,
        }),
        schemaExplorer: schema
          ? { ...state.schemaExplorer, [connectionId]: schema }
          : state.schemaExplorer,
      }));
      if (result.redisCapabilities) {
        get().setRedisCapabilities(connectionId, result.redisCapabilities);
      }
      const acltarget = get().connections.find((c) => c.id === connectionId);
      if (acltarget && storageClassFor(acltarget.engine) === "keyvalue") {
        // ACL self-probe — non-fatal on older servers / restricted
        // users. The keyspace browser simply skips the gating banner
        // when this is absent. Wrapped in try/Promise.resolve so a
        // mocked-undefined return (test environment) can't break the
        // outer try/catch.
        Promise.resolve()
          .then(() => fetchRedisAclSelf({ connectionId }))
          .then((acl) => {
            if (acl) get().setRedisAclSelf(connectionId, acl);
          })
          .catch(() => {});
      }
    } catch (error) {
      const message = errorToMessage(error);
      console.error("Failed to connect", error);
      set((state) => ({
        connections: applyConnectionUpdate(state.connections, connectionId, {
          status: "Disconnected",
          errorMessage: message,
        }),
      }));
    } finally {
      if (managedServer) {
        await get().loadManagedServers();
      }
    }
  },

  disconnectConnection: async (connectionId) => {
    if (!connectionId) {
      return;
    }
    const state = get();
    if (
      !state.connections.some((connection) => connection.id === connectionId)
    ) {
      return;
    }

    const teardownBackend = isTauri()
      ? async () => {
          await tauriInvoke("disconnect_connection", {
            payload: { connectionId },
          });
        }
      : undefined;
    try {
      await teardownConnectionWorkspace(state, connectionId, teardownBackend);
    } catch (error) {
      console.error("Failed to disconnect backend connection", error);
      return;
    }

    set((state) => {
      const { [connectionId]: _droppedTab, ...remainingTabs } =
        state.connectionOverviewTab;
      const {
        [connectionId]: _droppedSchemaMapSchema,
        ...remainingSchemaMapSchemas
      } = state.connectionSchemaMapSchema;
      return {
        connections: applyConnectionUpdate(state.connections, connectionId, {
          status: "Disconnected",
          latency: "--",
          errorMessage: undefined,
        }),
        connectionOverviewTab: remainingTabs,
        connectionSchemaMapSchema: remainingSchemaMapSchemas,
      };
    });
  },
});
