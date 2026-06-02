import type { StateCreator } from "zustand";

import { errorToMessage, isTauri, tauriInvoke } from "@/lib/tauri";

import { connectionReferencesBastion } from "./bastion-references";
import type {
  AppStoreState,
  BastionServer,
  BastionStatus,
  SaveBastionServerInput,
  SecretChange,
} from "./types";

type TestBastionResult = {
  latencyMs: number;
};

export type BastionsSlice = {
  bastionServers: BastionServer[];
  bastionStatus: BastionStatus;
  loadBastionServers: () => Promise<void>;
  saveBastionServer: (input: SaveBastionServerInput) => Promise<boolean>;
  deleteBastionServer: (bastionServerId: string) => Promise<boolean>;
  resetBastionHostKey: (bastionServerId: string) => Promise<boolean>;
  testBastionServer: (
    bastionServerId: string,
  ) => Promise<{ ok: true; latencyMs: number } | { ok: false; error: string }>;
};

export const createBastionsSlice: StateCreator<
  AppStoreState,
  [],
  [],
  BastionsSlice
> = (set, get) => ({
  bastionServers: [],
  bastionStatus: { state: "idle" },

  loadBastionServers: async () => {
    if (!isTauri()) {
      set({ bastionServers: [], bastionStatus: { state: "ready" } });
      return;
    }
    set({ bastionStatus: { state: "loading" } });
    try {
      const bastionServers = await tauriInvoke<BastionServer[]>(
        "load_bastion_servers",
      );
      set({ bastionServers, bastionStatus: { state: "ready" } });
    } catch (error) {
      const message = errorToMessage(error);
      set({ bastionStatus: { state: "error", error: message } });
    }
  },

  saveBastionServer: async (input) => {
    const referencedConnectionIds = get()
      .connections.filter((connection) =>
        connectionReferencesBastion(connection, input.id),
      )
      .map((connection) => connection.id);

    if (!isTauri()) {
      set((state) => ({
        bastionServers: upsertLocalBastion(state.bastionServers, input),
      }));
      return true;
    }
    try {
      const bastionServers = await tauriInvoke<BastionServer[]>(
        "save_bastion_server",
        { payload: input },
      );
      set((state) => ({
        bastionServers,
        connections: markReferencedConnectionsDisconnected(
          state.connections,
          referencedConnectionIds,
        ),
        bastionStatus: { state: "ready" },
      }));
      return true;
    } catch (error) {
      const message = errorToMessage(error);
      set({ bastionStatus: { state: "error", error: message } });
      return false;
    }
  },

  deleteBastionServer: async (bastionServerId) => {
    if (!isTauri()) {
      set((state) => ({
        bastionServers: state.bastionServers.filter(
          (server) => server.id !== bastionServerId,
        ),
      }));
      return true;
    }
    try {
      const bastionServers = await tauriInvoke<BastionServer[]>(
        "delete_bastion_server",
        { payload: { bastionServerId } },
      );
      set({ bastionServers, bastionStatus: { state: "ready" } });
      return true;
    } catch (error) {
      const message = errorToMessage(error);
      set({ bastionStatus: { state: "error", error: message } });
      return false;
    }
  },

  resetBastionHostKey: async (bastionServerId) => {
    const referencedConnectionIds = get()
      .connections.filter((connection) =>
        connectionReferencesBastion(connection, bastionServerId),
      )
      .map((connection) => connection.id);

    if (!isTauri()) {
      return true;
    }
    try {
      const bastionServers = await tauriInvoke<BastionServer[]>(
        "reset_bastion_host_key",
        { payload: { bastionServerId } },
      );
      set((state) => ({
        bastionServers,
        connections: markReferencedConnectionsDisconnected(
          state.connections,
          referencedConnectionIds,
        ),
        bastionStatus: { state: "ready" },
      }));
      return true;
    } catch (error) {
      const message = errorToMessage(error);
      set({ bastionStatus: { state: "error", error: message } });
      return false;
    }
  },

  testBastionServer: async (bastionServerId) => {
    if (!isTauri()) {
      return { ok: true, latencyMs: 0 };
    }
    try {
      const result = await tauriInvoke<TestBastionResult>(
        "test_bastion_server",
        { payload: { bastionServerId } },
      );
      await get().loadBastionServers();
      return { ok: true, latencyMs: result.latencyMs };
    } catch (error) {
      return { ok: false, error: errorToMessage(error) };
    }
  },
});

function upsertLocalBastion(
  bastions: BastionServer[],
  input: SaveBastionServerInput,
): BastionServer[] {
  const now = new Date().toISOString();
  const existing = bastions.find((server) => server.id === input.id);
  const next: BastionServer = {
    id: input.id,
    name: input.name,
    host: input.host,
    port: input.port,
    user: input.user,
    authMethod: input.authMethod,
    privateKeyPath: input.privateKeyPath,
    createdAt: existing?.createdAt ?? now,
    updatedAt: now,
    hasPassword:
      input.authMethod === "password"
        ? applySecretPresence(existing?.hasPassword ?? false, input.password)
        : false,
    hasPrivateKeyContent:
      input.authMethod === "privateKeyContent"
        ? applySecretPresence(
            existing?.hasPrivateKeyContent ?? false,
            input.privateKeyContent,
          )
        : false,
    hasPassphrase:
      input.authMethod === "password"
        ? false
        : applySecretPresence(
            existing?.hasPassphrase ?? false,
            input.passphrase,
          ),
  };
  const without = bastions.filter((server) => server.id !== input.id);
  return [...without, next].sort((a, b) => a.name.localeCompare(b.name));
}

function markReferencedConnectionsDisconnected<
  T extends AppStoreState["connections"][number],
>(connections: T[], connectionIds: string[]): T[] {
  if (connectionIds.length === 0) return connections;
  const ids = new Set(connectionIds);
  return connections.map((connection) =>
    ids.has(connection.id)
      ? ({
          ...connection,
          status: "Disconnected",
          latency: "--",
          errorMessage: "Bastion Server changed. Reconnect this Connection.",
        } as T)
      : connection,
  );
}

function applySecretPresence(existing: boolean, change: SecretChange): boolean {
  switch (change.action) {
    case "keep":
      return existing;
    case "set":
      return Boolean(change.value.trim());
    case "clear":
      return false;
  }
}
