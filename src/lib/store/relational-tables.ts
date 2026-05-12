/**
 * Relational Tables slice — owns Schema Explorer, Schema Relationships,
 * Table Structure, Cell Edits, DDL pending changes, Database Overview
 * Stats, Table Data + previews. Relational-only — see ADR-0008.
 *
 * Exposes `dropRelationalCachesForConnection(connectionId)` as its
 * piece of the delete-connection cleanup cascade.
 *
 * Phase: scaffold only.
 */

import type { StateCreator } from "zustand";

import type { AppStoreState } from "./types";

export type RelationalTablesSlice = Record<string, never>;

export const createRelationalTablesSlice: StateCreator<
  AppStoreState,
  [],
  [],
  RelationalTablesSlice
> = () => ({});
