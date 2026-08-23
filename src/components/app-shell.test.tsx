/* oxlint-disable anti-slop/no-known-value-widening anti-slop/no-module-mocking anti-slop/no-unknown-parameters anti-slop/require-safety-comment-for-type-assertion -- Test fixtures use controlled mocks and assertions to exercise otherwise inaccessible boundaries. */
// @vitest-environment jsdom
import {
  act,
  cleanup,
  fireEvent,
  render,
  screen,
  waitFor,
} from "@testing-library/react";
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
    tauriRestoreWindowTrafficLightPosition: vi.fn(() => Promise.resolve()),
    tauriToggleWindowZoom: vi.fn(() => Promise.resolve()),
  };
});

vi.mock("@/lib/tauri", () => ({
  isTauri: vi.fn(() => true),
  tauriInvoke: vi.fn(),
  tauriOnWindowFullscreenChange: tauriMocks.tauriOnWindowFullscreenChange,
  tauriPrepareWindowZoomTransition: tauriMocks.tauriPrepareWindowZoomTransition,
  tauriRestoreWindowTrafficLightPosition:
    tauriMocks.tauriRestoreWindowTrafficLightPosition,
  tauriStartDragging: tauriMocks.tauriStartDragging,
  tauriToggleWindowZoom: tauriMocks.tauriToggleWindowZoom,
  errorToMessage: (error: unknown) =>
    error instanceof Error ? error.message : String(error),
}));

// The workbench mocks mirror the real shells' contract: a top drag region
// wired to the handlers AppShell passes down, so drag/zoom behaviour can be
// exercised without rendering the full workbench tree.
vi.mock("@/components/workbench/relational-workbench", () => ({
  RelationalWorkbench: ({
    onPointerDown,
    onDoubleClick,
  }: {
    onPointerDown: React.PointerEventHandler<HTMLElement>;
    onDoubleClick: React.MouseEventHandler<HTMLElement>;
  }) => (
    <div
      data-testid="relational-workbench"
      data-window-drag-region
      onPointerDown={onPointerDown}
      onDoubleClick={onDoubleClick}
    />
  ),
}));

vi.mock("@/components/workbench/keyvalue-workbench", () => ({
  KeyValueWorkbench: () => <div data-testid="keyvalue-workbench" />,
}));

vi.mock("@/components/settings-view", () => ({
  SettingsView: () => <div data-testid="settings-view" />,
}));

vi.mock("@/components/credential-onboarding", () => ({
  CredentialOnboarding: () => <div data-testid="credential-onboarding" />,
  CredentialUnlock: () => <div data-testid="credential-unlock" />,
}));

import { AppShell } from "@/components/app-shell";
import { type Connection, useAppStore } from "@/lib/store";
import {
  tauriPrepareWindowZoomTransition,
  tauriStartDragging,
  tauriToggleWindowZoom,
} from "@/lib/tauri";

const initialStoreState = useAppStore.getState();
const mockedStartDragging = vi.mocked(tauriStartDragging);
const mockedPrepareWindowZoomTransition = vi.mocked(
  tauriPrepareWindowZoomTransition,
);
const mockedToggleWindowZoom = vi.mocked(tauriToggleWindowZoom);

const connection: Connection = {
  id: "conn-1",
  name: "Local MySQL",
  database: "reports",
  status: "Connected",
  engine: "MySQL",
  host: "localhost",
  port: 3306,
  user: "root",
  password: "",
  role: "",
  latency: "12 ms",
  ssl: true,
};

const readySettings = {
  onboardingCompleted: true,
  credentialStorageMode: "plain-sqlite" as const,
  credentialState: "ready" as const,
  configDir: "/tmp/dbunk",
};

beforeEach(() => {
  // The macOS traffic-light gutter only renders on a mac platform.
  // jsdom defaults to an empty `navigator.platform`, so stub it here
  // for the dragging suite which asserts mac-only behaviour.
  Object.defineProperty(window.navigator, "platform", {
    value: "MacIntel",
    configurable: true,
  });
  window.localStorage.clear();
  useAppStore.setState(initialStoreState, true);
  useAppStore.setState({
    activeView: "workspace",
    activeConnectionId: connection.id,
    appSettings: readySettings,
    appSettingsStatus: { state: "ready" },
    connections: [connection],
    loadAppSettings: vi.fn(async () => readySettings),
    loadBastionServers: vi.fn(async () => undefined),
    loadConnections: vi.fn(async () => undefined),
    loadQueryHistory: vi.fn(async () => undefined),
    loadSavedQueries: vi.fn(async () => undefined),
    runHealthChecks: vi.fn(async () => undefined),
  });
  mockedStartDragging.mockClear();
  mockedPrepareWindowZoomTransition.mockClear();
  mockedPrepareWindowZoomTransition.mockResolvedValue(null);
  tauriMocks.tauriRestoreWindowTrafficLightPosition.mockClear();
  mockedToggleWindowZoom.mockClear();
  tauriMocks.tauriOnWindowFullscreenChange.mockClear();
  tauriMocks.unlistenFullscreen.mockClear();
  tauriMocks.state.fullscreenHandler = undefined;
});

afterEach(() => {
  cleanup();
  useAppStore.setState(initialStoreState, true);
});

describe("AppShell window controls", () => {
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

  it("starts a Tauri window drag from the workbench drag region", () => {
    render(<AppShell />);

    fireEvent.pointerDown(screen.getByTestId("relational-workbench"), {
      button: 0,
    });

    expect(mockedStartDragging).toHaveBeenCalledTimes(1);
  });

  it("zooms the window on a drag-region double click", async () => {
    render(<AppShell />);

    const dragRegion = screen.getByTestId("relational-workbench");
    fireEvent.pointerDown(dragRegion, { button: 0, detail: 2 });
    fireEvent.doubleClick(dragRegion, { button: 0 });

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
      fireEvent.doubleClick(screen.getByTestId("relational-workbench"), {
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
    fireEvent.doubleClick(screen.getByTestId("relational-workbench"), {
      button: 0,
    });

    await waitFor(() => {
      expect(mockedToggleWindowZoom).toHaveBeenCalledTimes(1);
    });
    expect(shell.dataset.windowViewportZoom).toBeUndefined();
    expect(shell.style.transform).toBe("");
  });

  it("restores the native macOS traffic-light position after leaving fullscreen", async () => {
    vi.useFakeTimers();
    try {
      render(<AppShell />);

      act(() => {
        tauriMocks.state.fullscreenHandler?.(true);
      });
      expect(
        tauriMocks.tauriRestoreWindowTrafficLightPosition,
      ).not.toHaveBeenCalled();

      act(() => {
        tauriMocks.state.fullscreenHandler?.(false);
      });

      expect(
        tauriMocks.tauriRestoreWindowTrafficLightPosition,
      ).toHaveBeenCalledTimes(1);

      await act(async () => {
        vi.advanceTimersByTime(120);
      });
      expect(
        tauriMocks.tauriRestoreWindowTrafficLightPosition,
      ).toHaveBeenCalledTimes(2);

      await act(async () => {
        vi.advanceTimersByTime(220);
      });
      expect(
        tauriMocks.tauriRestoreWindowTrafficLightPosition,
      ).toHaveBeenCalledTimes(3);
    } finally {
      vi.useRealTimers();
    }
  });
});

describe("AppShell workbench routing", () => {
  it("renders RelationalWorkbench for relational connections", () => {
    render(<AppShell />);

    expect(screen.getByTestId("relational-workbench")).toBeTruthy();
    expect(screen.queryByTestId("keyvalue-workbench")).toBeNull();
  });

  it("renders KeyValueWorkbench for Redis connections", () => {
    const redisConnection: Connection = {
      id: "conn-redis",
      name: "Local Redis",
      database: "",
      status: "Connected",
      engine: "Redis",
      host: "localhost",
      port: 6379,
      user: "",
      password: "",
      role: "",
      latency: "3 ms",
      dbNumber: 0,
      useTls: false,
      verifyTlsCert: true,
      readOnly: false,
    };

    useAppStore.setState({
      activeConnectionId: redisConnection.id,
      connections: [redisConnection],
      activeView: "workspace",
    });

    render(<AppShell />);

    expect(screen.getByTestId("keyvalue-workbench")).toBeTruthy();
    expect(screen.queryByTestId("relational-workbench")).toBeNull();
  });
});
