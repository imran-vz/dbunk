// @vitest-environment jsdom
import {
  act,
  cleanup,
  fireEvent,
  render,
  screen,
  within,
} from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("@/lib/tauri", () => ({
  isTauri: vi.fn(() => false),
  tauriInvoke: vi.fn(),
  errorToMessage: (error: unknown) =>
    error instanceof Error ? error.message : String(error),
}));

import {
  analyzePrivateKeyContent,
  BastionServersTab,
} from "@/components/bastion-servers-tab";
import { type BastionServer, type Connection, useAppStore } from "@/lib/store";

const initialStoreState = useAppStore.getState();

function bastion(overrides: Partial<BastionServer>): BastionServer {
  return {
    id: "bastion-1",
    name: "Production Bastion",
    host: "bastion.prod",
    port: 22,
    user: "ubuntu",
    authMethod: "privateKeyPath",
    privateKeyPath: "/Users/me/.ssh/id_ed25519",
    hostKeyFingerprint: "SHA256:abc",
    createdAt: "2026-06-02T00:00:00Z",
    updatedAt: "2026-06-02T00:00:00Z",
    hasPassword: false,
    hasPrivateKeyContent: false,
    hasPassphrase: false,
    ...overrides,
  };
}

const pgConnection: Connection = {
  id: "conn-1",
  name: "Primary PG",
  engine: "PostgreSQL",
  host: "db.internal",
  port: 5432,
  database: "postgres",
  user: "postgres",
  password: "",
  role: "read/write",
  ssl: true,
  sshTunnel: {
    enabled: true,
    bastionServerId: "bastion-1",
    jumpChain: ["bastion-2"],
  },
  status: "Disconnected",
  latency: "--",
};

beforeEach(() => {
  useAppStore.setState(initialStoreState, true);
  useAppStore.setState({
    bastionStatus: { state: "ready" },
    bastionServers: [
      bastion({ id: "bastion-1", name: "Production Bastion" }),
      bastion({
        id: "bastion-2",
        name: "Staging Gateway",
        host: "gateway.staging",
        user: "deploy",
        hostKeyFingerprint: undefined,
      }),
    ],
    connections: [pgConnection],
  });
});

afterEach(() => {
  cleanup();
  useAppStore.setState(initialStoreState, true);
  vi.clearAllMocks();
});

describe("BastionServersTab", () => {
  it("filters Bastion Servers by search text", () => {
    render(<BastionServersTab />);

    fireEvent.change(screen.getByLabelText("Search Bastion Servers"), {
      target: { value: "prod" },
    });

    expect(screen.getByText("Production Bastion")).toBeTruthy();
    expect(screen.queryByText("Staging Gateway")).toBeNull();
    expect(screen.getByText("Showing 1 of 2.")).toBeTruthy();
  });

  it("requires typing the host before resetting trusted host-key state", async () => {
    const resetBastionHostKey = vi.fn(async () => true);
    useAppStore.setState({ resetBastionHostKey });

    render(<BastionServersTab />);

    fireEvent.click(screen.getAllByRole("button", { name: /Reset trust/ })[0]);
    const resetPanel = screen.getByRole("group", {
      name: "Confirm host-key reset for Production Bastion",
    });
    const confirm = within(resetPanel).getByRole("button", {
      name: "Reset trust",
    });
    expect(confirm.hasAttribute("disabled")).toBe(true);

    fireEvent.change(
      screen.getByLabelText("Type bastion.prod to confirm host-key reset"),
      { target: { value: "bastion.prod" } },
    );
    expect(confirm.hasAttribute("disabled")).toBe(false);

    await act(async () => {
      fireEvent.click(confirm);
    });

    expect(resetBastionHostKey).toHaveBeenCalledWith("bastion-1");
  });

  it("shows guided private-key capture feedback", () => {
    render(<BastionServersTab />);

    fireEvent.click(screen.getByRole("button", { name: /Capture key/ }));
    fireEvent.change(screen.getByLabelText("Private key content"), {
      target: { value: "not a key" },
    });

    expect(
      screen.getByText("Missing matching private-key header and footer."),
    ).toBeTruthy();
  });
});

describe("analyzePrivateKeyContent", () => {
  it("detects a private key header and footer", () => {
    expect(
      analyzePrivateKeyContent(
        "-----BEGIN OPENSSH PRIVATE KEY-----\nbody\n-----END OPENSSH PRIVATE KEY-----",
      ),
    ).toEqual({ tone: "ready", message: "Private key format detected." });
  });
});
