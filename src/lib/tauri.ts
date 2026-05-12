import { invoke } from "@tauri-apps/api/core";
import { getCurrentWindow } from "@tauri-apps/api/window";

type UnlistenFn = () => void;

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

export async function tauriToggleMaximize() {
  if (!isTauri()) {
    return;
  }
  await getCurrentWindow().toggleMaximize();
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
