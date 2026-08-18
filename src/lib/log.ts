/* oxlint-disable anti-slop/no-unknown-parameters -- Logging intentionally accepts arbitrary diagnostic context at the application boundary and serializes it defensively. */
/**
 * Frontend logging that routes through `tauri-plugin-log`, so JS-side
 * info/warn/error calls land in the same file target as Rust-side
 * `log::info!()` / `log::error!()` (release builds rotate to
 * `app.path().app_log_dir()`; dev builds also stream to stdout +
 * DevTools).
 *
 * Outside Tauri (jsdom tests, web preview), falls back to `console.*`
 * so call sites stay safe.
 */

import {
  debug as tauriDebug,
  error as tauriError,
  info as tauriInfo,
  warn as tauriWarn,
} from "@tauri-apps/plugin-log";

import { isTauri } from "@/lib/tauri";

type LogFn = (message: string) => Promise<void>;

const dispatch = (
  tauriFn: LogFn,
  fallback: (...args: unknown[]) => void,
  message: string,
  context?: unknown,
): void => {
  if (!isTauri()) {
    if (context === undefined) fallback(message);
    else fallback(message, context);
    return;
  }
  const fullMessage =
    context === undefined ? message : `${message} ${safeStringify(context)}`;
  // Fire-and-forget. A failed log shouldn't surface to the user.
  tauriFn(fullMessage).catch(() => {});
};

function safeStringify(value: unknown): string {
  if (value instanceof Error) return value.stack ?? value.message;
  try {
    return JSON.stringify(value);
  } catch {
    return String(value);
  }
}

export const log = {
  debug: (message: string, context?: unknown) =>
    dispatch(tauriDebug, console.debug, message, context),
  info: (message: string, context?: unknown) =>
    dispatch(tauriInfo, console.info, message, context),
  warn: (message: string, context?: unknown) =>
    dispatch(tauriWarn, console.warn, message, context),
  error: (message: string, context?: unknown) =>
    dispatch(tauriError, console.error, message, context),
};
