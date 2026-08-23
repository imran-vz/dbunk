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

    expect(screen.queryByRole("button", { name: /users/ })).toBeNull();

    fireEvent.click(screen.getByRole("button", { name: /public/ }));

    expect(screen.getByRole("button", { name: /users/ })).toBeTruthy();
    expect(screen.getByRole("button", { name: /orders/ })).toBeTruthy();
    expect(screen.queryByRole("button", { name: /events/ })).toBeNull();
  });

  it("collapses an expanded schema on a second click", () => {
    renderNavigator();

    const schemaRow = screen.getByRole("button", { name: /public/ });
    fireEvent.click(schemaRow);
    expect(screen.getByRole("button", { name: /users/ })).toBeTruthy();

    fireEvent.click(schemaRow);
    expect(screen.queryByRole("button", { name: /users/ })).toBeNull();
  });

  it("opens a table from an expanded schema", () => {
    const onOpenTable = renderNavigator();

    fireEvent.click(screen.getByRole("button", { name: /public/ }));
    fireEvent.click(screen.getByRole("button", { name: /users/ }));

    expect(onOpenTable).toHaveBeenCalledWith("public", "users");
  });
});
