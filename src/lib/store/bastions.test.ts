/* oxlint-disable anti-slop/no-module-mocking anti-slop/no-unknown-parameters -- Test fixtures use controlled mocks and assertions to exercise otherwise inaccessible boundaries. */
import { act } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { pgToolClient } from "@/lib/pg-tool-jobs/client";

vi.mock("@/lib/tauri", () => ({
  isTauri: vi.fn(() => true),
  tauriInvoke: vi.fn(),
  errorToMessage: (error: unknown) =>
    error instanceof Error ? error.message : String(error),
}));

import {
  type BastionServer,
  type Connection,
  type SaveBastionServerInput,
  useAppStore,
} from "@/lib/store";
import { isTauri, tauriInvoke } from "@/lib/tauri";

const mockedInvoke = vi.mocked(tauriInvoke);
const mockedIsTauri = vi.mocked(isTauri);
const initialStoreState = useAppStore.getState();

beforeEach(() => {
  vi.spyOn(pgToolClient, "list").mockResolvedValue([]);
  mockedIsTauri.mockReturnValue(true);
  mockedInvoke.mockReset();
  useAppStore.setState(initialStoreState, true);
});

afterEach(() => {
  useAppStore.setState(initialStoreState, true);
});

describe("Bastion store slice", () => {
  it("disconnects Connections that reference an edited Bastion Server through the jump chain", async () => {
    const changedBastion = bastion("jump-1");
    mockedInvoke.mockResolvedValueOnce([changedBastion]);
    useAppStore.setState({
      connections: [
        connectedPostgres("conn-1", {
          bastionServerId: "final",
          jumpChain: [" jump-1 "],
        }),
        connectedPostgres("conn-2", {
          bastionServerId: "other",
          jumpChain: [],
        }),
      ],
    });

    await act(async () => {
      await useAppStore.getState().saveBastionServer(saveInput("jump-1"));
    });

    expect(mockedInvoke).toHaveBeenCalledWith("save_bastion_server", {
      payload: saveInput("jump-1"),
    });
    expect(statusFor("conn-1")).toBe("Disconnected");
    expect(errorFor("conn-1")).toBe(
      "Bastion Server changed. Reconnect this Connection.",
    );
    expect(statusFor("conn-2")).toBe("Connected");
    expect(errorFor("conn-2")).toBeUndefined();
  });

  it("disconnects Connections that reference a host-key-reset Bastion Server through the jump chain", async () => {
    mockedInvoke.mockResolvedValueOnce([bastion("jump-1")]);
    useAppStore.setState({
      connections: [
        connectedPostgres("conn-1", {
          bastionServerId: "final",
          jumpChain: [" jump-1 "],
        }),
        connectedPostgres("conn-2", {
          bastionServerId: "other",
          jumpChain: [],
        }),
      ],
    });

    await act(async () => {
      await useAppStore.getState().resetBastionHostKey("jump-1");
    });

    expect(mockedInvoke).toHaveBeenCalledWith("reset_bastion_host_key", {
      payload: { bastionServerId: "jump-1" },
    });
    expect(statusFor("conn-1")).toBe("Disconnected");
    expect(errorFor("conn-1")).toBe(
      "Bastion Server changed. Reconnect this Connection.",
    );
    expect(statusFor("conn-2")).toBe("Connected");
    expect(errorFor("conn-2")).toBeUndefined();
  });
});

function connectedPostgres(
  id: string,
  tunnel: Omit<
    NonNullable<Extract<Connection, { engine: "PostgreSQL" }>["sshTunnel"]>,
    "enabled"
  >,
): Extract<Connection, { engine: "PostgreSQL" }> {
  return {
    id,
    name: id,
    engine: "PostgreSQL",
    host: "db.internal",
    port: 5432,
    database: "postgres",
    user: "postgres",
    password: "",
    role: "read/write",
    ssl: true,
    sshTunnel: { enabled: true, ...tunnel },
    status: "Connected",
    latency: "5ms",
  };
}

function bastion(id: string): BastionServer {
  return {
    id,
    name: id,
    host: "bastion.internal",
    port: 22,
    user: "ubuntu",
    authMethod: "privateKeyContent",
    createdAt: "2026-01-01T00:00:00.000Z",
    updatedAt: "2026-01-01T00:00:00.000Z",
    hasPassword: false,
    hasPrivateKeyContent: true,
    hasPassphrase: false,
  };
}

function saveInput(id: string): SaveBastionServerInput {
  return {
    id,
    name: id,
    host: "bastion.internal",
    port: 22,
    user: "ubuntu",
    authMethod: "privateKeyContent",
    password: { action: "keep" },
    privateKeyContent: { action: "keep" },
    passphrase: { action: "keep" },
  };
}

function statusFor(connectionId: string): Connection["status"] | undefined {
  return useAppStore
    .getState()
    .connections.find((connection) => connection.id === connectionId)?.status;
}

function errorFor(connectionId: string): string | undefined {
  return useAppStore
    .getState()
    .connections.find((connection) => connection.id === connectionId)
    ?.errorMessage;
}
