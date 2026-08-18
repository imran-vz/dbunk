/* oxlint-disable anti-slop/no-runtime-typeof, anti-slop/no-unknown-parameters, anti-slop/no-unknown-returns, anti-slop/require-safety-comment-for-type-assertion -- Theme persistence and optional browser APIs are external boundaries validated here before use. */
/**
 * Theme mode — `"system"` follows the OS via `prefers-color-scheme`,
 * `"light"` / `"dark"` are explicit overrides. See
 * `docs/design/theme-support-plan.md`.
 *
 * Persistence has two layers: AppSettings/SQLite is canonical;
 * `localStorage[LOCAL_STORAGE_KEY]` / `LOCAL_STORAGE_PRESET_KEY` are
 * the boot cache so an inline `<script>` can apply the resolved
 * class + `data-theme` before React mounts and avoid a flash of the
 * wrong theme.
 *
 * Mode and Preset are orthogonal axes (`.dark[data-theme="github"]`
 * is a valid combination). Some presets (Dracula) are intrinsically
 * dark — the menu gates the Mode picker in that case and we force
 * the `.dark` class regardless of the stored mode.
 */

export type ThemeMode = "system" | "light" | "dark";
export type ThemePreset = "default" | "dracula" | "github" | "gruvbox";

export const LOCAL_STORAGE_KEY = "dbunk.theme";
export const LOCAL_STORAGE_PRESET_KEY = "dbunk.theme.preset";

const THEME_MODES = new Set<ThemeMode>(["system", "light", "dark"]);
const THEME_PRESETS = new Set<ThemePreset>([
  "default",
  "dracula",
  "github",
  "gruvbox",
]);

/** Presets that don't have a light variant — Mode is forced to dark. */
const INTRINSICALLY_DARK_PRESETS = new Set<ThemePreset>(["dracula"]);

export function isPresetIntrinsicallyDark(preset: ThemePreset): boolean {
  return INTRINSICALLY_DARK_PRESETS.has(preset);
}

export function resolveMode(mode: ThemeMode): "light" | "dark" {
  if (mode !== "system") return mode;
  if (typeof window === "undefined" || typeof window.matchMedia !== "function")
    return "dark";
  return window.matchMedia("(prefers-color-scheme: dark)").matches
    ? "dark"
    : "light";
}

type DocumentWithViewTransitions = Document & {
  startViewTransition?: (callback: () => void) => unknown;
};

export function applyTheme(
  mode: ThemeMode,
  preset: ThemePreset = "default",
): void {
  if (typeof document === "undefined") return;
  const root = document.documentElement;
  const nextDark =
    isPresetIntrinsicallyDark(preset) || resolveMode(mode) === "dark";
  const nextPresetAttr = preset === "default" ? null : preset;

  // Bail when the resolved state already matches — keeps boot-time
  // hydration and idempotent re-applies from triggering a no-op
  // view-transition animation.
  const currentDark = root.classList.contains("dark");
  const currentPresetAttr = root.getAttribute("data-theme");
  if (currentDark === nextDark && currentPresetAttr === nextPresetAttr) return;

  const commit = () => {
    root.classList.toggle("dark", nextDark);
    if (nextPresetAttr === null) {
      root.removeAttribute("data-theme");
    } else {
      root.setAttribute("data-theme", nextPresetAttr);
    }
  };

  const start = (document as DocumentWithViewTransitions).startViewTransition;
  if (typeof start === "function") {
    start.call(document, commit);
  } else {
    commit();
  }
}

export function subscribeSystem(handler: () => void): () => void {
  if (typeof window === "undefined" || typeof window.matchMedia !== "function")
    return () => {};
  const mql = window.matchMedia("(prefers-color-scheme: dark)");
  mql.addEventListener("change", handler);
  return () => mql.removeEventListener("change", handler);
}

export function isThemeMode(value: unknown): value is ThemeMode {
  return typeof value === "string" && THEME_MODES.has(value as ThemeMode);
}

export function isThemePreset(value: unknown): value is ThemePreset {
  return typeof value === "string" && THEME_PRESETS.has(value as ThemePreset);
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

export function writeStoredMode(mode: ThemeMode): void {
  if (typeof window === "undefined") return;
  try {
    window.localStorage.setItem(LOCAL_STORAGE_KEY, mode);
  } catch {
    // localStorage may be unavailable in private mode / quota exceeded —
    // boot cache is best-effort; SQLite is canonical.
  }
}

export function readStoredPreset(): ThemePreset {
  if (typeof window === "undefined") return "default";
  try {
    const raw = window.localStorage.getItem(LOCAL_STORAGE_PRESET_KEY);
    return isThemePreset(raw) ? raw : "default";
  } catch {
    return "default";
  }
}

export function writeStoredPreset(preset: ThemePreset): void {
  if (typeof window === "undefined") return;
  try {
    window.localStorage.setItem(LOCAL_STORAGE_PRESET_KEY, preset);
  } catch {
    // Best-effort boot cache; SQLite is canonical.
  }
}
