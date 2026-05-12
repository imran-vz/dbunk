/**
 * KeyValue Workspace slice — placeholder for client-side Keyspace
 * Browser + Key Inspector caches. Keyvalue-only (Redis) — see
 * ADR-0008.
 *
 * Today this slice holds no state: the Keyspace Browser keeps its
 * scan cursor + filter chips in component state, and the Key
 * Inspector tab stores its per-key fetch in component state. The
 * slice exists for cascade-contract clarity — when the deferred
 * watched-keys feature or the per-session DB switcher lands, this is
 * where they'll live.
 *
 * Exposes `closeKeyTabsForConnection(connectionId)` as its piece of
 * the delete-connection cleanup cascade. Today the
 * Workspace Tabs slice already cleans key tabs via
 * `closeTabsForConnection` (key tabs live in the shared workspaceTabs
 * list), so this method is a documented no-op. Future per-key cache
 * state in this slice would clean up here.
 */

import type { StateCreator } from "zustand";

import type { AppStoreState } from "./types";

export type KeyValueWorkspaceSlice = {
  /**
   * Cascade cleanup — see slice doc. Currently a no-op.
   */
  closeKeyTabsForConnection: (connectionId: string) => void;
};

export const createKeyValueWorkspaceSlice: StateCreator<
  AppStoreState,
  [],
  [],
  KeyValueWorkspaceSlice
> = () => ({
  closeKeyTabsForConnection: (_connectionId: string) => {
    // No state to clean today. Reserved for future per-key cache.
  },
});
