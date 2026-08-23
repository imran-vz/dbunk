import { beforeEach, describe, expect, it } from "vitest";

import { useAppStore } from "@/lib/store";
import { consoleSeverityForNotice } from "@/lib/store/console";

const resetConsole = () =>
  useAppStore.setState({
    consoleEvents: [],
    consoleUnread: 0,
    dockOpen: false,
  });

describe("console slice", () => {
  beforeEach(resetConsole);

  it("appends events and increments unread while the dock is hidden", () => {
    useAppStore.getState().appendConsoleEvent({
      severity: "info",
      source: "query",
      message: "Query on local · 3 rows · 12 ms",
    });
    useAppStore.getState().appendConsoleEvent({
      severity: "warning",
      source: "notice",
      message: "WARNING: something",
    });

    const state = useAppStore.getState();
    expect(state.consoleEvents).toHaveLength(2);
    expect(state.consoleEvents[1].severity).toBe("warning");
    expect(state.consoleUnread).toBe(2);
    // §5.6: the dock never auto-opens on new content.
    expect(state.dockOpen).toBe(false);
  });

  it("does not count events as unread while the dock is open", () => {
    useAppStore.getState().setDockOpen(true);
    useAppStore.getState().appendConsoleEvent({
      severity: "info",
      source: "connection",
      message: "Connected to local",
    });
    expect(useAppStore.getState().consoleUnread).toBe(0);
  });

  it("clears unread when the dock opens", () => {
    useAppStore.getState().appendConsoleEvent({
      severity: "error",
      source: "connection",
      message: "Failed to connect",
    });
    expect(useAppStore.getState().consoleUnread).toBe(1);

    useAppStore.getState().toggleDock();
    expect(useAppStore.getState().dockOpen).toBe(true);
    expect(useAppStore.getState().consoleUnread).toBe(0);
  });

  it("caps the event list", () => {
    for (let index = 0; index < 520; index += 1) {
      useAppStore.getState().appendConsoleEvent({
        severity: "info",
        source: "query",
        message: `event ${index}`,
      });
    }
    const events = useAppStore.getState().consoleEvents;
    expect(events).toHaveLength(500);
    expect(events.at(-1)?.message).toBe("event 519");
    expect(events[0]?.message).toBe("event 20");
  });

  it("clearConsole empties events and unread", () => {
    useAppStore.getState().appendConsoleEvent({
      severity: "info",
      source: "task",
      message: "Exported 10 rows as CSV — a.csv",
    });
    useAppStore.getState().clearConsole();
    expect(useAppStore.getState().consoleEvents).toHaveLength(0);
    expect(useAppStore.getState().consoleUnread).toBe(0);
  });

  it("maps server notice severities onto console levels", () => {
    expect(consoleSeverityForNotice("WARNING")).toBe("warning");
    expect(consoleSeverityForNotice("NOTICE")).toBe("info");
    expect(consoleSeverityForNotice("ERROR")).toBe("error");
    expect(consoleSeverityForNotice("FATAL")).toBe("error");
    expect(consoleSeverityForNotice("debug")).toBe("info");
  });
});
