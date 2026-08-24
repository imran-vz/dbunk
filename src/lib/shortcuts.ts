/**
 * Central shortcut registry (DESIGN-SYSTEM §6.1, P7) — the single
 * source for binding definitions. Palette rows, menu hints, and
 * tooltips all read from here instead of hardcoding key strings.
 *
 * Definitions are static; *handlers* are registered by whichever
 * surface owns the action while it is mounted (`useShortcutHandler`).
 * Some bindings stay physically bound where they must live (Monaco
 * actions, the grid's focused-cell keymap) — their definitions are
 * still listed here so the palette and tooltips can show them, and
 * they gain a palette-invokable handler when their surface is mounted.
 */

import { useEffect } from "react";

export type ShortcutGroup = "General" | "Tabs" | "Panels" | "Query" | "Grid";

export type ShortcutDef = {
  id: string;
  /** Kbd tokens (`["mod", "K"]`) — platform-aware via the Kbd renderer. */
  keys: ReadonlyArray<string>;
  label: string;
  group: ShortcutGroup;
};

export const SHORTCUTS: ReadonlyArray<ShortcutDef> = [
  {
    id: "command-palette",
    keys: ["mod", "K"],
    label: "Command palette",
    group: "General",
  },
  {
    id: "settings",
    keys: ["mod", ","],
    label: "Open settings",
    group: "General",
  },
  {
    id: "new-query-tab",
    keys: ["mod", "T"],
    label: "New query tab",
    group: "Tabs",
  },
  { id: "close-tab", keys: ["mod", "W"], label: "Close tab", group: "Tabs" },
  {
    id: "mru-tab",
    keys: ["ctrl", "Tab"],
    label: "Switch to recent tab",
    group: "Tabs",
  },
  {
    id: "prev-tab",
    keys: ["mod", "shift", "["],
    label: "Previous tab",
    group: "Tabs",
  },
  {
    id: "next-tab",
    keys: ["mod", "shift", "]"],
    label: "Next tab",
    group: "Tabs",
  },
  {
    id: "tab-by-index",
    keys: ["mod", "1–9"],
    label: "Go to tab 1–9",
    group: "Tabs",
  },
  {
    id: "toggle-navigator",
    keys: ["mod", "B"],
    label: "Toggle navigator",
    group: "Panels",
  },
  {
    id: "toggle-results",
    keys: ["mod", "J"],
    label: "Toggle results pane",
    group: "Panels",
  },
  {
    id: "toggle-console",
    keys: ["ctrl", "`"],
    label: "Toggle console",
    group: "Panels",
  },
  {
    id: "focus-navigator-filter",
    keys: [],
    label: "Filter tables…",
    group: "Panels",
  },
  {
    id: "run-statement",
    keys: ["mod", "enter"],
    label: "Run statement at caret",
    group: "Query",
  },
  {
    id: "run-all",
    keys: ["mod", "shift", "enter"],
    label: "Run all",
    group: "Query",
  },
  {
    id: "cancel-query",
    keys: ["mod", "."],
    label: "Cancel running query",
    group: "Query",
  },
  {
    id: "format-sql",
    keys: ["mod", "shift", "F"],
    label: "Format SQL",
    group: "Query",
  },
  {
    id: "commit-staged",
    keys: ["mod", "S"],
    label: "Review & commit staged changes",
    group: "Query",
  },
  { id: "go-to-row", keys: ["mod", "G"], label: "Go to row", group: "Grid" },
  {
    id: "inspect-value",
    keys: ["Space"],
    label: "Inspect cell value",
    group: "Grid",
  },
  {
    id: "copy-selection",
    keys: ["mod", "C"],
    label: "Copy grid selection",
    group: "Grid",
  },
  {
    id: "select-all-cells",
    keys: ["mod", "A"],
    label: "Select all cells",
    group: "Grid",
  },
  {
    id: "clone-row",
    keys: ["mod", "D"],
    label: "Clone selected row",
    group: "Grid",
  },
  {
    id: "delete-rows",
    keys: ["Del"],
    label: "Delete selected rows",
    group: "Grid",
  },
];

const byId = new Map(SHORTCUTS.map((shortcut) => [shortcut.id, shortcut]));

/** Kbd tokens for a binding — menu hints and tooltips read this. */
export function shortcutKeys(id: string): ReadonlyArray<string> {
  return byId.get(id)?.keys ?? [];
}

export function shortcutLabel(id: string): string {
  return byId.get(id)?.label ?? id;
}

// ---------------------------------------------------------------------------
// Handler registry — surfaces attach the action while mounted
// ---------------------------------------------------------------------------

const handlers = new Map<string, () => void>();
const registryListeners = new Set<() => void>();
let registryVersion = 0;

const bumpRegistry = () => {
  registryVersion += 1;
  for (const listener of registryListeners) listener();
};

export const subscribeShortcutRegistry = (listener: () => void) => {
  registryListeners.add(listener);
  return () => registryListeners.delete(listener);
};

export const getShortcutRegistryVersion = () => registryVersion;

export function hasShortcutHandler(id: string): boolean {
  return handlers.has(id);
}

/** Invoke a registered handler (palette rows route through this). */
export function dispatchShortcut(id: string): boolean {
  const handler = handlers.get(id);
  if (!handler) return false;
  handler();
  return true;
}

/**
 * Attach a handler for a shortcut id while the owning surface is
 * mounted. Pass `null` to deregister without unmounting.
 */
export function useShortcutHandler(
  id: string,
  handler: (() => void) | null,
): void {
  useEffect(() => {
    if (!handler) return;
    handlers.set(id, handler);
    bumpRegistry();
    return () => {
      if (handlers.get(id) === handler) {
        handlers.delete(id);
        bumpRegistry();
      }
    };
  }, [id, handler]);
}
