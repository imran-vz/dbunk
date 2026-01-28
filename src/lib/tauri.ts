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
