// @vitest-environment jsdom
import {
  act,
  cleanup,
  fireEvent,
  render,
  screen,
  waitFor,
} from "@testing-library/react";
import type * as React from "react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const tauriMocks = vi.hoisted(() => {
  const state: {
    fullscreenHandler?: (fullscreen: boolean) => void;
  } = {};
  const unlistenFullscreen = vi.fn();
  return {
    state,
    unlistenFullscreen,
    tauriOnWindowFullscreenChange: vi.fn(
      async (handler: (fullscreen: boolean) => void) => {
        state.fullscreenHandler = handler;
        handler(false);
        return unlistenFullscreen;
      },
    ),
    tauriStartDragging: vi.fn(() => Promise.resolve()),
    tauriPrepareWindowZoomTransition: vi.fn(() => Promise.resolve(null)),
    tauriToggleWindowZoom: vi.fn(() => Promise.resolve()),
  };
});

vi.mock("@/lib/tauri", () => ({
  isTauri: vi.fn(() => true),
  tauriInvoke: vi.fn(),
  tauriOnWindowFullscreenChange: tauriMocks.tauriOnWindowFullscreenChange,
  tauriPrepareWindowZoomTransition: tauriMocks.tauriPrepareWindowZoomTransition,
  tauriStartDragging: tauriMocks.tauriStartDragging,
  tauriToggleWindowZoom: tauriMocks.tauriToggleWindowZoom,
  errorToMessage: (error: unknown) =>
    error instanceof Error ? error.message : String(error),
}));

vi.mock("@/components/workspace-view", () => ({
  WorkspaceView: () => <div data-testid="workspace-view" />,
}));

vi.mock("@/components/sidebar", () => ({
  Sidebar: () => <aside data-testid="sidebar" />,
}));

vi.mock("@/components/connections-view", () => ({
  ConnectionsView: () => <div data-testid="connections-view" />,
}));

vi.mock("@/components/settings-view", () => ({
  SettingsView: () => <div data-testid="settings-view" />,
}));

vi.mock("@/components/credential-onboarding", () => ({
  CredentialOnboarding: () => <div data-testid="credential-onboarding" />,
  CredentialUnlock: () => <div data-testid="credential-unlock" />,
}));

vi.mock("@/components/new-connection-dialog", () => ({
  NewConnectionDialog: ({ trigger }: { trigger: React.ReactNode }) => trigger,
}));

import { AppShell } from "@/components/app-shell";
import { type Connection, useAppStore } from "@/lib/store";
import {
  tauriPrepareWindowZoomTransition,
  tauriStartDragging,
  tauriToggleWindowZoom,
} from "@/lib/tauri";

const initialStoreState = useAppStore.getState();
const mockedPrepareWindowZoomTransition = vi.mocked(
  tauriPrepareWindowZoomTransition,
);
const mockedStartDragging = vi.mocked(tauriStartDragging);
const mockedToggleWindowZoom = vi.mocked(tauriToggleWindowZoom);

const connection: Connection = {
  id: "conn-1",
  name: "Local Postgres",
  database: "postgres",
  status: "Connected",
  engine: "PostgreSQL",
  host: "localhost",
  port: 5432,
  user: "postgres",
  password: "",
  role: "",
  latency: "12 ms",
  lastSync: "Just now",
  ssl: true,
};

const readySettings = {
  onboardingCompleted: true,
  credentialStorageMode: "plain-sqlite" as const,
  credentialState: "ready" as const,
  configDir: "/tmp/dbunk",
};

beforeEach(() => {
  window.localStorage.clear();
  useAppStore.setState(initialStoreState, true);
  useAppStore.setState({
    activeView: "workspace",
    activeConnectionId: connection.id,
    appSettings: readySettings,
    appSettingsStatus: { state: "ready" },
    connections: [connection],
    loadAppSettings: vi.fn(async () => readySettings),
    loadConnections: vi.fn(async () => undefined),
    loadQueryHistory: vi.fn(async () => undefined),
    loadSavedQueries: vi.fn(async () => undefined),
    runHealthChecks: vi.fn(async () => undefined),
    createNewQueryTab: vi.fn(),
  });
  mockedStartDragging.mockClear();
  mockedPrepareWindowZoomTransition.mockClear();
  mockedPrepareWindowZoomTransition.mockResolvedValue(null);
  mockedToggleWindowZoom.mockClear();
  tauriMocks.tauriOnWindowFullscreenChange.mockClear();
  tauriMocks.unlistenFullscreen.mockClear();
  tauriMocks.state.fullscreenHandler = undefined;
});

afterEach(() => {
  cleanup();
  useAppStore.setState(initialStoreState, true);
});

describe("AppShell title bar dragging", () => {
  it("starts a Tauri window drag while the app is still loading settings", () => {
    useAppStore.setState({
      appSettings: null,
      appSettingsStatus: { state: "loading" },
    });

    render(<AppShell />);

    fireEvent.pointerDown(screen.getByTestId("window-drag-surface"), {
      button: 0,
    });

    expect(mockedStartDragging).toHaveBeenCalledTimes(1);
  });

  it("starts a Tauri window drag from non-interactive top-bar space", () => {
    render(<AppShell />);

    fireEvent.pointerDown(screen.getByTestId("window-drag-spacer"), {
      button: 0,
    });

    expect(mockedStartDragging).toHaveBeenCalledTimes(1);
  });

  it("zooms the window on a top-bar double click", async () => {
    render(<AppShell />);

    const dragSpacer = screen.getByTestId("window-drag-spacer");
    fireEvent.pointerDown(dragSpacer, { button: 0, detail: 2 });
    fireEvent.doubleClick(dragSpacer, { button: 0 });

    expect(dragSpacer.hasAttribute("data-tauri-drag-region")).toBe(false);
    expect(dragSpacer.hasAttribute("data-window-drag-region")).toBe(true);
    expect(mockedStartDragging).not.toHaveBeenCalled();
    await waitFor(() => {
      expect(mockedPrepareWindowZoomTransition).toHaveBeenCalledTimes(1);
      expect(mockedToggleWindowZoom).toHaveBeenCalledTimes(1);
    });
  });

  it("animates the app viewport while native zoom runs", async () => {
    vi.useFakeTimers();
    mockedPrepareWindowZoomTransition.mockResolvedValueOnce({
      fromWidth: 900,
      fromHeight: 650,
      toWidth: 1440,
      toHeight: 875,
    });
    try {
      render(<AppShell />);

      const shell = screen.getByTestId("app-shell");
      fireEvent.doubleClick(screen.getByTestId("window-drag-spacer"), {
        button: 0,
      });

      await act(async () => {
        await Promise.resolve();
      });
      expect(shell.dataset.windowViewportZoom).toBe("idle");
      expect(shell.style.width).toBe("1440px");
      expect(shell.style.height).toBe("875px");
      expect(shell.style.transform).toBe("scale(0.625, 0.7428571428571429)");

      act(() => {
        vi.advanceTimersByTime(16);
      });
      expect(shell.dataset.windowViewportZoom).toBe("active");
      expect(shell.style.transform).toBe("scale(1)");

      await act(async () => {
        vi.advanceTimersByTime(500);
      });
      expect(shell.dataset.windowViewportZoom).toBeUndefined();
    } finally {
      vi.useRealTimers();
    }
  });

  it("does not animate the app viewport while native restore runs", async () => {
    mockedPrepareWindowZoomTransition.mockResolvedValueOnce({
      fromWidth: 1440,
      fromHeight: 875,
      toWidth: 900,
      toHeight: 650,
    });

    render(<AppShell />);

    const shell = screen.getByTestId("app-shell");
    fireEvent.doubleClick(screen.getByTestId("window-drag-spacer"), {
      button: 0,
    });

    await waitFor(() => {
      expect(mockedToggleWindowZoom).toHaveBeenCalledTimes(1);
    });
    expect(shell.dataset.windowViewportZoom).toBeUndefined();
    expect(shell.style.transform).toBe("");
  });

  it("does not start dragging from top-bar controls", () => {
    render(<AppShell />);

    fireEvent.pointerDown(screen.getByRole("button", { name: "Run Query" }), {
      button: 0,
    });
    fireEvent.pointerDown(screen.getByLabelText("Search tables"), {
      button: 0,
    });

    expect(mockedStartDragging).not.toHaveBeenCalled();
  });

  it("renders a bounded resize handle for the connections and tables sidebar", () => {
    window.localStorage.setItem("dbunk.sidebar.globalWidth", "999");
    render(<AppShell />);

    const resizer = screen.getByRole("separator", {
      name: "Resize connections and tables sidebar",
    });
    const sidebarPanel = resizer.previousElementSibling as HTMLElement;

    expect(resizer.getAttribute("aria-valuemin")).toBe("220");
    expect(resizer.getAttribute("aria-valuemax")).toBe("420");
    expect(resizer.getAttribute("aria-valuenow")).toBe("420");
    expect(sidebarPanel.style.width).toBe("420px");

    fireEvent.keyDown(resizer, { key: "ArrowLeft", shiftKey: true });

    expect(resizer.getAttribute("aria-valuenow")).toBe("388");
    expect(sidebarPanel.style.width).toBe("388px");
  });

  it("does not zoom the window from top-bar controls", () => {
    render(<AppShell />);

    fireEvent.doubleClick(screen.getByRole("button", { name: "Run Query" }), {
      button: 0,
    });
    fireEvent.doubleClick(screen.getByLabelText("Search tables"), {
      button: 0,
    });

    expect(mockedToggleWindowZoom).not.toHaveBeenCalled();
  });

  it("collapses the macOS traffic-light gutter while the native window is fullscreen", () => {
    render(<AppShell />);

    const topBar = screen.getByTestId("app-top-bar");
    expect(topBar.dataset.windowFullscreen).toBe("false");
    expect(topBar.className).toContain("pl-22");

    act(() => {
      tauriMocks.state.fullscreenHandler?.(true);
    });

    expect(topBar.dataset.windowFullscreen).toBe("true");
    expect(topBar.className).toContain("pl-2.5");
    expect(topBar.className).not.toContain("pl-22");

    act(() => {
      tauriMocks.state.fullscreenHandler?.(false);
    });

    expect(topBar.dataset.windowFullscreen).toBe("false");
    expect(topBar.className).toContain("pl-22");
  });
});
