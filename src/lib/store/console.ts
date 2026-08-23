/**
 * Console slice — the global console dock's event stream (DESIGN-SYSTEM
 * §5.6). App-wide streams land here: connection lifecycle events, server
 * notices, background task/export progress, and the cross-tab query log.
 *
 * The dock never auto-opens: events appended while it is hidden increment
 * `consoleUnread`, surfaced as a status-bar badge. Opening the dock (via
 * `` Ctrl+` `` or the badge) clears the unread count.
 */

import type { StateCreator } from "zustand";

import type { AppStoreState } from "./types";

export type ConsoleSeverity = "info" | "warning" | "error";
export type ConsoleSource = "connection" | "query" | "notice" | "task";

export type ConsoleEvent = {
  id: string;
  /** ISO timestamp of when the event was appended. */
  at: string;
  severity: ConsoleSeverity;
  source: ConsoleSource;
  message: string;
  /** Optional payload rendered in mono (SQL text, raw notice detail). */
  detail?: string;
  connectionId?: string;
};

export type AppendConsoleEventInput = Omit<ConsoleEvent, "id" | "at">;

export type ConsoleSlice = {
  consoleEvents: ConsoleEvent[];
  /** Events appended while the dock was hidden. */
  consoleUnread: number;
  dockOpen: boolean;
  appendConsoleEvent: (event: AppendConsoleEventInput) => void;
  clearConsole: () => void;
  setDockOpen: (open: boolean) => void;
  toggleDock: () => void;
};

/** Ring cap — the console is a tail, not an archive. */
const CONSOLE_EVENT_CAP = 500;

/** Maps a server notice severity string onto the console's three levels. */
export function consoleSeverityForNotice(severity: string): ConsoleSeverity {
  const normalized = severity.toUpperCase();
  if (
    normalized.includes("ERROR") ||
    normalized === "FATAL" ||
    normalized === "PANIC"
  ) {
    return "error";
  }
  if (normalized.includes("WARN")) return "warning";
  return "info";
}

export const createConsoleSlice: StateCreator<
  AppStoreState,
  [],
  [],
  ConsoleSlice
> = (set) => ({
  consoleEvents: [],
  consoleUnread: 0,
  dockOpen: false,

  appendConsoleEvent: (event) =>
    set((state) => ({
      consoleEvents: [
        ...state.consoleEvents.slice(-(CONSOLE_EVENT_CAP - 1)),
        {
          ...event,
          id: crypto.randomUUID(),
          at: new Date().toISOString(),
        },
      ],
      // §5.6: never auto-open — new content while hidden increments the
      // status-bar badge instead.
      consoleUnread: state.dockOpen ? 0 : state.consoleUnread + 1,
    })),

  clearConsole: () => set({ consoleEvents: [], consoleUnread: 0 }),

  setDockOpen: (open) =>
    set((state) => ({
      dockOpen: open,
      consoleUnread: open ? 0 : state.consoleUnread,
    })),

  toggleDock: () =>
    set((state) => ({
      dockOpen: !state.dockOpen,
      consoleUnread: state.dockOpen ? state.consoleUnread : 0,
    })),
});
