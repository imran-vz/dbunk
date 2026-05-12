import { invoke } from "@tauri-apps/api/core";
import { currentMonitor, getCurrentWindow } from "@tauri-apps/api/window";

type UnlistenFn = () => void;
type ViewportSize = {
  width: number;
  height: number;
};
export type WindowViewportZoomTransition = {
  fromWidth: number;
  fromHeight: number;
  toWidth: number;
  toHeight: number;
};

let windowZoomInFlight: Promise<void> | null = null;

export const isTauri = () =>
  typeof window !== "undefined" &&
  ("__TAURI_INTERNALS__" in window || "__TAURI__" in window);

export async function tauriInvoke<T>(
  command: string,
  payload?: Record<string, unknown>,
) {
  return invoke<T>(command, payload);
}

export async function tauriStartDragging() {
  if (!isTauri()) {
    return;
  }
  await getCurrentWindow().startDragging();
}

export async function tauriRestoreWindowTrafficLightPosition() {
  if (!isTauri()) {
    return;
  }
  await tauriInvoke("restore_window_traffic_light_position");
}

export async function tauriPrepareWindowZoomTransition(): Promise<WindowViewportZoomTransition | null> {
  if (!isTauri()) {
    return null;
  }
  const appWindow = getCurrentWindow();
  if (await appWindow.isFullscreen()) {
    return null;
  }

  const from = currentBrowserViewportSize();
  if (!from) {
    return null;
  }

  if (await appWindow.isMaximized()) {
    return null;
  }

  const monitor = await currentMonitor();
  if (!monitor) {
    return null;
  }

  const to = {
    width: monitor.workArea.size.width / monitor.scaleFactor,
    height: monitor.workArea.size.height / monitor.scaleFactor,
  };
  return createViewportZoomTransition(from, to);
}

export async function tauriToggleWindowZoom() {
  if (!isTauri()) {
    return;
  }
  const appWindow = getCurrentWindow();
  if (await appWindow.isFullscreen()) {
    return;
  }
  windowZoomInFlight ??= appWindow.toggleMaximize().finally(() => {
    windowZoomInFlight = null;
  });
  await windowZoomInFlight;
}

export async function tauriOnWindowFullscreenChange(
  listener: (fullscreen: boolean) => void,
): Promise<UnlistenFn> {
  if (!isTauri()) {
    return () => undefined;
  }
  const appWindow = getCurrentWindow();
  let disposed = false;
  const emitFullscreenState = async () => {
    if (disposed) {
      return;
    }
    listener(await appWindow.isFullscreen());
  };
  const emitFromNativeEvent = () => {
    void emitFullscreenState().catch(() => undefined);
    window.setTimeout(() => {
      void emitFullscreenState().catch(() => undefined);
    }, 150);
  };
  const unlisteners = await Promise.all([
    appWindow.onResized(emitFromNativeEvent),
    appWindow.onFocusChanged(emitFromNativeEvent),
  ]);
  await emitFullscreenState();

  return () => {
    disposed = true;
    for (const unlisten of unlisteners) {
      unlisten();
    }
  };
}

/**
 * Coerce any thrown value into a user-displayable string. Tauri's
 * invoke surface tends to throw plain strings (Result::Err mapped via
 * serde), but we also see real `Error` instances and structured
 * objects from non-Tauri code paths. Lives next to `tauriInvoke`
 * because every Tauri-calling action needs it.
 */
export function errorToMessage(error: unknown): string {
  if (error instanceof Error) {
    return error.message;
  }
  if (typeof error === "string") {
    return error;
  }
  if (error && typeof error === "object" && "message" in error) {
    const message = (error as { message?: unknown }).message;
    if (typeof message === "string") {
      return message;
    }
  }
  try {
    return JSON.stringify(error);
  } catch {
    return "Unknown error";
  }
}

function currentBrowserViewportSize(): ViewportSize | null {
  if (typeof window === "undefined") {
    return null;
  }
  const width = window.innerWidth;
  const height = window.innerHeight;
  if (width <= 0 || height <= 0) {
    return null;
  }
  return { width, height };
}

function createViewportZoomTransition(
  from: ViewportSize,
  to: ViewportSize,
): WindowViewportZoomTransition | null {
  if (
    !Number.isFinite(to.width) ||
    !Number.isFinite(to.height) ||
    to.width <= 0 ||
    to.height <= 0 ||
    (Math.abs(from.width - to.width) < 2 &&
      Math.abs(from.height - to.height) < 2)
  ) {
    return null;
  }
  return {
    fromWidth: from.width,
    fromHeight: from.height,
    toWidth: to.width,
    toHeight: to.height,
  };
}
