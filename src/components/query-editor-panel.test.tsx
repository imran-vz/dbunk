import { act, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("@monaco-editor/react", () => ({
  __esModule: true,
  default: () => <div data-testid="monaco-editor" />,
}));

vi.mock("@/lib/tauri", () => ({
  isTauri: vi.fn(() => true),
  tauriInvoke: vi.fn(),
}));

import { QueryEditorPanel } from "@/components/query-editor-panel";
import { type QueryStatus, useAppStore, type WorkspaceTab } from "@/lib/store";

const initialStoreState = useAppStore.getState();

const queryTab: WorkspaceTab = {
  id: "tab-1",
  kind: "query",
  label: "query_1.sql",
  connectionId: "conn-1",
  schema: "public",
  query: "select 1;",
};

const seedStatus = (status: QueryStatus) => {
  useAppStore.setState({
    workspaceTabs: [queryTab],
    activeConnectionId: "conn-1",
    activeTabId: queryTab.id,
    queryStatus: { [queryTab.id]: status },
  });
};

beforeEach(() => {
  useAppStore.setState(initialStoreState, true);
});

afterEach(() => {
  useAppStore.setState(initialStoreState, true);
});

describe("QueryEditorPanel feedback", () => {
  it("disables the Run button while a query is running", () => {
    seedStatus({ state: "running" });

    render(<QueryEditorPanel tab={queryTab} isClient />);

    const runButton = screen.getByRole("button", {
      name: /running/i,
    }) as HTMLButtonElement;
    expect(runButton.disabled).toBe(true);
  });

  it("shows the error message in a banner when the query fails", () => {
    seedStatus({ state: "error", error: "syntax error at column 5" });

    render(<QueryEditorPanel tab={queryTab} isClient />);

    expect(screen.getByRole("alert").textContent).toContain(
      "syntax error at column 5",
    );
  });

  it("renders Run as the default label when idle", () => {
    useAppStore.setState({
      workspaceTabs: [queryTab],
      activeConnectionId: "conn-1",
      activeTabId: queryTab.id,
      queryStatus: {},
    });

    render(<QueryEditorPanel tab={queryTab} isClient />);
    const runButton = screen.getByRole("button", {
      name: /^run$/i,
    }) as HTMLButtonElement;
    expect(runButton.disabled).toBe(false);
  });

  it("does not allow re-running while in flight", async () => {
    seedStatus({ state: "running" });

    render(<QueryEditorPanel tab={queryTab} isClient />);

    const runButton = screen.getByRole("button", { name: /running/i });

    await act(async () => {
      runButton.click();
    });

    // Disabled buttons shouldn't trigger handlers; the status should remain
    // running because no invoke was called.
    expect(useAppStore.getState().queryStatus[queryTab.id].state).toBe(
      "running",
    );
  });
});
