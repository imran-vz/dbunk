import { render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("@/lib/tauri", () => ({
  isTauri: vi.fn(() => true),
  tauriInvoke: vi.fn(),
  errorToMessage: (error: unknown) =>
    error instanceof Error ? error.message : String(error),
}));

import { ConnectionsView } from "@/components/connections-view";
import { type Connection, useAppStore } from "@/lib/store";

const baseConnection: Connection = {
  id: "conn-1",
  name: "Local Postgres",
  database: "postgres",
  status: "Disconnected",
  engine: "PostgreSQL",
  host: "localhost",
  port: 5432,
  user: "postgres",
  password: "",
  role: "admin",
  latency: "--",
  lastSync: "Never",
};

const initialStoreState = useAppStore.getState();

beforeEach(() => {
  useAppStore.setState(initialStoreState, true);
});

afterEach(() => {
  useAppStore.setState(initialStoreState, true);
});

describe("ConnectionsView error feedback", () => {
  it("renders the connection's errorMessage inline", () => {
    useAppStore.setState({
      connections: [
        {
          ...baseConnection,
          errorMessage: "password authentication failed for user postgres",
        },
      ],
      activeConnectionId: baseConnection.id,
    });

    render(<ConnectionsView />);

    expect(screen.getByRole("alert").textContent).toContain(
      "password authentication failed for user postgres",
    );
  });

  it("does not render an alert when no errorMessage is set", () => {
    useAppStore.setState({
      connections: [baseConnection],
      activeConnectionId: baseConnection.id,
    });

    render(<ConnectionsView />);

    expect(screen.queryByRole("alert")).toBeNull();
  });
});
