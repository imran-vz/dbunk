/**
 * Connections slice — owns Connection records, Active Connection ID,
 * Health Check status, Last Activity. Also the entity-owner for the
 * cross-slice cleanup cascade triggered by `deleteConnection` (see
 * `store/README.md`).
 *
 * Phase: scaffold only. Real state and actions move in the next
 * commit. The empty `ConnectionsSlice` type and `createConnectionsSlice`
 * factory exist so the `AppStoreState` union in `./types.ts` resolves.
 */

import type { StateCreator } from "zustand";

import type { AppStoreState } from "./types";

export type ConnectionsSlice = Record<string, never>;

export const createConnectionsSlice: StateCreator<
  AppStoreState,
  [],
  [],
  ConnectionsSlice
> = () => ({});
