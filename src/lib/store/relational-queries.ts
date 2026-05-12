/**
 * Relational Queries slice — owns Query History, Saved Queries,
 * per-tab query editor state. Relational-only (the query editor is a
 * relational-class workspace tab kind).
 *
 * Exposes `dropQueryStateForConnection(connectionId)` as its piece of
 * the delete-connection cleanup cascade.
 *
 * Phase: scaffold only.
 */

import type { StateCreator } from "zustand";

import type { AppStoreState } from "./types";

export type RelationalQueriesSlice = Record<string, never>;

export const createRelationalQueriesSlice: StateCreator<
  AppStoreState,
  [],
  [],
  RelationalQueriesSlice
> = () => ({});
