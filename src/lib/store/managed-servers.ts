import type { StateCreator } from "zustand";

import { errorToMessage, isTauri, tauriInvoke } from "@/lib/tauri";

import type {
  AppStoreState,
  ManagedServersStatus,
  ManagedServerWithStatus,
  ProvisionManagedServerInput,
  ProvisionManagedServerResult,
} from "./types";

export type ManagedServersSlice = {
  managedServers: ManagedServerWithStatus[];
  managedServersStatus: ManagedServersStatus;
  loadManagedServers: () => Promise<void>;
  provisionManagedServer: (
    input: ProvisionManagedServerInput,
  ) => Promise<
    | { ok: true; result: ProvisionManagedServerResult }
    | { ok: false; error: string }
  >;
  startManagedServer: (id: string) => Promise<string | null>;
  stopManagedServer: (id: string) => Promise<string | null>;
  destroyManagedServer: (id: string) => Promise<string | null>;
  recreateManagedServer: (id: string) => Promise<string | null>;
};

export const createManagedServersSlice: StateCreator<
  AppStoreState,
  [],
  [],
  ManagedServersSlice
> = (set, get) => {
  /** Run a lifecycle command, then refresh the list; returns an error
   *  message or null on success. */
  const lifecycle = async (
    command: string,
    managedServerId: string,
  ): Promise<string | null> => {
    if (!isTauri()) return "Managed servers require the desktop runtime.";
    try {
      await tauriInvoke(command, { payload: { managedServerId } });
      return null;
    } catch (error) {
      return errorToMessage(error);
    } finally {
      await get().loadManagedServers();
    }
  };

  return {
    managedServers: [],
    managedServersStatus: { state: "idle" },

    loadManagedServers: async () => {
      if (!isTauri()) {
        set({ managedServers: [], managedServersStatus: { state: "ready" } });
        return;
      }
      set({ managedServersStatus: { state: "loading" } });
      try {
        const managedServers = await tauriInvoke<ManagedServerWithStatus[]>(
          "list_managed_servers",
        );
        set({ managedServers, managedServersStatus: { state: "ready" } });
      } catch (error) {
        set({
          managedServersStatus: {
            state: "error",
            error: errorToMessage(error),
          },
        });
      }
    },

    provisionManagedServer: async (input) => {
      if (!isTauri()) {
        return {
          ok: false as const,
          error: "Managed servers require the desktop runtime.",
        };
      }
      try {
        const result = await tauriInvoke<ProvisionManagedServerResult>(
          "provision_managed_server",
          { payload: input },
        );
        // The backend persisted both the managed server and its
        // Connection; refresh both lists so the sidebar shows the new
        // connection with its managed badge immediately.
        await Promise.all([
          get().loadManagedServers(),
          get().loadConnections(),
        ]);
        return { ok: true as const, result };
      } catch (error) {
        return { ok: false as const, error: errorToMessage(error) };
      }
    },

    startManagedServer: (id) => {
      set((state) => ({
        managedServers: state.managedServers.map((server) =>
          server.id === id ? { ...server, status: "starting" } : server,
        ),
      }));
      return lifecycle("start_managed_server", id);
    },
    stopManagedServer: (id) => lifecycle("stop_managed_server", id),
    destroyManagedServer: (id) => lifecycle("destroy_managed_server", id),
    recreateManagedServer: (id) => lifecycle("recreate_managed_server", id),
  };
};
