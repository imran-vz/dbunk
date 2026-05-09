import { act, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("@/lib/tauri", () => ({
  isTauri: vi.fn(() => true),
  tauriInvoke: vi.fn(),
}));

vi.mock("reactflow", () => ({
  __esModule: true,
  default: () => null,
  Background: () => null,
  Controls: () => null,
  MiniMap: () => null,
}));

import { TableEditorPanel } from "@/components/table-editor-panel";
import {
  type TableDataState,
  type TableLoadStatus,
  tableDataKey,
  useAppStore,
  type WorkspaceTab,
} from "@/lib/store";
import { tauriInvoke } from "@/lib/tauri";

const mockedInvoke = vi.mocked(tauriInvoke);

const tableTab: WorkspaceTab = {
  id: "tab-1",
  kind: "table",
  label: "users",
  connectionId: "conn-1",
  schema: "public",
  table: "users",
};

const initialStoreState = useAppStore.getState();

const seed = (
  data: TableDataState,
  status: TableLoadStatus = { state: "success" },
) => {
  const key = tableDataKey(data.connectionId, data.schema, data.table);
  useAppStore.setState({
    activeConnectionId: data.connectionId,
    tableData: { [key]: data },
    tableLoadStatus: { [data.table]: status },
  });
  return key;
};

beforeEach(() => {
  useAppStore.setState(initialStoreState, true);
  mockedInvoke.mockReset();
  // The panel kicks off a fetch on mount. Default to a never-resolving
  // promise so seeded tableData/tableLoadStatus survive in tests that
  // don't care about the mount fetch. Status will be "loading" but the
  // tableData stays as seeded.
  mockedInvoke.mockImplementation(() => new Promise(() => {}));
});

afterEach(() => {
  useAppStore.setState(initialStoreState, true);
  mockedInvoke.mockReset();
});

// After render the panel's mount effect calls loadTableData and sets
// status to "loading" (we keep that pending), which would otherwise
// disable navigation buttons in our pagination tests. Override the
// status back to success so the buttons reflect the seeded data.
const settleStatus = (table: string) => {
  act(() => {
    useAppStore.setState((state) => ({
      tableLoadStatus: {
        ...state.tableLoadStatus,
        [table]: { state: "success" },
      },
    }));
  });
};

describe("TableEditorPanel pagination", () => {
  it("disables Prev on page 1", () => {
    seed({
      connectionId: "conn-1",
      schema: "public",
      table: "users",
      columns: ["id"],
      rows: [["1"]],
      page: 1,
      pageSize: 100,
      totalRows: 250,
      runtimeMs: 5,
    });

    render(<TableEditorPanel tab={tableTab} />);
    settleStatus("users");

    const prev = screen.getByLabelText("Previous page") as HTMLButtonElement;
    expect(prev.disabled).toBe(true);
  });

  it("clicking Next requests the next page", () => {
    seed({
      connectionId: "conn-1",
      schema: "public",
      table: "users",
      columns: ["id"],
      rows: [["1"]],
      page: 1,
      pageSize: 100,
      totalRows: 250,
      runtimeMs: 5,
    });

    render(<TableEditorPanel tab={tableTab} />);
    settleStatus("users");
    mockedInvoke.mockClear();

    fireEvent.click(screen.getByLabelText("Next page"));

    expect(mockedInvoke).toHaveBeenCalledWith("load_table_data", {
      payload: {
        connectionId: "conn-1",
        schema: "public",
        table: "users",
        page: 2,
        pageSize: 100,
      },
    });
  });

  it("disables Next on the last page when totalRows is known", () => {
    seed({
      connectionId: "conn-1",
      schema: "public",
      table: "users",
      columns: ["id"],
      rows: [["1"]],
      page: 3,
      pageSize: 100,
      totalRows: 250,
      runtimeMs: 5,
    });

    render(<TableEditorPanel tab={tableTab} />);
    settleStatus("users");

    const next = screen.getByLabelText("Next page") as HTMLButtonElement;
    expect(next.disabled).toBe(true);
  });

  it("clicking Refresh re-fetches with the stored page and pageSize", () => {
    seed({
      connectionId: "conn-1",
      schema: "public",
      table: "users",
      columns: ["id"],
      rows: [["1"]],
      page: 2,
      pageSize: 50,
      totalRows: 250,
      runtimeMs: 5,
    });

    render(<TableEditorPanel tab={tableTab} />);
    settleStatus("users");
    mockedInvoke.mockClear();

    fireEvent.click(screen.getByRole("button", { name: /refresh/i }));

    expect(mockedInvoke).toHaveBeenCalledWith("load_table_data", {
      payload: {
        connectionId: "conn-1",
        schema: "public",
        table: "users",
        page: 2,
        pageSize: 50,
      },
    });
  });
});

describe("TableEditorPanel status banners", () => {
  it("shows a loading indicator when tableLoadStatus is loading", () => {
    seed(
      {
        connectionId: "conn-1",
        schema: "public",
        table: "users",
        columns: [],
        rows: [],
        page: 1,
        pageSize: 100,
        runtimeMs: 0,
      },
      { state: "loading" },
    );

    render(<TableEditorPanel tab={tableTab} />);

    expect(screen.getByTestId("table-loading")).toBeTruthy();
  });

  it("shows an error banner when tableLoadStatus is error", async () => {
    // Make the on-mount loadTableData fail so the rendered status ends up
    // as `error` even after the panel's mount effect runs.
    mockedInvoke.mockReset();
    mockedInvoke.mockRejectedValue(new Error("boom"));

    seed(
      {
        connectionId: "conn-1",
        schema: "public",
        table: "users",
        columns: [],
        rows: [],
        page: 1,
        pageSize: 100,
        runtimeMs: 0,
      },
      { state: "error", error: "boom" },
    );

    render(<TableEditorPanel tab={tableTab} />);

    // Allow the mount-effect's rejection to flush.
    await act(async () => {
      await Promise.resolve();
    });

    expect(screen.getByTestId("table-error")).toBeTruthy();
    expect(screen.getByRole("alert").textContent).toContain("boom");
  });
});
