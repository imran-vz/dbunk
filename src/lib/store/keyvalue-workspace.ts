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

import type { RedisAclSelf } from "@/lib/redis/api";

import type { AppStoreState, RedisCapabilities } from "./types";

export type KeyValueWorkspaceSlice = {
  /**
   * Connect-time Redis capability probe (`INFO`, `MODULE LIST`,
   * `DBSIZE`, `CONFIG GET maxmemory-policy`) keyed by `connection.id`.
   * Refreshed whenever `connectConnection` succeeds; cleared by
   * `closeKeyTabsForConnection` on disconnect or delete.
   */
  redisCapabilitiesByConnection: Record<string, RedisCapabilities>;

  /**
   * ACL surface for the current user on each Redis connection —
   * username + key-pattern restrictions. Populated by
   * `connectConnection` via `fetch_acl_self`. Drives the "this
   * connection can only see keys matching <patterns>" hint in the
   * keyspace browser.
   */
  redisAclSelfByConnection: Record<string, RedisAclSelf>;

  setRedisCapabilities: (
    connectionId: string,
    capabilities: RedisCapabilities,
  ) => void;

  setRedisAclSelf: (connectionId: string, acl: RedisAclSelf) => void;

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
  redisAclSelfByConnection: {},

  setRedisCapabilities: (connectionId, capabilities) =>
    set((state) => ({
      redisCapabilitiesByConnection: {
        ...state.redisCapabilitiesByConnection,
        [connectionId]: capabilities,
      },
    })),

  setRedisAclSelf: (connectionId, acl) =>
    set((state) => ({
      redisAclSelfByConnection: {
        ...state.redisAclSelfByConnection,
        [connectionId]: acl,
      },
    })),

  closeKeyTabsForConnection: (connectionId) =>
    set((state) => {
      const hasCaps = connectionId in state.redisCapabilitiesByConnection;
      const hasAcl = connectionId in state.redisAclSelfByConnection;
      if (!hasCaps && !hasAcl) {
        return {};
      }
      const next: Partial<KeyValueWorkspaceSlice> = {};
      if (hasCaps) {
        const { [connectionId]: _, ...rest } =
          state.redisCapabilitiesByConnection;
        next.redisCapabilitiesByConnection = rest;
      }
      if (hasAcl) {
        const { [connectionId]: _, ...rest } = state.redisAclSelfByConnection;
        next.redisAclSelfByConnection = rest;
      }
      return next;
    }),
});
