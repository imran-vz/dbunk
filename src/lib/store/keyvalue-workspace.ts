/**
 * KeyValue Workspace slice — owns Keyspace Browser cache and Key
 * Inspector state. Keyvalue-only (Redis) — see ADR-0008.
 *
 * Today this slice holds no state: the Keyspace Browser component
 * keeps its scan cursor + filter chips in component state, and the
 * Key Inspector tab also stores its per-key fetch in component state.
 * The slice exists for cascade-contract clarity — when the deferred
 * watched-keys feature or the DB switcher lands, this is where they
 * go.
 *
 * Exposes `closeKeyTabsForConnection(connectionId)` as its piece of
 * the delete-connection cleanup cascade.
 *
 * Phase: scaffold only.
 */

import type { StateCreator } from "zustand";

import type { AppStoreState } from "./types";

export type KeyValueWorkspaceSlice = Record<string, never>;

export const createKeyValueWorkspaceSlice: StateCreator<
  AppStoreState,
  [],
  [],
  KeyValueWorkspaceSlice
> = () => ({});
