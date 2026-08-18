/* oxlint-disable anti-slop/no-module-mocking anti-slop/no-unknown-parameters -- Test fixtures use controlled mocks and assertions to exercise otherwise inaccessible boundaries. */
// @vitest-environment jsdom
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("@/lib/tauri", () => ({
  isTauri: vi.fn(() => true),
  tauriInvoke: vi.fn(),
  errorToMessage: (error: unknown) =>
    error instanceof Error ? error.message : String(error),
}));

import { QuerySidebar } from "@/components/query-sidebar";
import {
  type QueryHistoryEntry,
  useAppStore,
  type WorkspaceTab,
} from "@/lib/store";

const queryTab: WorkspaceTab = {
  id: "tab-1",
  kind: "query",
  label: "query_1.sql",
  connectionId: "conn-1",
  schema: "public",
  query: "select 1;",
};

const initialStoreState = useAppStore.getState();

const baseEntry: QueryHistoryEntry = {
  id: "history-1",
  sql: "select * from users;",
  connectionId: "conn-1",
  connectionName: "Local Postgres",
  database: "postgres",
  engine: "PostgreSQL",
  status: "success",
  runtimeMs: 42,
  rowCount: 5,
  startedAt: "2026-05-09T12:00:00.000Z",
};

beforeEach(() => {
  useAppStore.setState(initialStoreState, true);
});

afterEach(() => {
  cleanup();
});

describe("QuerySidebar query history", () => {
  it("renders an entry per history item with SQL preview, connection, and status", () => {
    useAppStore.setState({
      workspaceTabs: [queryTab],
      activeTabId: queryTab.id,
      activeConnectionId: "conn-1",
      queryHistory: [
        baseEntry,
        {
          ...baseEntry,
          id: "history-2",
          status: "error",
          errorMessage: "boom",
        },
      ],
    });

    render(<QuerySidebar tab={queryTab} />);

    const entries = screen.getAllByTestId("query-history-entry");
    expect(entries).toHaveLength(2);
    expect(entries[0]?.textContent).toContain("select * from users");
    expect(entries[0]?.textContent).toContain("Local Postgres");
    expect(entries[0]?.textContent).toContain("OK");
    expect(entries[1]?.textContent).toContain("Error");
  });

  it("shows a placeholder when there is no history", () => {
    useAppStore.setState({
      workspaceTabs: [queryTab],
      activeTabId: queryTab.id,
      activeConnectionId: "conn-1",
      queryHistory: [],
    });

    render(<QuerySidebar tab={queryTab} />);

    expect(screen.getByText(/no recent queries/i)).toBeDefined();
  });

  it("triggers reopenHistoryEntry when an entry is clicked", () => {
    const reopenSpy = vi.fn();
    useAppStore.setState({
      workspaceTabs: [queryTab],
      activeTabId: queryTab.id,
      activeConnectionId: "conn-1",
      queryHistory: [baseEntry],
      reopenHistoryEntry: reopenSpy,
    });

    render(<QuerySidebar tab={queryTab} />);
    fireEvent.click(screen.getByTestId("query-history-entry"));

    expect(reopenSpy).toHaveBeenCalledTimes(1);
    expect(reopenSpy).toHaveBeenCalledWith(baseEntry);
  });
});
