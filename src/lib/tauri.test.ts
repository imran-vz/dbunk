// @vitest-environment jsdom
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

type TauriTestWindow = Window & {
  __TAURI_INTERNALS__?: unknown;
};

const tauriWindowApi = vi.hoisted(() => {
  const currentWindow = {
    isFullscreen: vi.fn(),
    isMaximized: vi.fn(),
    toggleMaximize: vi.fn(),
    startDragging: vi.fn(),
    onResized: vi.fn(),
    onFocusChanged: vi.fn(),
  };
  const monitor = {
    scaleFactor: 2,
    workArea: {
      position: { x: 0, y: 50 },
      size: { width: 2880, height: 1750 },
    },
  };

  return {
    currentMonitor: vi.fn(),
    currentWindow,
    getCurrentWindow: vi.fn(() => currentWindow),
    monitor,
  };
});

vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(),
}));

vi.mock("@tauri-apps/api/window", () => ({
  currentMonitor: tauriWindowApi.currentMonitor,
  getCurrentWindow: tauriWindowApi.getCurrentWindow,
}));

const resetTauriWindowMocks = () => {
  tauriWindowApi.currentMonitor.mockResolvedValue(tauriWindowApi.monitor);
  tauriWindowApi.currentWindow.isFullscreen.mockResolvedValue(false);
  tauriWindowApi.currentWindow.isMaximized.mockResolvedValue(false);
  tauriWindowApi.currentWindow.toggleMaximize.mockResolvedValue(undefined);
  tauriWindowApi.currentWindow.startDragging.mockResolvedValue(undefined);
  tauriWindowApi.currentWindow.onResized.mockResolvedValue(vi.fn());
  tauriWindowApi.currentWindow.onFocusChanged.mockResolvedValue(vi.fn());
};

const loadTauriModule = async () => import("@/lib/tauri");
const setViewportSize = (width: number, height: number) => {
  Object.defineProperty(window, "innerWidth", {
    configurable: true,
    value: width,
  });
  Object.defineProperty(window, "innerHeight", {
    configurable: true,
    value: height,
  });
};

beforeEach(() => {
  vi.resetModules();
  vi.clearAllMocks();
  resetTauriWindowMocks();
  setViewportSize(900, 650);
  Object.defineProperty(window, "__TAURI_INTERNALS__", {
    configurable: true,
    value: {},
  });
});

afterEach(() => {
  delete (window as TauriTestWindow).__TAURI_INTERNALS__;
});

describe("tauriToggleWindowZoom", () => {
  it("prepares a viewport transition to the monitor work area before native zoom", async () => {
    const { tauriPrepareWindowZoomTransition } = await loadTauriModule();

    await expect(tauriPrepareWindowZoomTransition()).resolves.toEqual({
      fromWidth: 900,
      fromHeight: 650,
      toWidth: 1440,
      toHeight: 875,
    });

    expect(tauriWindowApi.currentWindow.toggleMaximize).not.toHaveBeenCalled();
  });

  it("uses native maximize for the actual window zoom", async () => {
    const { tauriToggleWindowZoom } = await loadTauriModule();

    await tauriToggleWindowZoom();

    expect(tauriWindowApi.currentWindow.toggleMaximize).toHaveBeenCalledTimes(
      1,
    );
  });

  it("does not prepare an internal viewport transition while restoring", async () => {
    const { tauriPrepareWindowZoomTransition } = await loadTauriModule();

    await tauriPrepareWindowZoomTransition();
    setViewportSize(1440, 875);
    tauriWindowApi.currentWindow.isMaximized.mockResolvedValue(true);

    await expect(tauriPrepareWindowZoomTransition()).resolves.toBeNull();
    expect(tauriWindowApi.currentWindow.toggleMaximize).not.toHaveBeenCalled();
  });

  it("does not resize while the native window is fullscreen", async () => {
    tauriWindowApi.currentWindow.isFullscreen.mockResolvedValue(true);
    const { tauriPrepareWindowZoomTransition, tauriToggleWindowZoom } =
      await loadTauriModule();

    await expect(tauriPrepareWindowZoomTransition()).resolves.toBeNull();
    await tauriToggleWindowZoom();

    expect(tauriWindowApi.currentWindow.toggleMaximize).not.toHaveBeenCalled();
  });
});
