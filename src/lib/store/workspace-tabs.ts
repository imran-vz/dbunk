/**
 * Workspace Tabs slice — owns the Workspace Tab list, active tab ID,
 * active view, sidebar/editor UI flags. Exposes
 * `closeTabsForConnection(connectionId)` as its piece of the
 * delete-connection cleanup cascade (called by the Connections slice).
 *
 * Phase: scaffold only.
 */

import type { StateCreator } from "zustand";

import type { AppStoreState } from "./types";

export type WorkspaceTabsSlice = Record<string, never>;

export const createWorkspaceTabsSlice: StateCreator<
  AppStoreState,
  [],
  [],
  WorkspaceTabsSlice
> = () => ({});
