import { act, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("@/lib/tauri", () => ({
  isTauri: vi.fn(() => true),
  tauriInvoke: vi.fn(),
}));

vi.mock("reactflow", () => ({
  __esModule: true,
  default: () => <div data-testid="react-flow" />,
  Background: () => null,
  Controls: () => null,
  MiniMap: () => null,
}));

import { TableEditorPanel } from "@/components/table-editor-panel";
import { useAppStore, type WorkspaceTab } from "@/lib/store";
import { tauriInvoke } from "@/lib/tauri";

const mockedInvoke = tauriInvoke as unknown as ReturnType<typeof vi.fn>;

const tableTab: WorkspaceTab = {
  id: "tab-1",
  kind: "table",
  label: "users",
  connectionId: "conn-1",
  schema: "public",
  table: "users",
};

const initialStoreState = useAppStore.getState();

beforeEach(() => {
  useAppStore.setState(initialStoreState, true);
  mockedInvoke.mockReset();
});

afterEach(() => {
  useAppStore.setState(initialStoreState, true);
});

describe("TableEditorPanel feedback", () => {
  it("shows a loading indicator while the table preview is loading", async () => {
    // Keep the invoke pending so the loading state persists.
    mockedInvoke.mockImplementationOnce(() => new Promise(() => {}));

    useAppStore.setState({
      workspaceTabs: [tableTab],
      activeConnectionId: "conn-1",
      activeTabId: tableTab.id,
    });

    await act(async () => {
      render(<TableEditorPanel tab={tableTab} />);
    });

    expect(screen.getByText(/loading table/i)).toBeTruthy();
  });

  it("shows an error banner with the error message when load fails", async () => {
    mockedInvoke.mockRejectedValueOnce(
      new Error("relation users does not exist"),
    );

    useAppStore.setState({
      workspaceTabs: [tableTab],
      activeConnectionId: "conn-1",
      activeTabId: tableTab.id,
    });

    await act(async () => {
      render(<TableEditorPanel tab={tableTab} />);
    });

    // Allow the rejected promise to settle and re-render.
    await act(async () => {
      await Promise.resolve();
    });

    expect(screen.getByRole("alert").textContent).toContain(
      "relation users does not exist",
    );
  });
});
