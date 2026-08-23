// @vitest-environment jsdom
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { ObjectTabRow } from "@/components/workbench/object-tab-row";
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

const tabOrder = () => useAppStore.getState().workspaceTabs.map((t) => t.id);
const tabEl = (label: string) => screen.getByRole("tab", { name: label });

// jsdom's DragEvent constructor drops coordinates; a MouseEvent with the
// dragover type carries clientX through to the React handler.
const dragOverAt = (el: Element, clientX: number) => {
  fireEvent(
    el,
    new MouseEvent("dragover", { bubbles: true, cancelable: true, clientX }),
  );
};

beforeEach(() => {
  useAppStore.setState(initialStoreState, true);
  // jsdom has no scrollIntoView (the strip scrolls the active tab into view).
  Element.prototype.scrollIntoView = vi.fn();
});

afterEach(() => {
  cleanup();
  useAppStore.setState(initialStoreState, true);
});

// jsdom rects are all-zero, so the target midpoint is at x=0: positive
// clientX is "past the midpoint" (rightward drag), negative is "before
// it" (leftward drag).
describe("ObjectTabRow drag reorder", () => {
  it("reorders when dragging right past the target midpoint", () => {
    useAppStore.setState({
      workspaceTabs: [tab("a", "one"), tab("b", "two"), tab("c", "three")],
      activeTabId: "a",
    });
    render(<ObjectTabRow />);

    fireEvent.dragStart(tabEl("one"));
    dragOverAt(tabEl("three"), 10);
    fireEvent.dragEnd(tabEl("one"));

    expect(tabOrder()).toEqual(["b", "c", "a"]);
  });

  it("does not reorder before the pointer crosses the midpoint", () => {
    useAppStore.setState({
      workspaceTabs: [tab("a", "one"), tab("b", "two"), tab("c", "three")],
      activeTabId: "a",
    });
    render(<ObjectTabRow />);

    // Rightward drag still on the near (left) side of the target.
    fireEvent.dragStart(tabEl("one"));
    dragOverAt(tabEl("three"), -10);
    expect(tabOrder()).toEqual(["a", "b", "c"]);
  });

  it("does not oscillate on repeated dragover events over the swapped tab", () => {
    useAppStore.setState({
      workspaceTabs: [tab("a", "one"), tab("b", "two"), tab("c", "three")],
      activeTabId: "a",
    });
    render(<ObjectTabRow />);

    fireEvent.dragStart(tabEl("one"));
    dragOverAt(tabEl("three"), 10);
    expect(tabOrder()).toEqual(["b", "c", "a"]);

    // dragover keeps firing at the same pointer position; the previous
    // target is now left of the source, so no swap-back may occur.
    dragOverAt(tabEl("three"), 10);
    dragOverAt(tabEl("three"), 10);
    expect(tabOrder()).toEqual(["b", "c", "a"]);
  });

  it("reorders leftward drags once past the midpoint", () => {
    useAppStore.setState({
      workspaceTabs: [tab("a", "one"), tab("b", "two"), tab("c", "three")],
      activeTabId: "c",
    });
    render(<ObjectTabRow />);

    fireEvent.dragStart(tabEl("three"));
    dragOverAt(tabEl("one"), -10);
    expect(tabOrder()).toEqual(["c", "a", "b"]);
  });

  it("never drags across the pinned boundary", () => {
    useAppStore.setState({
      workspaceTabs: [tab("a", "one"), tab("p", "pinned-tab", true)],
      activeTabId: "a",
    });
    render(<ObjectTabRow />);

    // Display order is pinned-first; a raw-index splice here would flip
    // the stored order invisibly on every dragover event.
    fireEvent.dragStart(tabEl("one"));
    dragOverAt(tabEl("pinned-tab"), 10);
    dragOverAt(tabEl("pinned-tab"), -10);
    expect(tabOrder()).toEqual(["a", "p"]);
  });
});
