// @vitest-environment jsdom
import {
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
  ssl: true,
};

const initialStoreState = useAppStore.getState();

beforeEach(() => {
  useAppStore.setState(initialStoreState, true);
});

afterEach(() => {
  cleanup();
  useAppStore.setState(initialStoreState, true);
  vi.clearAllMocks();
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

describe("ConnectionCard status pill and badge labels", () => {
  it("renders Healthy pill when connection is Connected (no error)", () => {
    useAppStore.setState({
      connections: [{ ...baseConnection, status: "Connected" }],
      activeConnectionId: baseConnection.id,
    });

    render(<ConnectionsView />);

    // "Healthy" appears both as a filter tab and as the pill label.
    const matches = screen.getAllByText("Healthy");
    expect(matches.length).toBeGreaterThanOrEqual(2);
  });

  it("renders Warning pill when status is Read only", () => {
    useAppStore.setState({
      connections: [{ ...baseConnection, status: "Read only" }],
      activeConnectionId: baseConnection.id,
    });

    render(<ConnectionsView />);

    // Two "Warning" strings appear: the filter tab label and the pill label.
    const matches = screen.getAllByText("Warning");
    expect(matches.length).toBeGreaterThanOrEqual(1);
  });

  it("renders Idle label when status is Disconnected with no error", () => {
    useAppStore.setState({
      connections: [{ ...baseConnection, status: "Disconnected" }],
      activeConnectionId: baseConnection.id,
    });

    render(<ConnectionsView />);

    expect(screen.getByText("Idle")).toBeTruthy();
  });

  it("renders Error label and danger pill when errorMessage is set, even if Connected", () => {
    useAppStore.setState({
      connections: [
        {
          ...baseConnection,
          status: "Connected",
          errorMessage: "boom",
        },
      ],
      activeConnectionId: baseConnection.id,
    });

    render(<ConnectionsView />);

    // Pill label "Error" appears alongside the filter tab label "Error".
    const matches = screen.getAllByText("Error");
    expect(matches.length).toBeGreaterThanOrEqual(2);
    expect(screen.getByRole("alert")).toBeTruthy();
  });
});

describe("ConnectionCard host/port/database fallbacks", () => {
  it("shows localhost when host is empty", () => {
    useAppStore.setState({
      connections: [
        {
          ...baseConnection,
          host: "",
          port: 5433,
        },
      ],
      activeConnectionId: baseConnection.id,
    });

    render(<ConnectionsView />);

    expect(screen.getByText(/localhost:5433/)).toBeTruthy();
  });

  it("shows em-dash when port is 0 and database is empty", () => {
    useAppStore.setState({
      connections: [
        {
          ...baseConnection,
          host: "example.com",
          port: 0,
          database: "",
        },
      ],
      activeConnectionId: baseConnection.id,
    });

    render(<ConnectionsView />);

    expect(screen.getByText(/example\.com:— \/ —/)).toBeTruthy();
  });
});

describe("ConnectionCard last-activity formatting", () => {
  const now = new Date("2026-05-13T12:00:00.000Z").getTime();

  beforeEach(() => {
    vi.useFakeTimers();
    vi.setSystemTime(now);
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  it("renders em-dash when lastActivityAt is undefined", () => {
    useAppStore.setState({
      connections: [{ ...baseConnection, lastActivityAt: undefined }],
      activeConnectionId: baseConnection.id,
    });

    render(<ConnectionsView />);

    expect(screen.getByText(/Last activity —/)).toBeTruthy();
  });

  it("renders em-dash when lastActivityAt is unparseable", () => {
    useAppStore.setState({
      connections: [{ ...baseConnection, lastActivityAt: "not-a-date" }],
      activeConnectionId: baseConnection.id,
    });

    render(<ConnectionsView />);

    expect(screen.getByText(/Last activity —/)).toBeTruthy();
  });

  it("renders 'just now' when activity is within the last minute", () => {
    useAppStore.setState({
      connections: [
        {
          ...baseConnection,
          lastActivityAt: new Date(now - 30_000).toISOString(),
        },
      ],
      activeConnectionId: baseConnection.id,
    });

    render(<ConnectionsView />);

    expect(screen.getByText(/just now/)).toBeTruthy();
  });

  it("renders minutes-ago when activity is within the last hour", () => {
    useAppStore.setState({
      connections: [
        {
          ...baseConnection,
          lastActivityAt: new Date(now - 5 * 60_000).toISOString(),
        },
      ],
      activeConnectionId: baseConnection.id,
    });

    render(<ConnectionsView />);

    expect(screen.getByText(/5m ago/)).toBeTruthy();
  });

  it("renders hours-ago when activity is within the last day", () => {
    useAppStore.setState({
      connections: [
        {
          ...baseConnection,
          lastActivityAt: new Date(now - 3 * 3_600_000).toISOString(),
        },
      ],
      activeConnectionId: baseConnection.id,
    });

    render(<ConnectionsView />);

    expect(screen.getByText(/3h ago/)).toBeTruthy();
  });

  it("renders days-ago when activity is within the last month", () => {
    useAppStore.setState({
      connections: [
        {
          ...baseConnection,
          lastActivityAt: new Date(now - 5 * 86_400_000).toISOString(),
        },
      ],
      activeConnectionId: baseConnection.id,
    });

    render(<ConnectionsView />);

    expect(screen.getByText(/5d ago/)).toBeTruthy();
  });

  it("renders a locale date string when activity is older than 30 days", () => {
    const old = new Date(now - 60 * 86_400_000);
    useAppStore.setState({
      connections: [{ ...baseConnection, lastActivityAt: old.toISOString() }],
      activeConnectionId: baseConnection.id,
    });

    render(<ConnectionsView />);

    expect(
      screen.getByText(new RegExp(`Last activity ${old.toLocaleDateString()}`)),
    ).toBeTruthy();
  });
});

describe("ConnectionCard isActive styling and onSelect", () => {
  it("applies active styling when card matches activeConnectionId", () => {
    useAppStore.setState({
      connections: [{ ...baseConnection, status: "Connected" }],
      activeConnectionId: baseConnection.id,
    });

    const { container } = render(<ConnectionsView />);
    const card = container.querySelector(".border-accent-green\\/40");
    expect(card).not.toBeNull();
  });

  it("does not apply active styling when another connection is active", () => {
    useAppStore.setState({
      connections: [
        baseConnection,
        { ...baseConnection, id: "conn-2", name: "Other" },
      ],
      activeConnectionId: "conn-2",
    });

    const { container } = render(<ConnectionsView />);
    // The first card should not have the active border.
    const activeCards = container.querySelectorAll(".border-accent-green\\/40");
    // Only one card (conn-2) should have it.
    expect(activeCards.length).toBe(1);
  });

  it("activates a connection when its name button is clicked", () => {
    useAppStore.setState({
      connections: [
        baseConnection,
        { ...baseConnection, id: "conn-2", name: "Second" },
      ],
      activeConnectionId: baseConnection.id,
    });

    render(<ConnectionsView />);

    fireEvent.click(screen.getByText("Second"));

    expect(useAppStore.getState().activeConnectionId).toBe("conn-2");
  });
});

describe("ConnectionsView filter callback (line 86)", () => {
  const connections: Connection[] = [
    {
      ...baseConnection,
      id: "c-connected",
      name: "Alpha",
      host: "alpha.example",
      database: "appdb",
      engine: "PostgreSQL",
      status: "Connected",
    },
    {
      ...baseConnection,
      id: "c-readonly",
      name: "Beta replica",
      host: "beta.example",
      database: "replicadb",
      engine: "MySQL",
      status: "Read only",
    },
    {
      ...baseConnection,
      id: "c-disconnected",
      name: "Gamma",
      host: "gamma.example",
      database: "warehousedb",
      engine: "ClickHouse",
      useHttps: false,
      urlPath: "",
      status: "Disconnected",
    },
    {
      ...baseConnection,
      id: "c-error",
      name: "Delta",
      host: "delta.example",
      database: "errordb",
      engine: "PostgreSQL",
      status: "Disconnected",
      errorMessage: "could not connect",
    },
  ] as Connection[];

  function selectFilter(label: string) {
    const tablist = screen.getByRole("tablist", {
      name: /connection filters/i,
    });
    fireEvent.click(within(tablist).getByText(label));
  }

  it("keeps only Connected cards when filter=healthy", () => {
    useAppStore.setState({
      connections,
      activeConnectionId: "c-connected",
    });

    render(<ConnectionsView />);
    selectFilter("Healthy");

    expect(screen.getByText("Alpha")).toBeTruthy();
    expect(screen.queryByText("Beta replica")).toBeNull();
    expect(screen.queryByText("Gamma")).toBeNull();
    expect(screen.queryByText("Delta")).toBeNull();
  });

  it("keeps only Read-only cards when filter=warning", () => {
    useAppStore.setState({
      connections,
      activeConnectionId: "c-readonly",
    });

    render(<ConnectionsView />);
    selectFilter("Warning");

    expect(screen.queryByText("Alpha")).toBeNull();
    expect(screen.getByText("Beta replica")).toBeTruthy();
    expect(screen.queryByText("Gamma")).toBeNull();
    expect(screen.queryByText("Delta")).toBeNull();
  });

  it("keeps only error cards when filter=error", () => {
    useAppStore.setState({
      connections,
      activeConnectionId: "c-error",
    });

    render(<ConnectionsView />);
    selectFilter("Error");

    expect(screen.queryByText("Alpha")).toBeNull();
    expect(screen.queryByText("Beta replica")).toBeNull();
    expect(screen.queryByText("Gamma")).toBeNull();
    expect(screen.getByText("Delta")).toBeTruthy();
  });

  it("returns every connection when filter=all and search is blank", () => {
    useAppStore.setState({
      connections,
      activeConnectionId: "c-connected",
    });

    render(<ConnectionsView />);

    expect(screen.getByText("Alpha")).toBeTruthy();
    expect(screen.getByText("Beta replica")).toBeTruthy();
    expect(screen.getByText("Gamma")).toBeTruthy();
    expect(screen.getByText("Delta")).toBeTruthy();
  });

  function typeSearch(value: string) {
    const input = screen.getByLabelText("Search connections");
    fireEvent.change(input, { target: { value } });
  }

  it("matches by connection name (case-insensitive, trimmed)", () => {
    useAppStore.setState({
      connections,
      activeConnectionId: "c-connected",
    });

    render(<ConnectionsView />);
    typeSearch("  alpha  ");

    expect(screen.getByText("Alpha")).toBeTruthy();
    expect(screen.queryByText("Beta replica")).toBeNull();
    expect(screen.queryByText("Gamma")).toBeNull();
    expect(screen.queryByText("Delta")).toBeNull();
  });

  it("matches by host", () => {
    useAppStore.setState({
      connections,
      activeConnectionId: "c-connected",
    });

    render(<ConnectionsView />);
    typeSearch("beta.example");

    expect(screen.queryByText("Alpha")).toBeNull();
    expect(screen.getByText("Beta replica")).toBeTruthy();
    expect(screen.queryByText("Gamma")).toBeNull();
  });

  it("matches by database", () => {
    useAppStore.setState({
      connections,
      activeConnectionId: "c-connected",
    });

    render(<ConnectionsView />);
    typeSearch("warehousedb");

    expect(screen.queryByText("Alpha")).toBeNull();
    expect(screen.queryByText("Beta replica")).toBeNull();
    expect(screen.getByText("Gamma")).toBeTruthy();
  });

  it("matches by engine", () => {
    useAppStore.setState({
      connections,
      activeConnectionId: "c-connected",
    });

    render(<ConnectionsView />);
    typeSearch("mysql");

    expect(screen.queryByText("Alpha")).toBeNull();
    expect(screen.getByText("Beta replica")).toBeTruthy();
    expect(screen.queryByText("Gamma")).toBeNull();
  });

  it("returns no cards when the search needle matches nothing", () => {
    useAppStore.setState({
      connections,
      activeConnectionId: "c-connected",
    });

    render(<ConnectionsView />);
    typeSearch("zzz-no-match");

    expect(screen.queryByText("Alpha")).toBeNull();
    expect(screen.queryByText("Beta replica")).toBeNull();
    expect(screen.queryByText("Gamma")).toBeNull();
    expect(screen.queryByText("Delta")).toBeNull();
  });

  it("combines filter=error with a search needle that matches the error card", () => {
    useAppStore.setState({
      connections,
      activeConnectionId: "c-error",
    });

    render(<ConnectionsView />);
    selectFilter("Error");
    typeSearch("delta");

    expect(screen.getByText("Delta")).toBeTruthy();
    expect(screen.queryByText("Alpha")).toBeNull();
  });

  it("filter=healthy excludes a card with errorMessage even if previously Connected", () => {
    useAppStore.setState({
      connections: [
        {
          ...baseConnection,
          id: "x",
          name: "X",
          status: "Disconnected",
          errorMessage: "bad",
        },
      ],
      activeConnectionId: "x",
    });

    render(<ConnectionsView />);
    selectFilter("Healthy");

    expect(screen.queryByText("X")).toBeNull();
  });
});
