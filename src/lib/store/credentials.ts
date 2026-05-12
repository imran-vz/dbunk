/**
 * Credentials slice — owns App Settings, Credential Storage Mode,
 * the credential storage lifecycle (configure/unlock/change/reset).
 *
 * No per-connection cleanup cascade — credentials are app-wide. The
 * `resetCredentialStorage` action does clear the Connection list (a
 * Connections-slice cross-cut) because resetting credentials
 * invalidates every stored password; that cross-cut goes through
 * `get()` once the Connections slice exists.
 */

import type { StateCreator } from "zustand";

import { errorToMessage, isTauri, tauriInvoke } from "@/lib/tauri";

import type {
  AppSettingsSnapshot,
  AppSettingsStatus,
  AppStoreState,
  CredentialStorageMode,
} from "./types";

export type CredentialsSlice = {
  appSettings: AppSettingsSnapshot | null;
  appSettingsStatus: AppSettingsStatus;
  credentialStorageStatus:
    | { state: "idle" }
    | { state: "running" }
    | { state: "error"; error: string };

  loadAppSettings: () => Promise<AppSettingsSnapshot | null>;
  configureCredentialStorage: (input: {
    mode: CredentialStorageMode;
    password?: string;
  }) => Promise<AppSettingsSnapshot | null>;
  unlockCredentials: (password: string) => Promise<AppSettingsSnapshot | null>;
  changeCredentialStorage: (input: {
    mode: CredentialStorageMode;
    password?: string;
  }) => Promise<AppSettingsSnapshot | null>;
  resetCredentialStorage: () => Promise<AppSettingsSnapshot | null>;
};

export const createCredentialsSlice: StateCreator<
  AppStoreState,
  [],
  [],
  CredentialsSlice
> = (set, get) => ({
  appSettings: null,
  appSettingsStatus: { state: "idle" },
  credentialStorageStatus: { state: "idle" },

  loadAppSettings: async () => {
    if (!isTauri()) {
      const fallback: AppSettingsSnapshot = {
        onboardingCompleted: true,
        credentialStorageMode: "plain-sqlite",
        credentialState: "ready",
        configDir: "~/.config/dbunk",
      };
      set({ appSettings: fallback, appSettingsStatus: { state: "ready" } });
      return fallback;
    }
    set({ appSettingsStatus: { state: "loading" } });
    try {
      const snapshot =
        await tauriInvoke<AppSettingsSnapshot>("load_app_settings");
      set({ appSettings: snapshot, appSettingsStatus: { state: "ready" } });
      return snapshot;
    } catch (error) {
      const message = errorToMessage(error);
      console.error("Failed to load app settings", error);
      set({ appSettingsStatus: { state: "error", error: message } });
      return null;
    }
  },

  configureCredentialStorage: async (input) => {
    if (!isTauri()) {
      const snapshot: AppSettingsSnapshot = {
        onboardingCompleted: true,
        credentialStorageMode: input.mode,
        credentialState: "ready",
        configDir: "~/.config/dbunk",
      };
      set({ appSettings: snapshot, appSettingsStatus: { state: "ready" } });
      return snapshot;
    }
    set({ credentialStorageStatus: { state: "running" } });
    try {
      const snapshot = await tauriInvoke<AppSettingsSnapshot>(
        "configure_credential_storage",
        { payload: input },
      );
      set({
        appSettings: snapshot,
        appSettingsStatus: { state: "ready" },
        credentialStorageStatus: { state: "idle" },
      });
      return snapshot;
    } catch (error) {
      const message = errorToMessage(error);
      set({ credentialStorageStatus: { state: "error", error: message } });
      return null;
    }
  },

  unlockCredentials: async (password) => {
    if (!isTauri()) {
      return get().loadAppSettings();
    }
    set({ credentialStorageStatus: { state: "running" } });
    try {
      const snapshot = await tauriInvoke<AppSettingsSnapshot>(
        "unlock_credentials",
        { payload: { password } },
      );
      set({
        appSettings: snapshot,
        appSettingsStatus: { state: "ready" },
        credentialStorageStatus: { state: "idle" },
      });
      return snapshot;
    } catch (error) {
      const message = errorToMessage(error);
      set({ credentialStorageStatus: { state: "error", error: message } });
      return null;
    }
  },

  changeCredentialStorage: async (input) => {
    if (!isTauri()) {
      const snapshot: AppSettingsSnapshot = {
        onboardingCompleted: true,
        credentialStorageMode: input.mode,
        credentialState: "ready",
        configDir: "~/.config/dbunk",
      };
      set({ appSettings: snapshot });
      return snapshot;
    }
    set({ credentialStorageStatus: { state: "running" } });
    try {
      const snapshot = await tauriInvoke<AppSettingsSnapshot>(
        "change_credential_storage",
        { payload: { ...input, confirm: true } },
      );
      set({
        appSettings: snapshot,
        credentialStorageStatus: { state: "idle" },
      });
      // Cross-slice: trigger a Connections reload so the password
      // hydration reflects the new storage backend.
      await get().loadConnections();
      return snapshot;
    } catch (error) {
      const message = errorToMessage(error);
      set({ credentialStorageStatus: { state: "error", error: message } });
      return null;
    }
  },

  resetCredentialStorage: async () => {
    if (!isTauri()) {
      return get().loadAppSettings();
    }
    set({ credentialStorageStatus: { state: "running" } });
    try {
      const snapshot = await tauriInvoke<AppSettingsSnapshot>(
        "reset_credential_storage",
      );
      // Cross-slice: a credential reset implicitly invalidates every
      // stored Connection password. Clearing the Connection list +
      // active-connection ID is a Connections-slice concern; we reach
      // across via `set` here because `connections`/`activeConnectionId`
      // are part of the full AppStoreState shape.
      set({
        appSettings: snapshot,
        connections: [],
        activeConnectionId: "",
        credentialStorageStatus: { state: "idle" },
      });
      return snapshot;
    } catch (error) {
      const message = errorToMessage(error);
      set({ credentialStorageStatus: { state: "error", error: message } });
      return null;
    }
  },
});
