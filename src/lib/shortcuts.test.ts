import { describe, expect, it } from "vitest";

import {
  dispatchShortcut,
  hasShortcutHandler,
  SHORTCUTS,
  shortcutKeys,
  shortcutLabel,
} from "@/lib/shortcuts";

describe("shortcut registry (§6.1)", () => {
  it("defines every §6.1 binding", () => {
    const ids = new Set(SHORTCUTS.map((shortcut) => shortcut.id));
    for (const required of [
      "command-palette",
      "toggle-navigator",
      "toggle-results",
      "toggle-console",
      "run-statement",
      "run-all",
      "cancel-query",
      "format-sql",
      "commit-staged",
      "new-query-tab",
      "close-tab",
      "mru-tab",
      "tab-by-index",
      "prev-tab",
      "next-tab",
      "settings",
      "go-to-row",
      "inspect-value",
    ]) {
      expect(ids.has(required), required).toBe(true);
    }
  });

  it("serves kbd tokens for menu hints and tooltips", () => {
    expect(shortcutKeys("toggle-results")).toEqual(["mod", "J"]);
    expect(shortcutKeys("toggle-console")).toEqual(["ctrl", "`"]);
    expect(shortcutKeys("unknown-id")).toEqual([]);
    expect(shortcutLabel("go-to-row")).toBe("Go to row");
  });

  it("dispatches only registered handlers", () => {
    expect(hasShortcutHandler("nonexistent")).toBe(false);
    expect(dispatchShortcut("nonexistent")).toBe(false);
  });
});
