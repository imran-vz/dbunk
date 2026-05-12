import { invoke } from "@tauri-apps/api/core";

export const isTauri = () =>
  typeof window !== "undefined" &&
  ("__TAURI_INTERNALS__" in window || "__TAURI__" in window);

export async function tauriInvoke<T>(
  command: string,
  payload?: Record<string, unknown>,
) {
  return invoke<T>(command, payload);
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
