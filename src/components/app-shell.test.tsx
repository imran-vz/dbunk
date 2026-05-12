// @vitest-environment jsdom
import {
  act,
  cleanup,
  fireEvent,
  render,
  screen,
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
    tauriToggleMaximize: vi.fn(() => Promise.resolve()),
  };
});

vi.mock("@/lib/tauri", () => ({
  isTauri: vi.fn(() => true),
  tauriInvoke: vi.fn(),
  tauriOnWindowFullscreenChange: tauriMocks.tauriOnWindowFullscreenChange,
  tauriStartDragging: tauriMocks.tauriStartDragging,
  tauriToggleMaximize: tauriMocks.tauriToggleMaximize,
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
import { tauriStartDragging, tauriToggleMaximize } from "@/lib/tauri";

const initialStoreState = useAppStore.getState();
const mockedStartDragging = vi.mocked(tauriStartDragging);
const mockedToggleMaximize = vi.mocked(tauriToggleMaximize);

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
  mockedToggleMaximize.mockClear();
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

  it("toggles native maximize on a top-bar double click", () => {
    render(<AppShell />);

    const dragSpacer = screen.getByTestId("window-drag-spacer");
    fireEvent.pointerDown(dragSpacer, { button: 0, detail: 2 });
    fireEvent.doubleClick(dragSpacer, { button: 0 });

    expect(dragSpacer.hasAttribute("data-tauri-drag-region")).toBe(false);
    expect(dragSpacer.hasAttribute("data-window-drag-region")).toBe(true);
    expect(mockedStartDragging).not.toHaveBeenCalled();
    expect(mockedToggleMaximize).toHaveBeenCalledTimes(1);
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

  it("does not toggle native maximize from top-bar controls", () => {
    render(<AppShell />);

    fireEvent.doubleClick(screen.getByRole("button", { name: "Run Query" }), {
      button: 0,
    });
    fireEvent.doubleClick(screen.getByLabelText("Search tables"), {
      button: 0,
    });

    expect(mockedToggleMaximize).not.toHaveBeenCalled();
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
