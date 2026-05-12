/**
 * Credentials slice — owns App Settings, Credential Storage Mode,
 * the credential storage lifecycle (configure/unlock/change/reset).
 * No per-connection cleanup cascade — credentials are app-wide.
 *
 * Phase: scaffold only.
 */

import type { StateCreator } from "zustand";

import type { AppStoreState } from "./types";

export type CredentialsSlice = Record<string, never>;

export const createCredentialsSlice: StateCreator<
  AppStoreState,
  [],
  [],
  CredentialsSlice
> = () => ({});
