/* oxlint-disable anti-slop/no-module-mocking -- the confirm service is a UI boundary; tests stub it like window.confirm. */
// @vitest-environment jsdom
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const { mockedRequestConfirm } = vi.hoisted(() => ({
  mockedRequestConfirm: vi.fn(() => Promise.resolve(true)),
}));
vi.mock("@/lib/confirm", () => ({
  requestConfirm: mockedRequestConfirm,
  requestPrompt: vi.fn(() => Promise.resolve(null)),
}));

import { TabShortcuts } from "@/components/workbench/tab-shortcuts";
import { useAppStore, type WorkspaceTab } from "@/lib/store";

const initialStoreState = useAppStore.getState();

const tab = (id: string, label: string, pinned = false): WorkspaceTab => ({
  id,
  kind: "query",
  label,
  connectionId: "conn-1",
  schema: "public",
  query: "select 1;",
  pinned,
});

beforeEach(() => {
  useAppStore.setState(initialStoreState, true);
  useAppStore.setState({
    workspaceTabs: [tab("a", "one"), tab("b", "two"), tab("c", "three")],
    activeTabId: "a",
  });
});

afterEach(() => {
  cleanup();
  useAppStore.setState(initialStoreState, true);
});

describe("TabShortcuts (§6.1)", () => {
  it("creates a query tab on Cmd+T", () => {
    const spy = vi.spyOn(useAppStore.getState(), "createNewQueryTab");
    render(<TabShortcuts />);
    fireEvent.keyDown(window, { key: "t", metaKey: true });
    expect(spy).toHaveBeenCalledTimes(1);
  });

  it("activates a tab by position with Cmd+1..9 (9 = last)", () => {
    render(<TabShortcuts />);
    fireEvent.keyDown(window, { key: "2", metaKey: true });
    expect(useAppStore.getState().activeTabId).toBe("b");
    fireEvent.keyDown(window, { key: "9", metaKey: true });
    expect(useAppStore.getState().activeTabId).toBe("c");
  });

  it("steps tabs with Cmd+Shift+[ and ] in visual order", () => {
    render(<TabShortcuts />);
    fireEvent.keyDown(window, {
      key: "}",
      code: "BracketRight",
      metaKey: true,
      shiftKey: true,
    });
    expect(useAppStore.getState().activeTabId).toBe("b");
    fireEvent.keyDown(window, {
      key: "{",
      code: "BracketLeft",
      metaKey: true,
      shiftKey: true,
    });
    expect(useAppStore.getState().activeTabId).toBe("a");
  });

  it("shows the MRU switcher on Ctrl+Tab and commits on Ctrl release", () => {
    const { rerender } = render(<TabShortcuts />);
    // Build MRU: a (initial) → b → a.
    useAppStore.setState({ activeTabId: "b" });
    rerender(<TabShortcuts />);
    useAppStore.setState({ activeTabId: "a" });
    rerender(<TabShortcuts />);

    fireEvent.keyDown(window, { key: "Tab", ctrlKey: true });
    expect(screen.getByTestId("mru-tab-switcher")).toBeTruthy();
    // Most recent other tab is highlighted first.
    fireEvent.keyUp(window, { key: "Control" });
    expect(useAppStore.getState().activeTabId).toBe("b");
    expect(screen.queryByTestId("mru-tab-switcher")).toBeNull();
  });

  it("does not close pinned tabs on Cmd+W", async () => {
    useAppStore.setState({
      workspaceTabs: [tab("a", "one", true), tab("b", "two")],
      activeTabId: "a",
    });
    render(<TabShortcuts />);
    fireEvent.keyDown(window, { key: "w", metaKey: true });
    await Promise.resolve();
    expect(useAppStore.getState().workspaceTabs).toHaveLength(2);
  });
});
