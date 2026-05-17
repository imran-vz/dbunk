/**
 * KeyValue Workspace slice — client-side cache of Keyspace Browser /
 * Key Inspector session state. Keyvalue-only (Redis) — see ADR-0008.
 *
 * Today it owns:
 * - `redisCapabilitiesByConnection`: the connect-time capability
 *   probe (server version, role, connected_slaves, modules, etc.).
 *   Populated by `Connections.connectConnection` and cleared by
 *   `closeKeyTabsForConnection`. Drives surfaces like the keyspace
 *   browser's replica-warning that need to know "is this a master
 *   with replicas?" without paying for a fresh `INFO replication`
 *   per render.
 *
 * Reserved for the deferred watched-keys feature and the per-session
 * DB switcher.
 *
 * Exposes `closeKeyTabsForConnection(connectionId)` as its piece of
 * the delete-connection cleanup cascade (called from
 * `Connections.deleteConnection`). Key tabs themselves are closed by
 * the Workspace Tabs slice's `closeTabsForConnection` because they
 * live in the shared `workspaceTabs` list — this method clears the
 * keyvalue-only per-connection state that lives here.
 */

import type { StateCreator } from "zustand";

import type { AppStoreState, RedisCapabilities } from "./types";

export type KeyValueWorkspaceSlice = {
  /**
   * Connect-time Redis capability probe (`INFO`, `MODULE LIST`,
   * `DBSIZE`, `CONFIG GET maxmemory-policy`) keyed by `connection.id`.
   * Refreshed whenever `connectConnection` succeeds; cleared by
   * `closeKeyTabsForConnection` on disconnect or delete.
   */
  redisCapabilitiesByConnection: Record<string, RedisCapabilities>;

  setRedisCapabilities: (
    connectionId: string,
    capabilities: RedisCapabilities,
  ) => void;

  /**
   * Cascade cleanup — drops every keyvalue-owned per-connection
   * cache entry. Called by `deleteConnection` /
   * `disconnectConnection`.
   */
  closeKeyTabsForConnection: (connectionId: string) => void;
};

export const createKeyValueWorkspaceSlice: StateCreator<
  AppStoreState,
  [],
  [],
  KeyValueWorkspaceSlice
> = (set) => ({
  redisCapabilitiesByConnection: {},

  setRedisCapabilities: (connectionId, capabilities) =>
    set((state) => ({
      redisCapabilitiesByConnection: {
        ...state.redisCapabilitiesByConnection,
        [connectionId]: capabilities,
      },
    })),

  closeKeyTabsForConnection: (connectionId) =>
    set((state) => {
      if (!(connectionId in state.redisCapabilitiesByConnection)) {
        return {};
      }
      const { [connectionId]: _dropped, ...rest } =
        state.redisCapabilitiesByConnection;
      return { redisCapabilitiesByConnection: rest };
    }),
});
