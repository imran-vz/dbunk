/* oxlint-disable anti-slop/no-chained-type-assertions anti-slop/no-module-mocking anti-slop/no-unknown-parameters anti-slop/require-safety-comment-for-type-assertion -- Test fixtures use controlled mocks and assertions to exercise otherwise inaccessible boundaries. */
// @vitest-environment jsdom
import {
  act,
  cleanup,
  fireEvent,
  render,
  screen,
} from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("@/lib/tauri", () => ({
  isTauri: vi.fn(() => false),
  tauriInvoke: vi.fn(),
  errorToMessage: (error: unknown) =>
    error instanceof Error ? error.message : String(error),
}));

const resizeObserverInstances: ResizeObserverMock[] = [];

class ResizeObserverMock implements ResizeObserver {
  callback: ResizeObserverCallback;
  observed: Element[] = [];
  disconnected = false;
  constructor(callback: ResizeObserverCallback) {
    this.callback = callback;
    resizeObserverInstances.push(this);
  }
  observe(target: Element) {
    this.observed.push(target);
  }
  unobserve() {}
  disconnect() {
    this.disconnected = true;
  }
  trigger() {
    this.callback([], this as unknown as ResizeObserver);
  }
}

vi.stubGlobal("ResizeObserver", ResizeObserverMock);

import { WorkspaceTabs } from "@/components/workspace-tabs";
import { useAppStore, type WorkspaceTab } from "@/lib/store";

const initialStoreState = useAppStore.getState();

const overflowingTabs: WorkspaceTab[] = Array.from(
  { length: 8 },
  (_, index): WorkspaceTab => {
    const isTable = index % 3 === 1;
    return {
      id: `tab-${index + 1}`,
      kind: isTable ? "table" : "query",
      label: `${isTable ? "Table" : "Query"} ${index + 1}`,
      connectionId: "conn-1",
      schema: "public",
      isDirty: index === 0,
      ...(isTable ? { table: `t_${index}` } : { query: "select 1;" }),
    };
  },
);

function setScrollMetrics(
  element: HTMLElement,
  {
    clientWidth,
    scrollLeft,
    scrollWidth,
  }: { clientWidth: number; scrollLeft: number; scrollWidth: number },
) {
  Object.defineProperty(element, "clientWidth", {
    configurable: true,
    value: clientWidth,
  });
  Object.defineProperty(element, "scrollWidth", {
    configurable: true,
    value: scrollWidth,
  });
  Object.defineProperty(element, "scrollLeft", {
    configurable: true,
    value: scrollLeft,
    writable: true,
  });
}

beforeEach(() => {
  resizeObserverInstances.length = 0;
  useAppStore.setState({ ...initialStoreState }, true);
});

afterEach(() => {
  cleanup();
  useAppStore.setState({ ...initialStoreState }, true);
  vi.clearAllMocks();
});

describe("WorkspaceTabs overflow affordance", () => {
  it.each([
    { name: "left edge", scrollLeft: 0, start: false, end: true },
    { name: "middle", scrollLeft: 300, start: true, end: true },
    { name: "right edge", scrollLeft: 660, start: true, end: false },
  ])("shows correct hints at $name", ({ scrollLeft, start, end }) => {
    useAppStore.setState({
      activeTabId: overflowingTabs[0].id,
      workspaceTabs: overflowingTabs,
    });

    render(<WorkspaceTabs />);
    const scroller = screen.getByTestId("workspace-tabs-scroll");

    setScrollMetrics(scroller, {
      clientWidth: 240,
      scrollLeft,
      scrollWidth: 900,
    });
    fireEvent(window, new Event("resize"));

    expect(!!screen.queryByTestId("workspace-tabs-more-start")).toBe(start);
    expect(!!screen.queryByTestId("workspace-tabs-more-end")).toBe(end);
  });

  it("does not show hints when the tabs fit", () => {
    useAppStore.setState({
      activeTabId: overflowingTabs[0].id,
      workspaceTabs: overflowingTabs.slice(0, 2),
    });

    render(<WorkspaceTabs />);

    const scroller = screen.getByTestId("workspace-tabs-scroll");

    setScrollMetrics(scroller, {
      clientWidth: 900,
      scrollLeft: 0,
      scrollWidth: 900,
    });
    fireEvent(window, new Event("resize"));

    expect(screen.queryByTestId("workspace-tabs-more-start")).toBeNull();
    expect(screen.queryByTestId("workspace-tabs-more-end")).toBeNull();
  });

  it("updates hints on window resize", () => {
    useAppStore.setState({
      activeTabId: overflowingTabs[0].id,
      workspaceTabs: overflowingTabs,
    });

    render(<WorkspaceTabs />);
    const scroller = screen.getByTestId("workspace-tabs-scroll");

    setScrollMetrics(scroller, {
      clientWidth: 900,
      scrollLeft: 0,
      scrollWidth: 900,
    });
    fireEvent(window, new Event("resize"));
    expect(screen.queryByTestId("workspace-tabs-more-end")).toBeNull();

    setScrollMetrics(scroller, {
      clientWidth: 240,
      scrollLeft: 0,
      scrollWidth: 900,
    });
    fireEvent(window, new Event("resize"));
    expect(screen.queryByTestId("workspace-tabs-more-end")).not.toBeNull();
  });

  it("updates hints when the ResizeObserver fires", () => {
    useAppStore.setState({
      activeTabId: overflowingTabs[0].id,
      workspaceTabs: overflowingTabs,
    });

    render(<WorkspaceTabs />);
    const scroller = screen.getByTestId("workspace-tabs-scroll");

    expect(resizeObserverInstances).toHaveLength(1);
    const observer = resizeObserverInstances[0];
    expect(observer.observed).toContain(scroller);

    setScrollMetrics(scroller, {
      clientWidth: 240,
      scrollLeft: 0,
      scrollWidth: 900,
    });
    act(() => {
      observer.trigger();
    });

    expect(screen.queryByTestId("workspace-tabs-more-end")).not.toBeNull();
  });

  it("cleans up subscriptions on unmount", () => {
    const removeSpy = vi.spyOn(window, "removeEventListener");
    useAppStore.setState({
      activeTabId: overflowingTabs[0].id,
      workspaceTabs: overflowingTabs,
    });

    const { unmount } = render(<WorkspaceTabs />);
    const observer = resizeObserverInstances[0];

    unmount();

    expect(removeSpy).toHaveBeenCalledWith("resize", expect.any(Function));
    expect(observer.disconnected).toBe(true);
  });

  it("renders a dirty indicator only on dirty tabs", () => {
    useAppStore.setState({
      activeTabId: overflowingTabs[0].id,
      workspaceTabs: overflowingTabs,
    });

    render(<WorkspaceTabs />);

    const dirtyTabs = overflowingTabs.filter((tab) => tab.isDirty);
    expect(dirtyTabs.length).toBeGreaterThan(0);
    for (const tab of dirtyTabs) {
      expect(
        screen.queryByTestId(`workspace-tab-dirty-${tab.id}`),
      ).not.toBeNull();
    }
    const cleanTabs = overflowingTabs.filter((tab) => !tab.isDirty);
    for (const tab of cleanTabs) {
      expect(screen.queryByTestId(`workspace-tab-dirty-${tab.id}`)).toBeNull();
    }
  });
});
