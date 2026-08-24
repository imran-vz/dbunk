/* oxlint-disable anti-slop/no-module-mocking, anti-slop/no-unknown-parameters -- Test fixtures use controlled mocks to exercise otherwise inaccessible boundaries. */
// @vitest-environment jsdom
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("@/lib/tauri", () => ({
  isTauri: vi.fn(() => true),
  tauriInvoke: vi.fn(),
  errorToMessage: (error: unknown) =>
    error instanceof Error ? error.message : String(error),
}));

import { DatabaseNavigator } from "@/components/workbench/database-navigator";
import { useAppStore } from "@/lib/store";

const initialStoreState = useAppStore.getState();

beforeEach(() => {
  useAppStore.setState(initialStoreState, true);
  useAppStore.setState({
    activeConnectionId: "conn-1",
    expandedSchemas: [],
  });
});

afterEach(() => {
  cleanup();
  useAppStore.setState(initialStoreState, true);
});

const schemas = [
  { name: "public", tables: ["users", "orders"] },
  { name: "analytics", tables: ["events"] },
];

function renderNavigator(onOpenTable = vi.fn()) {
  render(
    <DatabaseNavigator
      connectionId="conn-1"
      schemas={schemas}
      activeTableKey={null}
      onOpenTable={onOpenTable}
    />,
  );
  return onOpenTable;
}

describe("DatabaseNavigator", () => {
  it("expands a schema on click and shows its tables", () => {
    renderNavigator();

    expect(screen.queryByRole("treeitem", { name: /users/ })).toBeNull();

    fireEvent.click(screen.getByRole("treeitem", { name: /public/ }));

    expect(screen.getByRole("treeitem", { name: /users/ })).toBeTruthy();
    expect(screen.getByRole("treeitem", { name: /orders/ })).toBeTruthy();
    expect(screen.queryByRole("treeitem", { name: /events/ })).toBeNull();
  });

  it("collapses an expanded schema on a second click", () => {
    renderNavigator();

    const schemaRow = screen.getByRole("treeitem", { name: /public/ });
    fireEvent.click(schemaRow);
    expect(screen.getByRole("treeitem", { name: /users/ })).toBeTruthy();

    fireEvent.click(schemaRow);
    expect(screen.queryByRole("treeitem", { name: /users/ })).toBeNull();
  });

  it("opens a table from an expanded schema", () => {
    const onOpenTable = renderNavigator();

    fireEvent.click(screen.getByRole("treeitem", { name: /public/ }));
    fireEvent.click(screen.getByRole("treeitem", { name: /users/ }));

    expect(onOpenTable).toHaveBeenCalledWith("public", "users");
  });
});

describe("DatabaseNavigator keyboard (§5.5)", () => {
  it("navigates with arrows, expands with ArrowRight, opens with Enter", () => {
    const onOpenTable = renderNavigator();
    const schemaRow = screen.getByRole("treeitem", { name: /public/ });
    schemaRow.focus();

    const tree = screen.getByRole("tree");
    fireEvent.keyDown(tree, { key: "ArrowRight" });
    expect(screen.getByRole("treeitem", { name: /users/ })).toBeTruthy();

    fireEvent.keyDown(tree, { key: "ArrowDown" });
    fireEvent.keyDown(tree, { key: "Enter" });
    expect(onOpenTable).toHaveBeenCalledWith("public", "users");
  });

  it("jumps by type-ahead", () => {
    const onOpenTable = renderNavigator();
    fireEvent.click(screen.getByRole("treeitem", { name: /public/ }));

    const tree = screen.getByRole("tree");
    fireEvent.keyDown(tree, { key: "o" });
    fireEvent.keyDown(tree, { key: "Enter" });
    expect(onOpenTable).toHaveBeenCalledWith("public", "orders");
  });

  it("uses roving tabindex — one tab stop", () => {
    renderNavigator();
    const items = screen.getAllByRole("treeitem");
    const tabbable = items.filter((el) => el.getAttribute("tabindex") === "0");
    expect(tabbable).toHaveLength(1);
  });
});
