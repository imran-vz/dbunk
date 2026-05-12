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
import { errorToMessage, isTauri, tauriInvoke } from "@/lib/tauri";

import type {
  AppStoreState,
  Connection,
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
  lastSync: "Never",
});

/**
 * Strip the runtime fields (`status`, `latency`, `lastSync`,
 * `errorMessage`) from a `Connection` to recover the `StoredConnection`
 * wire shape. The variant-specific fields (`ssl`, `useHttps`, etc.)
 * pass through untouched via the spread — TypeScript narrows on
 * `engine` so the returned union member matches the input variant.
 */
const toStoredConnection = (connection: Connection): StoredConnection => {
  const { status, latency, lastSync, errorMessage, ...stored } = connection;
  // The runtime-field destructure leaves a shape whose engine tag plus
  // per-variant fields exactly satisfy `StoredConnection`; the cast is
  // narrowing the union the destructure widens.
  void status;
  void latency;
  void lastSync;
  void errorMessage;
  return stored as StoredConnection;
};

/**
 * Updates to a Connection's runtime fields (`status`, `latency`,
 * `lastSync`, `errorMessage`, `lastActivityAt`). Variant-specific
 * fields are excluded because they're part of the Connection's
 * engine identity, not transient state.
 */
type ConnectionRuntimeUpdate = Partial<
  Pick<
    Connection,
    "status" | "latency" | "lastSync" | "errorMessage" | "lastActivityAt"
  >
>;

const applyConnectionUpdate = (
  connections: Connection[],
  connectionId: string,
  updates: ConnectionRuntimeUpdate,
): Connection[] =>
  connections.map((connection) =>
    connection.id === connectionId
      ? // The runtime-fields spread preserves the original variant's
        // `engine` discriminator and engine-specific fields; the cast
        // is narrowing back to the union after TypeScript widens it
        // through the spread.
        ({ ...connection, ...updates } as Connection)
      : connection,
  );

export type ConnectionsSlice = {
  connections: Connection[];
  activeConnectionId: string;

  setActiveConnectionId: (id: string) => void;
  loadConnections: () => Promise<void>;
  addConnection: (connection: Connection) => Promise<void>;
  updateConnection: (connection: Connection) => Promise<void>;
  deleteConnection: (connectionId: string) => Promise<void>;
  connectConnection: (connectionId: string) => Promise<void>;
  disconnectConnection: (connectionId: string) => void;
  testConnection: (
    connection: StoredConnection,
  ) => Promise<
    | { ok: true; latencyMs: number; redisCapabilities?: RedisCapabilities }
    | { ok: false; error: string }
  >;
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

  setActiveConnectionId: (id) => set({ activeConnectionId: id }),

  loadConnections: async () => {
    if (!isTauri()) {
      return;
    }
    try {
      const stored = await tauriInvoke<StoredConnection[]>("load_connections");
      const connections = stored.map(hydrateConnection);
      set((state) => ({
        connections,
        activeConnectionId: connections.some(
          (connection) => connection.id === state.activeConnectionId,
        )
          ? state.activeConnectionId
          : (connections[0]?.id ?? ""),
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
                lastSync: currentConnection.lastSync,
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
    // Cascade cleanup is inline until the downstream slices land
    // their cleanup methods (commits 6–9). Once they exist, the
    // schemaExplorer-filter / activeConnectionId-update logic moves to
    // get().dropRelationalCachesForConnection(id) etc.
    if (!isTauri()) {
      set((state) => {
        const connections = state.connections.filter(
          (c) => c.id !== connectionId,
        );
        const newActiveId =
          state.activeConnectionId === connectionId
            ? (connections[0]?.id ?? "")
            : state.activeConnectionId;
        return {
          connections,
          activeConnectionId: newActiveId,
          schemaExplorer: Object.fromEntries(
            Object.entries(state.schemaExplorer).filter(
              ([key]) => key !== connectionId,
            ),
          ),
        };
      });
      return;
    }
    try {
      const stored = await tauriInvoke<StoredConnection[]>(
        "delete_connection",
        {
          payload: { connectionId },
        },
      );
      const connections = stored.map(hydrateConnection);
      set((state) => {
        const newActiveId =
          state.activeConnectionId === connectionId
            ? (connections[0]?.id ?? "")
            : state.activeConnectionId;
        return {
          connections,
          activeConnectionId: newActiveId,
          schemaExplorer: Object.fromEntries(
            Object.entries(state.schemaExplorer).filter(
              ([key]) => key !== connectionId,
            ),
          ),
        };
      });
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
            lastSync: new Date().toISOString(),
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

  connectConnection: async (connectionId) => {
    if (!connectionId) {
      return;
    }
    set({ activeConnectionId: connectionId });
    if (!isTauri()) {
      set((state) => ({
        connections: applyConnectionUpdate(state.connections, connectionId, {
          status: "Connected",
          lastSync: "Just now",
          errorMessage: undefined,
        }),
      }));
      return;
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
      const schema = isRelational
        ? await tauriInvoke<import("./types").SchemaExplorer[]>(
            "load_schema_explorer",
            { payload: { connectionId } },
          )
        : undefined;
      set((state) => ({
        connections: applyConnectionUpdate(state.connections, connectionId, {
          status: "Connected",
          latency: formatLatencyMs(result.latencyMs),
          lastSync: "Just now",
          lastActivityAt: new Date().toISOString(),
          errorMessage: undefined,
        }),
        schemaExplorer: schema
          ? { ...state.schemaExplorer, [connectionId]: schema }
          : state.schemaExplorer,
      }));
    } catch (error) {
      const message = errorToMessage(error);
      console.error("Failed to connect", error);
      set((state) => ({
        connections: applyConnectionUpdate(state.connections, connectionId, {
          status: "Disconnected",
          errorMessage: message,
        }),
      }));
    }
  },

  disconnectConnection: (connectionId) => {
    if (!connectionId) {
      return;
    }
    const state = get();
    if (
      !state.connections.some((connection) => connection.id === connectionId)
    ) {
      return;
    }

    state.dropOpenQueryStateForConnection(connectionId);
    state.dropRelationalCachesForConnection(connectionId);
    state.closeKeyTabsForConnection(connectionId);
    state.closePubSubSessionsForConnection(connectionId);
    state.closeTabsForConnection(connectionId);

    set((state) => ({
      connections: applyConnectionUpdate(state.connections, connectionId, {
        status: "Disconnected",
        latency: "--",
        lastSync: "Never",
        errorMessage: undefined,
      }),
    }));
  },
});
