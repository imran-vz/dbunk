/* oxlint-disable anti-slop/no-runtime-typeof, anti-slop/no-unknown-parameters -- localStorage and DOM access are external boundaries validated here before use. */
/**
 * UI density — an explicit user setting with three modes
 * (DESIGN-SYSTEM.md §2.3). Density shifts metric CSS variables only
 * (control heights, row heights, toolbar heights, panel padding);
 * typography never changes with density. There is no width-based
 * automatic density switching.
 *
 * Persistence: localStorage for now (applied pre-paint by the boot
 * script in __root.tsx). Phase 8 migrates UI preferences into the
 * app's SQLite store; this module is the single read/write point so
 * that migration touches one file.
 */
import { useSyncExternalStore } from "react";

export type Density = "compact" | "default" | "comfortable";

export const DENSITY_STORAGE_KEY = "dbunk.density";

const DENSITIES = new Set<Density>(["compact", "default", "comfortable"]);

export function isDensity(value: unknown): value is Density {
  // SAFETY: Set.has only returns true when the string is one of the
  // three Density literals the set was built from.
  return typeof value === "string" && DENSITIES.has(value as Density);
}

export function loadDensity(): Density {
  if (typeof window === "undefined") return "default";
  try {
    const stored = window.localStorage.getItem(DENSITY_STORAGE_KEY);
    return isDensity(stored) ? stored : "default";
  } catch {
    return "default";
  }
}

export function applyDensity(density: Density): void {
  if (typeof document === "undefined") return;
  const root = document.documentElement;
  if (density === "default") {
    delete root.dataset.density;
  } else {
    root.dataset.density = density;
  }
}

const listeners = new Set<() => void>();
let current: Density | null = null;

function snapshot(): Density {
  if (current === null) current = loadDensity();
  return current;
}

export function setDensity(density: Density): void {
  current = density;
  applyDensity(density);
  try {
    if (typeof window !== "undefined") {
      window.localStorage.setItem(DENSITY_STORAGE_KEY, density);
    }
  } catch {
    // Persistence must never break the setting taking effect.
  }
  for (const listener of listeners) listener();
}

function subscribe(listener: () => void): () => void {
  listeners.add(listener);
  return () => listeners.delete(listener);
}

/** React hook for the current density (settings UI). */
export function useDensity(): Density {
  // SAFETY: "default" is a member of the Density union; the assertion
  // only widens the literal for the server-snapshot callback signature.
  return useSyncExternalStore(subscribe, snapshot, () => "default" as Density);
}
