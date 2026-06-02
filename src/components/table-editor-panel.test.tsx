import { act, fireEvent, render, screen, within } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("@/lib/tauri", () => ({
  isTauri: vi.fn(() => true),
  tauriInvoke: vi.fn(),
  errorToMessage: (error: unknown) =>
    error instanceof Error ? error.message : String(error),
}));

vi.mock("@xyflow/react", () => ({
  __esModule: true,
  default: () => null,
  Background: () => null,
  Controls: () => null,
  MiniMap: () => null,
  applyNodeChanges: (_changes: unknown, nodes: unknown) => nodes,
}));

vi.mock("@/components/workspace-overview/schema-map-toolbar", () => ({
  SchemaMapToolbar: () => <div data-testid="schema-map-toolbar" />,
  schemaMapExportFilename: () => "schema-map.png",
}));

import {
  TableEditorPanel,
  TableSidebar,
} from "@/components/table-editor-panel";
import {
  type Connection,
  type TableDataState,
  type TableLoadStatus,
  type TableStructure,
  tableDataKey,
  tableStructureKey,
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
    tableLoadStatus: { [key]: status },
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
        [Object.entries(state.tableData).find(
          ([, data]) => data.table === table,
        )?.[0] ?? table]: { state: "success" },
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

describe("TableSidebar schema map fullscreen", () => {
  it("opens and closes the schema map fullscreen overlay", () => {
    render(<TableSidebar tab={tableTab} isClient={false} />);

    fireEvent.click(screen.getByLabelText("Open schema map fullscreen"));

    expect(screen.getByTestId("schema-map-fullscreen")).toBeTruthy();

    fireEvent.click(screen.getByRole("button", { name: /close/i }));

    expect(screen.queryByTestId("schema-map-fullscreen")).toBeNull();
  });

  it("closes the fullscreen schema map with Escape", () => {
    render(<TableSidebar tab={tableTab} isClient={false} />);

    fireEvent.click(screen.getByLabelText("Open schema map fullscreen"));
    fireEvent.keyDown(window, { key: "Escape" });

    expect(screen.queryByTestId("schema-map-fullscreen")).toBeNull();
  });
});

const postgresConnection: Connection = {
  id: "conn-1",
  name: "Local",
  database: "dbunk",
  status: "Connected",
  engine: "PostgreSQL",
  host: "localhost",
  port: 5432,
  user: "postgres",
  password: "",
  role: "admin",
  latency: "10 ms",
  ssl: true,
};

const seedStructure = (structure: TableStructure) => {
  const key = tableStructureKey("conn-1", "public", "users");
  useAppStore.setState((state) => ({
    connections:
      state.connections.length > 0 ? state.connections : [postgresConnection],
    activeConnectionId: "conn-1",
    tableStructure: {
      ...state.tableStructure,
      [key]: structure,
    },
  }));
};

const editableStructure: TableStructure = {
  columns: [
    {
      name: "id",
      dataType: "integer",
      nullable: false,
      defaultValue: null,
      isPrimaryKey: true,
      ordinalPosition: 1,
    },
    {
      name: "email",
      dataType: "text",
      nullable: false,
      defaultValue: null,
      isPrimaryKey: false,
      ordinalPosition: 2,
    },
  ],
  primaryKey: ["id"],
  foreignKeys: [],
  indexes: [],
  constraints: [],
  capabilities: {
    columns: true,
    primaryKey: true,
    foreignKeys: true,
    indexes: true,
    constraints: true,
    canInsertRows: true,
    canUpdateRows: true,
    canDeleteRows: true,
    canAlterSchema: true,
    uniquenessGuarantee: "exact",
  },
};

const readOnlyStructure: TableStructure = {
  ...editableStructure,
  primaryKey: null,
  indexes: [],
  capabilities: {
    ...editableStructure.capabilities,
    canUpdateRows: false,
    canDeleteRows: false,
    uniquenessGuarantee: "best-effort",
  },
};

describe("TableEditorPanel read-only handling", () => {
  it("shows a read-only banner when the structure has no PK or unique non-null index", () => {
    seed({
      connectionId: "conn-1",
      schema: "public",
      table: "users",
      columns: ["id", "email"],
      rows: [["1", "ada@example.com"]],
      page: 1,
      pageSize: 100,
      totalRows: 1,
      runtimeMs: 5,
    });
    seedStructure(readOnlyStructure);

    render(<TableEditorPanel tab={tableTab} />);
    settleStatus("users");

    expect(screen.getByTestId("table-readonly-banner")).toBeTruthy();
  });

  it("does not show the read-only banner when structure has a primary key", () => {
    seed({
      connectionId: "conn-1",
      schema: "public",
      table: "users",
      columns: ["id", "email"],
      rows: [["1", "ada@example.com"]],
      page: 1,
      pageSize: 100,
      totalRows: 1,
      runtimeMs: 5,
    });
    seedStructure(editableStructure);

    render(<TableEditorPanel tab={tableTab} />);
    settleStatus("users");

    expect(screen.queryByTestId("table-readonly-banner")).toBeNull();
  });
});

describe("TableEditorPanel save wiring", () => {
  it("clicking Save calls commit_cell_edits with the right payload", async () => {
    seed({
      connectionId: "conn-1",
      schema: "public",
      table: "users",
      columns: ["id", "email"],
      rows: [["1", "ada@example.com"]],
      page: 1,
      pageSize: 100,
      totalRows: 1,
      runtimeMs: 5,
    });
    seedStructure(editableStructure);

    // Pre-populate an edit so the Save button is rendered.
    act(() => {
      useAppStore.getState().setTableEdit("users", 0, 1, "ada@new.com");
    });

    render(<TableEditorPanel tab={tableTab} />);
    settleStatus("users");
    mockedInvoke.mockReset();
    mockedInvoke
      .mockResolvedValueOnce({ rowsAffected: 1, runtimeMs: 4 })
      // refresh
      .mockResolvedValueOnce({
        columns: ["id", "email"],
        rows: [["1", "ada@new.com"]],
        page: 1,
        pageSize: 100,
        totalRows: 1,
        runtimeMs: 1,
      });

    await act(async () => {
      fireEvent.click(screen.getByRole("button", { name: /save changes/i }));
      await Promise.resolve();
      await Promise.resolve();
    });

    expect(mockedInvoke).toHaveBeenCalledWith("commit_cell_edits", {
      payload: {
        connectionId: "conn-1",
        schema: "public",
        table: "users",
        edits: [
          {
            rowIndex: 0,
            identity: [{ column: "id", value: "1" }],
            set: [{ column: "email", value: "ada@new.com" }],
          },
        ],
      },
    });
  });
});

describe("TableEditorPanel add record", () => {
  it("disables the Add row button until structure is loaded", () => {
    seed({
      connectionId: "conn-1",
      schema: "public",
      table: "users",
      columns: ["id", "email"],
      rows: [["1", "ada@example.com"]],
      page: 1,
      pageSize: 100,
      totalRows: 1,
      runtimeMs: 5,
    });
    // No structure seeded yet.
    useAppStore.setState({
      connections: [postgresConnection],
      activeConnectionId: "conn-1",
    });

    render(<TableEditorPanel tab={tableTab} />);
    settleStatus("users");

    const addButton = screen.getByRole("button", {
      name: /add row/i,
    }) as HTMLButtonElement;
    expect(addButton.disabled).toBe(true);
  });

  it("enables the Add row button once structure is loaded for Postgres", () => {
    seed({
      connectionId: "conn-1",
      schema: "public",
      table: "users",
      columns: ["id", "email"],
      rows: [["1", "ada@example.com"]],
      page: 1,
      pageSize: 100,
      totalRows: 1,
      runtimeMs: 5,
    });
    seedStructure(editableStructure);

    render(<TableEditorPanel tab={tableTab} />);
    settleStatus("users");

    const addButton = screen.getByRole("button", {
      name: /add row/i,
    }) as HTMLButtonElement;
    expect(addButton.disabled).toBe(false);
  });

  it("opens an inline form with one input per column when Add row is clicked", () => {
    seed({
      connectionId: "conn-1",
      schema: "public",
      table: "users",
      columns: ["id", "email"],
      rows: [["1", "ada@example.com"]],
      page: 1,
      pageSize: 100,
      totalRows: 1,
      runtimeMs: 5,
    });
    seedStructure(editableStructure);

    render(<TableEditorPanel tab={tableTab} />);
    settleStatus("users");

    fireEvent.click(screen.getByRole("button", { name: /add row/i }));

    const form = screen.getByTestId("add-row-form");
    expect(form).toBeTruthy();
    // One input per column from the structure.
    expect(
      form.querySelectorAll("input[data-testid^='add-row-value-']"),
    ).toHaveLength(2);
  });

  it("submits insert_row with the entered values and refreshes", async () => {
    seed({
      connectionId: "conn-1",
      schema: "public",
      table: "users",
      columns: ["id", "email"],
      rows: [["1", "ada@example.com"]],
      page: 1,
      pageSize: 100,
      totalRows: 1,
      runtimeMs: 5,
    });
    seedStructure(editableStructure);

    render(<TableEditorPanel tab={tableTab} />);
    settleStatus("users");

    fireEvent.click(screen.getByRole("button", { name: /add row/i }));

    // For non-nullable, no-default columns the form starts in `value` mode
    // with an empty input. Type into the email input.
    const emailInput = screen.getByTestId(
      "add-row-value-email",
    ) as HTMLInputElement;
    fireEvent.change(emailInput, { target: { value: "grace@example.com" } });

    // Set id mode to default-bypass via radio group. The id column has no
    // default in editableStructure so the form starts in `value` mode; we
    // type something to satisfy the not-null constraint.
    const idInput = screen.getByTestId("add-row-value-id") as HTMLInputElement;
    fireEvent.change(idInput, { target: { value: "2" } });

    mockedInvoke.mockReset();
    mockedInvoke
      .mockResolvedValueOnce({ rowsAffected: 1, runtimeMs: 4 })
      .mockResolvedValueOnce({
        columns: ["id", "email"],
        rows: [
          ["1", "ada@example.com"],
          ["2", "grace@example.com"],
        ],
        page: 1,
        pageSize: 100,
        totalRows: 2,
        runtimeMs: 1,
      });

    await act(async () => {
      fireEvent.click(screen.getByRole("button", { name: /^insert$/i }));
      await Promise.resolve();
      await Promise.resolve();
    });

    expect(mockedInvoke).toHaveBeenCalledWith("insert_row", {
      payload: {
        connectionId: "conn-1",
        schema: "public",
        table: "users",
        values: [
          { column: "id", value: "2" },
          { column: "email", value: "grace@example.com" },
        ],
      },
    });
  });
});

describe("TableEditorPanel delete selected", () => {
  it("disables Delete selected when no rows are selected", () => {
    seed({
      connectionId: "conn-1",
      schema: "public",
      table: "users",
      columns: ["id", "email"],
      rows: [["1", "ada@example.com"]],
      page: 1,
      pageSize: 100,
      totalRows: 1,
      runtimeMs: 5,
    });
    seedStructure(editableStructure);

    render(<TableEditorPanel tab={tableTab} />);
    settleStatus("users");

    const del = screen.getByRole("button", {
      name: /delete selected/i,
    }) as HTMLButtonElement;
    expect(del.disabled).toBe(true);
  });

  it("disables Delete selected when the table has no identity (read-only)", () => {
    seed({
      connectionId: "conn-1",
      schema: "public",
      table: "users",
      columns: ["id", "email"],
      rows: [["1", "ada@example.com"]],
      page: 1,
      pageSize: 100,
      totalRows: 1,
      runtimeMs: 5,
    });
    seedStructure(readOnlyStructure);

    render(<TableEditorPanel tab={tableTab} />);
    settleStatus("users");

    // Even after selecting a row, delete remains disabled because the
    // table is read-only (no identity). Click the first row's checkbox.
    const checkboxes = screen.getAllByRole("checkbox");
    fireEvent.click(checkboxes[1] as HTMLInputElement);

    const del = screen.getByRole("button", {
      name: /delete selected/i,
    }) as HTMLButtonElement;
    expect(del.disabled).toBe(true);
  });

  it("invokes delete_rows on confirmed delete and refreshes", async () => {
    seed({
      connectionId: "conn-1",
      schema: "public",
      table: "users",
      columns: ["id", "email"],
      rows: [
        ["1", "ada@example.com"],
        ["2", "grace@example.com"],
      ],
      page: 1,
      pageSize: 100,
      totalRows: 2,
      runtimeMs: 5,
    });
    seedStructure(editableStructure);

    render(<TableEditorPanel tab={tableTab} />);
    settleStatus("users");

    // Select the first data row's checkbox (header is index 0).
    const checkboxes = screen.getAllByRole("checkbox");
    fireEvent.click(checkboxes[1] as HTMLInputElement);

    const confirmSpy = vi
      .spyOn(window, "confirm")
      .mockImplementation(() => true);

    mockedInvoke.mockReset();
    mockedInvoke
      .mockResolvedValueOnce({ rowsAffected: 1, runtimeMs: 3 })
      .mockResolvedValueOnce({
        columns: ["id", "email"],
        rows: [["2", "grace@example.com"]],
        page: 1,
        pageSize: 100,
        totalRows: 1,
        runtimeMs: 1,
      });

    await act(async () => {
      fireEvent.click(screen.getByRole("button", { name: /delete selected/i }));
      await Promise.resolve();
      await Promise.resolve();
    });

    expect(confirmSpy).toHaveBeenCalled();
    expect(mockedInvoke).toHaveBeenCalledWith("delete_rows", {
      payload: {
        connectionId: "conn-1",
        schema: "public",
        table: "users",
        rows: [[{ column: "id", value: "1" }]],
      },
    });
    confirmSpy.mockRestore();
  });

  it("does not invoke delete_rows when the confirmation is cancelled", () => {
    seed({
      connectionId: "conn-1",
      schema: "public",
      table: "users",
      columns: ["id", "email"],
      rows: [["1", "ada@example.com"]],
      page: 1,
      pageSize: 100,
      totalRows: 1,
      runtimeMs: 5,
    });
    seedStructure(editableStructure);

    render(<TableEditorPanel tab={tableTab} />);
    settleStatus("users");

    const checkboxes = screen.getAllByRole("checkbox");
    fireEvent.click(checkboxes[1] as HTMLInputElement);

    const confirmSpy = vi
      .spyOn(window, "confirm")
      .mockImplementation(() => false);
    mockedInvoke.mockReset();

    fireEvent.click(screen.getByRole("button", { name: /delete selected/i }));

    expect(confirmSpy).toHaveBeenCalled();
    expect(mockedInvoke).not.toHaveBeenCalled();
    confirmSpy.mockRestore();
  });
});

describe("TableEditorPanel row details", () => {
  it("shows a multi-selection state instead of first-row details", async () => {
    seed({
      connectionId: "conn-1",
      schema: "public",
      table: "users",
      columns: ["id", "email"],
      rows: [
        ["1", "ada@example.com"],
        ["2", "grace@example.com"],
      ],
      page: 1,
      pageSize: 100,
      totalRows: 2,
      runtimeMs: 5,
    });
    seedStructure(editableStructure);

    render(<TableEditorPanel tab={tableTab} />);
    settleStatus("users");

    const checkboxes = screen.getAllByRole("checkbox");
    await act(async () => {
      fireEvent.click(checkboxes[0] as HTMLInputElement);
    });

    const detailsPanel = within(screen.getByTestId("row-details-panel"));
    expect(detailsPanel.getByText("2 rows selected")).toBeTruthy();
    expect(detailsPanel.getByText("Multiple rows selected")).toBeTruthy();
    expect(
      detailsPanel.getByText("Select a single row to inspect column values."),
    ).toBeTruthy();
    expect(detailsPanel.queryByText("ada@example.com")).toBeNull();
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
