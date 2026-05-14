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
import {
  applyTheme,
  isPresetIntrinsicallyDark,
  type ThemeMode,
  type ThemePreset,
  writeStoredMode,
  writeStoredPreset,
} from "@/lib/theme";

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
  /**
   * Persist the user's theme mode. Updates the boot cache + DOM class
   * synchronously; the SQLite write is fire-and-forget so the menu
   * stays snappy. AppSettings is updated optimistically.
   */
  setTheme: (mode: ThemeMode) => Promise<void>;
  /**
   * Persist the user's theme preset. Same write semantics as
   * `setTheme` — DOM + cache first, SQLite after.
   */
  setThemePreset: (preset: ThemePreset) => Promise<void>;
};

function hydrateTheme(snapshot: AppSettingsSnapshot | null) {
  const mode: ThemeMode = snapshot?.theme ?? "system";
  const preset: ThemePreset = snapshot?.themePreset ?? "default";
  applyTheme(mode, preset);
  writeStoredMode(mode);
  writeStoredPreset(preset);
}

type ThemePatch = { mode?: ThemeMode; preset?: ThemePreset };

async function persistThemePatch(
  get: () => AppStoreState,
  set: (partial: Partial<AppStoreState>) => void,
  patch: ThemePatch,
) {
  const current = get().appSettings;
  let mode = patch.mode ?? current?.theme ?? "system";
  const preset = patch.preset ?? current?.themePreset ?? "default";

  // Selecting an intrinsically-dark preset (Dracula) while mode is
  // light/system should advance the stored mode to "dark" so the radio
  // reflects what's painted and, on switch back to a non-dark preset,
  // the user lands in dark mode instead of snapping back to light.
  const coerceMode =
    isPresetIntrinsicallyDark(preset) && mode !== "dark" ? "dark" : null;
  if (coerceMode) mode = coerceMode;

  applyTheme(mode, preset);
  if (patch.mode !== undefined || coerceMode) writeStoredMode(mode);
  if (patch.preset !== undefined) writeStoredPreset(patch.preset);
  if (current) {
    set({
      appSettings: {
        ...current,
        ...(patch.mode !== undefined || coerceMode ? { theme: mode } : null),
        ...(patch.preset !== undefined ? { themePreset: patch.preset } : null),
      },
    });
  }
  if (!isTauri()) return;
  try {
    await tauriInvoke<AppSettingsSnapshot>("save_app_settings", {
      payload: {
        ...(patch.mode !== undefined || coerceMode ? { theme: mode } : null),
        ...(patch.preset !== undefined ? { themePreset: patch.preset } : null),
      },
    });
  } catch (error) {
    console.error("Failed to persist theme settings", error);
  }
}

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
      hydrateTheme(fallback);
      return fallback;
    }
    set({ appSettingsStatus: { state: "loading" } });
    try {
      const snapshot =
        await tauriInvoke<AppSettingsSnapshot>("load_app_settings");
      set({ appSettings: snapshot, appSettingsStatus: { state: "ready" } });
      hydrateTheme(snapshot);
      return snapshot;
    } catch (error) {
      const message = errorToMessage(error);
      console.error("Failed to load app settings", error);
      set({ appSettingsStatus: { state: "error", error: message } });
      return null;
    }
  },

  setTheme: (mode) => persistThemePatch(get, set, { mode }),
  setThemePreset: (preset) => persistThemePatch(get, set, { preset }),

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
