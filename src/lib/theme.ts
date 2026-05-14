/**
 * Theme mode — `"system"` follows the OS via `prefers-color-scheme`,
 * `"light"` / `"dark"` are explicit overrides. See
 * `docs/design/theme-support-plan.md`.
 *
 * Persistence has two layers: AppSettings/SQLite is canonical;
 * `localStorage[LOCAL_STORAGE_KEY]` is a boot cache so an inline
 * `<script>` can apply the resolved class before React mounts and
 * avoid a flash of the wrong theme.
 */

export type ThemeMode = "system" | "light" | "dark";

export const LOCAL_STORAGE_KEY = "dbunk.theme";

export function resolveMode(mode: ThemeMode): "light" | "dark" {
  if (mode !== "system") return mode;
  if (typeof window === "undefined" || typeof window.matchMedia !== "function")
    return "dark";
  return window.matchMedia("(prefers-color-scheme: dark)").matches
    ? "dark"
    : "light";
}

export function applyTheme(mode: ThemeMode) {
  if (typeof document === "undefined") return;
  const resolved = resolveMode(mode);
  document.documentElement.classList.toggle("dark", resolved === "dark");
}

export function subscribeSystem(handler: () => void): () => void {
  if (typeof window === "undefined" || typeof window.matchMedia !== "function")
    return () => {};
  const mql = window.matchMedia("(prefers-color-scheme: dark)");
  mql.addEventListener("change", handler);
  return () => mql.removeEventListener("change", handler);
}

export function isThemeMode(value: unknown): value is ThemeMode {
  return value === "system" || value === "light" || value === "dark";
}

export function readStoredMode(): ThemeMode {
  if (typeof window === "undefined") return "system";
  try {
    const raw = window.localStorage.getItem(LOCAL_STORAGE_KEY);
    return isThemeMode(raw) ? raw : "system";
  } catch {
    return "system";
  }
}

export function writeStoredMode(mode: ThemeMode) {
  if (typeof window === "undefined") return;
  try {
    window.localStorage.setItem(LOCAL_STORAGE_KEY, mode);
  } catch {
    // localStorage may be unavailable in private mode / quota exceeded —
    // boot cache is best-effort; SQLite is canonical.
  }
}
