/* oxlint-disable anti-slop/no-module-mocking anti-slop/no-unknown-parameters anti-slop/require-safety-comment-for-type-assertion -- Test fixtures use controlled mocks and assertions to exercise otherwise inaccessible boundaries. */
import {
  act,
  fireEvent,
  render,
  screen,
  waitFor,
  within,
} from "@testing-library/react";
import { useState } from "react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const { mockedRequestConfirm } = vi.hoisted(() => ({
  mockedRequestConfirm: vi.fn(() => Promise.resolve(true)),
}));
vi.mock("@/lib/confirm", () => ({
  requestConfirm: mockedRequestConfirm,
  requestPrompt: vi.fn(() => Promise.resolve(null)),
}));

vi.mock("@/lib/ui-state", () => {
  const memory = new Map<string, string>();
  return {
    initUiState: () => Promise.resolve(),
    isUiStateReady: () => true,
    flushUiState: () => Promise.resolve(),
    uiGet: (key: string) => memory.get(key) ?? null,
    uiSet: (key: string, value: string) => {
      memory.set(key, value);
    },
    uiRemove: (key: string) => {
      memory.delete(key);
    },
    uiRemovePrefix: (prefix: string) => {
      const doomed: string[] = [];
      for (const key of memory.keys()) {
        if (key.startsWith(prefix)) doomed.push(key);
      }
      for (const key of doomed) memory.delete(key);
    },
    resetUiStateForTests: () => {
      memory.clear();
    },
  };
});

vi.mock("@/lib/tauri", () => ({
  isTauri: vi.fn(() => true),
  tauriInvoke: vi.fn(),
  errorToMessage: (error: unknown) =>
    error instanceof Error ? error.message : String(error),
}));

vi.mock("@/lib/result-mutation", async (importOriginal) => {
  const actual = await importOriginal<typeof import("@/lib/result-mutation")>();
  return { ...actual, supportsResultMutations: vi.fn(() => false) };
});

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

import type { StatusBarItem } from "@/components/status-bar";
import {
  TableEditorPanel,
  TableSidebar,
} from "@/components/table-editor-panel";
import type { AnalyzeResultSetResult } from "@/lib/result-mutation";
import { supportsResultMutations } from "@/lib/result-mutation";
import {
  buildMutationDraftPlan,
  type Connection,
  type TableBrowseTabState,
  type TableDataState,
  type TableLoadStatus,
  type TableStructure,
  tableDataKey,
  tableMutationDraftScope,
  tableStructureKey,
  useAppStore,
  type WorkspaceTab,
} from "@/lib/store";
import { defaultTableGridPrefs } from "@/lib/table-browse";
import { tauriInvoke } from "@/lib/tauri";

const mockedInvoke = vi.mocked(tauriInvoke);
const mockedSupportsResultMutations = vi.mocked(supportsResultMutations);

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
  mockedSupportsResultMutations.mockReturnValue(false);
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

const mutationAnalysis = (
  identity: AnalyzeResultSetResult["tables"][number]["identity"] = {
    kind: "primaryKey",
    columns: ["id"],
  },
  options: {
    identityProjected?: boolean;
    emailWritability?: "writable" | "generated" | "identityAlways";
  } = {},
): AnalyzeResultSetResult => ({
  requestId: 1,
  analysisId: 91,
  columns: [
    {
      name: "id",
      origin: {
        kind: "table",
        schema: "public",
        table: "users",
        column: "id",
        attnum: 1,
      },
      castType: "integer",
      nullable: false,
      writability: { kind: "writable" },
    },
    {
      name: "email",
      origin: {
        kind: "table",
        schema: "public",
        table: "users",
        column: "email",
        attnum: 2,
      },
      castType: "text",
      nullable: true,
      writability: { kind: options.emailWritability ?? "writable" },
    },
  ],
  tables: [
    {
      schema: "public",
      table: "users",
      identity,
      identityProjected: options.identityProjected ?? true,
      identityProjectionIndexes: identity.columns.flatMap((column) =>
        column === "id" ? [0] : column === "email" ? [1] : [],
      ),
      updatable:
        identity.kind === "none"
          ? { allowed: false, reason: "noIdentity" }
          : { allowed: true },
      deletable:
        identity.kind === "none"
          ? { allowed: false, reason: "noIdentity" }
          : { allowed: true },
      insertable: { allowed: true },
    },
  ],
  statement: { kind: "analyzed" },
});

const seedMutationDraftAnalysis = (analysis: AnalyzeResultSetResult) => {
  const handle = useAppStore.getState().openMutationDraft({
    owner: { kind: "table", tabId: "tab-1" },
    connectionId: "conn-1",
    source: {
      kind: "relation",
      schema: "public",
      table: "users",
    },
  });
  if (!handle) throw new Error("Expected mutation draft handle");
  useAppStore.getState().setMutationDraftAnalysis(handle, analysis);
  return handle;
};

describe("TableEditorPanel read-only handling", () => {
  it("disables edit affordances for a read-only connection", () => {
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
    useAppStore.setState({
      connections: [{ ...postgresConnection, readOnly: true }],
    });
    seedStructure(editableStructure);

    render(<TableEditorPanel tab={tableTab} />);
    settleStatus("users");

    expect(screen.getByTestId("table-readonly-banner").textContent).toContain(
      "Local is a read-only connection",
    );
    expect(
      (screen.getByRole("button", { name: /add row/i }) as HTMLButtonElement)
        .disabled,
    ).toBe(true);
  });

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

    mockedRequestConfirm.mockReset();
    mockedRequestConfirm.mockResolvedValue(true);

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

    expect(mockedRequestConfirm).toHaveBeenCalled();
    expect(mockedInvoke).toHaveBeenCalledWith("delete_rows", {
      payload: {
        connectionId: "conn-1",
        schema: "public",
        table: "users",
        rows: [[{ column: "id", value: "1" }]],
      },
    });
  });

  it("does not invoke delete_rows when the confirmation is cancelled", async () => {
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

    mockedRequestConfirm.mockReset();
    mockedRequestConfirm.mockResolvedValue(false);
    mockedInvoke.mockReset();

    await act(async () => {
      fireEvent.click(screen.getByRole("button", { name: /delete selected/i }));
    });

    expect(mockedRequestConfirm).toHaveBeenCalled();
    expect(mockedInvoke).not.toHaveBeenCalled();
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

describe("TableEditorPanel server browse", () => {
  const refreshedBrowseResult = {
    requestId: 1_000_000,
    columns: [
      { name: "id", castType: "integer", nullable: false },
      { name: "email", castType: "text", nullable: false },
    ],
    rows: [["1", "ada@example.com"]],
    identity: { kind: "primaryKey" as const, columns: ["id"] },
    rowIdentity: [["1"]],
    pageInfo: {
      mode: "keyset" as const,
      page: 1,
      hasMore: false,
      nextCursor: null,
    },
    count: { kind: "estimated" as const, value: 128 },
    inspection: { sql: "SELECT id, email FROM public.users", params: [] },
    omittedRows: 0,
    truncatedCells: 0,
    runtimeMs: 5,
  };

  const seedBrowse = (overrides: Partial<TableBrowseTabState> = {}) => {
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
    useAppStore.setState({
      tableBrowses: {
        "tab-1": {
          tabId: "tab-1",
          connectionId: "conn-1",
          schema: "public",
          table: "users",
          generation: 1,
          typedFilters: [],
          rawFilterText: "",
          filterMode: "typed",
          sort: [],
          pageSize: 100,
          page: 1,
          cursorStack: [],
          nextRequestToken: 1,
          inflightRequestId: null,
          appliedRequestId: 1,
          result: {
            requestId: 1,
            columns: [
              { name: "id", castType: "integer", nullable: false },
              { name: "email", castType: "text", nullable: false },
            ],
            rows: [["1", "ada@example.com"]],
            identity: { kind: "primaryKey", columns: ["id"] },
            rowIdentity: [["1"]],
            pageInfo: {
              mode: "keyset",
              page: 1,
              hasMore: true,
              nextCursor: { values: ["1"] },
            },
            count: { kind: "estimated", value: 128 },
            inspection: {
              sql: "SELECT id, email FROM public.users",
              params: [],
            },
            omittedRows: 0,
            truncatedCells: 0,
            runtimeMs: 5,
          },
          loadStatus: { state: "success" },
          countStatus: { state: "idle" },
          exactCount: null,
          prefsLoaded: true,
          prefs: defaultTableGridPrefs(),
          ...overrides,
        },
      },
    });
  };

  it("labels estimated counts and offers Count rows", () => {
    seedBrowse();
    const onStatusItemsChange = vi.fn();
    render(
      <TableEditorPanel
        tab={tableTab}
        onStatusItemsChange={onStatusItemsChange}
      />,
    );
    expect(
      screen.getAllByText("~128 rows (estimated)").length,
    ).toBeGreaterThanOrEqual(2);
    expect(screen.getByRole("button", { name: "Count rows" })).toBeTruthy();
    const statusItems = onStatusItemsChange.mock.calls.at(-1)?.[0];
    expect(statusItems).toEqual(
      expect.arrayContaining([
        expect.objectContaining({ id: "data", value: "~128 rows (estimated)" }),
        expect.objectContaining({ id: "page", value: "1 of ~2" }),
      ]),
    );
  });

  it("does not display browse rows whose target differs from the tab", () => {
    seedBrowse();
    const auditTab = { ...tableTab, schema: "audit" };
    render(<TableEditorPanel tab={auditTab} />);

    expect(screen.queryByText("ada@example.com")).toBeNull();
    expect(useAppStore.getState().tableBrowses["tab-1"]).toMatchObject({
      connectionId: "conn-1",
      schema: "audit",
      table: "users",
      result: null,
    });
  });

  it("expands the grid and restores it with Escape", () => {
    seedBrowse();
    render(<TableEditorPanel tab={tableTab} />);
    fireEvent.click(screen.getByRole("button", { name: "Expand grid" }));
    expect(screen.queryByLabelText("Table actions")).toBeNull();
    fireEvent.keyDown(window, { key: "Escape" });
    expect(screen.getByLabelText("Table actions")).toBeTruthy();
  });

  it("prompts before a page change discards pending edits", () => {
    seedBrowse();
    act(() => {
      useAppStore.getState().setTableEdit("users", 0, 1, "ada@new.com");
    });
    render(<TableEditorPanel tab={tableTab} />);
    fireEvent.click(screen.getByLabelText("Next page"));
    expect(screen.getByText("Discard pending edits?")).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "Cancel" }));
    expect(screen.queryByText("Discard pending edits?")).toBeNull();
    expect(
      useAppStore.getState().tableEdits[
        tableDataKey("conn-1", "public", "users")
      ],
    ).toBeTruthy();
  });

  it("discards confirmed edits before starting a foreground page request", () => {
    seedBrowse();
    act(() => {
      useAppStore.getState().setTableEdit("users", 0, 1, "ada@new.com");
    });
    render(<TableEditorPanel tab={tableTab} />);
    fireEvent.click(screen.getByLabelText("Next page"));
    fireEvent.click(screen.getByRole("button", { name: "Discard edits" }));

    expect(
      useAppStore.getState().tableEdits[
        tableDataKey("conn-1", "public", "users")
      ],
    ).toBeUndefined();
    expect(useAppStore.getState().tableBrowses["tab-1"]?.loadStatus).toEqual({
      state: "loading",
    });
  });

  it("prompts before refresh discards pending edits", () => {
    seedBrowse();
    act(() => {
      useAppStore.getState().setTableEdit("users", 0, 1, "ada@new.com");
    });
    render(<TableEditorPanel tab={tableTab} />);
    fireEvent.click(screen.getByRole("button", { name: "Refresh" }));
    expect(screen.getByText("Discard pending edits?")).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "Cancel" }));
    expect(screen.queryByText("Discard pending edits?")).toBeNull();
    expect(
      useAppStore.getState().tableEdits[
        tableDataKey("conn-1", "public", "users")
      ],
    ).toBeTruthy();
  });

  it("refreshes the backend descriptor for an explicit user refresh", async () => {
    seedBrowse({
      exactCount: { requestId: 1, kind: "exact", value: 128 },
    });
    render(<TableEditorPanel tab={tableTab} />);
    mockedInvoke.mockReset();
    mockedInvoke.mockResolvedValue(refreshedBrowseResult);

    fireEvent.click(screen.getByRole("button", { name: "Refresh" }));

    await waitFor(() =>
      expect(mockedInvoke).toHaveBeenCalledWith("browse_table_data", {
        payload: expect.objectContaining({ refreshStructure: true }),
      }),
    );
    expect(useAppStore.getState().tableBrowses["tab-1"]?.exactCount).toBeNull();
  });

  it("refreshes imported browse data without refreshing the descriptor", async () => {
    seedBrowse({
      exactCount: { requestId: 1, kind: "exact", value: 128 },
    });
    render(<TableEditorPanel tab={tableTab} />);
    fireEvent.click(screen.getByRole("button", { name: "Import data" }));
    fireEvent.change(screen.getByLabelText("Import file"), {
      target: {
        files: [
          {
            name: "users.csv",
            text: async () => "id,email\n2,grace@example.com",
          } as File,
        ],
      },
    });
    await screen.findByText(/1 rows ready/);
    mockedInvoke.mockReset();
    mockedInvoke
      .mockResolvedValueOnce({ runtimeMs: 3, rowsAffected: 1 })
      .mockResolvedValueOnce(refreshedBrowseResult);

    fireEvent.click(screen.getByRole("button", { name: "Import rows" }));

    await waitFor(() =>
      expect(mockedInvoke).toHaveBeenCalledWith("browse_table_data", {
        payload: expect.objectContaining({ refreshStructure: false }),
      }),
    );
    expect(useAppStore.getState().tableBrowses["tab-1"]?.exactCount).toBeNull();
  });

  it("refreshes seeded browse data without refreshing the descriptor", async () => {
    seedBrowse({
      exactCount: { requestId: 1, kind: "exact", value: 128 },
    });
    render(<TableEditorPanel tab={tableTab} />);
    fireEvent.click(screen.getByRole("button", { name: "Table actions" }));
    fireEvent.click(
      await screen.findByRole("menuitem", { name: /seed table/i }),
    );
    mockedInvoke.mockReset();
    mockedInvoke
      .mockResolvedValueOnce({
        runtimeMs: 4,
        rowsInserted: 100,
        seedUsed: 42,
      })
      .mockResolvedValueOnce(refreshedBrowseResult);

    fireEvent.click(screen.getByTestId("seed-submit"));

    await waitFor(() =>
      expect(mockedInvoke).toHaveBeenCalledWith("browse_table_data", {
        payload: expect.objectContaining({ refreshStructure: false }),
      }),
    );
    expect(useAppStore.getState().tableBrowses["tab-1"]?.exactCount).toBeNull();
  });

  it("clears same-index selection when a same-shape result is applied", () => {
    seedBrowse();
    render(<TableEditorPanel tab={tableTab} />);
    fireEvent.click(screen.getAllByRole("checkbox")[1] as HTMLInputElement);
    expect(
      (
        screen.getByRole("button", {
          name: "Delete selected",
        }) as HTMLButtonElement
      ).disabled,
    ).toBe(false);

    act(() => {
      useAppStore.setState((state) => {
        const current = state.tableBrowses["tab-1"];
        if (!current?.result) return state;
        return {
          tableBrowses: {
            ...state.tableBrowses,
            "tab-1": {
              ...current,
              appliedRequestId: 2,
              result: {
                ...current.result,
                requestId: 2,
                rows: [["2", "grace@example.com"]],
                rowIdentity: [["2"]],
              },
            },
          },
        };
      });
    });

    expect(
      (
        screen.getByRole("button", {
          name: "Delete selected",
        }) as HTMLButtonElement
      ).disabled,
    ).toBe(true);
  });

  it("enables editing and deletion for authoritative unique-index identity", () => {
    seedBrowse({
      result: {
        requestId: 1,
        columns: [
          { name: "id", castType: "integer", nullable: false },
          { name: "email", castType: "text", nullable: false },
        ],
        rows: [["1", "ada@example.com"]],
        identity: { kind: "uniqueIndex", columns: ["email"] },
        rowIdentity: [["ada@example.com"]],
        pageInfo: {
          mode: "keyset",
          page: 1,
          hasMore: false,
          nextCursor: null,
        },
        count: { kind: "exact", value: 1 },
        inspection: { sql: "SELECT id, email FROM public.users", params: [] },
        omittedRows: 0,
        truncatedCells: 0,
        runtimeMs: 5,
      },
    });
    seedStructure(readOnlyStructure);
    render(<TableEditorPanel tab={tableTab} />);

    fireEvent.doubleClick(
      screen.getByRole("button", { name: "ada@example.com" }),
    );
    expect(screen.getByDisplayValue("ada@example.com")).toBeTruthy();
    fireEvent.click(screen.getAllByRole("checkbox")[1] as HTMLInputElement);
    expect(
      (
        screen.getByRole("button", {
          name: "Delete selected",
        }) as HTMLButtonElement
      ).disabled,
    ).toBe(false);
    expect(screen.queryByTestId("table-readonly-banner")).toBeNull();
  });

  it("offers Cancel while an exact count is loading", () => {
    seedBrowse({ countStatus: { state: "loading" } });
    render(<TableEditorPanel tab={tableTab} />);
    expect(screen.getByRole("button", { name: "Counting…" })).toBeTruthy();
    expect(screen.getByRole("button", { name: "Cancel" })).toBeTruthy();
  });

  it("shows honest read-only copy for virtual browse identity", () => {
    seedBrowse({
      result: {
        requestId: 1,
        columns: [{ name: "note", castType: "text", nullable: true }],
        rows: [["heap"]],
        identity: { kind: "virtual", columns: ["ctid"] },
        rowIdentity: [["(0,1)"]],
        pageInfo: {
          mode: "keyset",
          page: 1,
          hasMore: false,
          nextCursor: null,
        },
        count: { kind: "unknown", value: null },
        inspection: { sql: "SELECT note FROM public.users", params: [] },
        omittedRows: 0,
        truncatedCells: 0,
        runtimeMs: 3,
      },
    });
    render(<TableEditorPanel tab={tableTab} />);
    expect(
      screen.getByText(
        "This table is paged with a virtual identity and is read-only.",
      ),
    ).toBeTruthy();
  });

  describe("activated mutation drafts", () => {
    beforeEach(() => {
      mockedSupportsResultMutations.mockReturnValue(true);
    });

    it("lazily analyzes the relation on the first edit gesture", async () => {
      seedBrowse();
      const analyzed = mutationAnalysis();
      mockedInvoke.mockImplementation((command) =>
        command === "analyze_result_set"
          ? Promise.resolve(analyzed)
          : new Promise(() => {}),
      );
      render(<TableEditorPanel tab={tableTab} />);

      fireEvent.click(screen.getByRole("button", { name: "ada@example.com" }));

      await waitFor(() =>
        expect(mockedInvoke).toHaveBeenCalledWith("analyze_result_set", {
          payload: expect.objectContaining({
            connectionId: "conn-1",
            tabId: "tab-1",
            source: {
              kind: "relation",
              schema: "public",
              table: "users",
            },
          }),
        }),
      );
      expect(
        useAppStore.getState().mutationDrafts[tableMutationDraftScope("tab-1")]
          ?.analysis?.analysisId,
      ).toBe(91);
      expect(screen.getByTestId("table-mutation-status").textContent).toContain(
        "Staged editing ready",
      );
    });

    it("stages a true-NULL cell update by browse row identity and opens Variant A review", async () => {
      seedBrowse({
        result: {
          ...refreshedBrowseResult,
          requestId: 2,
          rows: [["1", null]],
          rowIdentity: [["1"]],
        },
      });
      seedMutationDraftAnalysis(mutationAnalysis());
      render(<TableEditorPanel tab={tableTab} />);

      fireEvent.doubleClick(screen.getByRole("button", { name: "NULL" }));
      const input = screen.getByDisplayValue("NULL") as HTMLInputElement;
      fireEvent.change(input, { target: { value: "ada@new.com" } });
      fireEvent.blur(input);

      const draft =
        useAppStore.getState().mutationDrafts[tableMutationDraftScope("tab-1")];
      const change = draft?.changes[draft.changeOrder[0] ?? ""];
      expect(change).toMatchObject({
        kind: "updateRow",
        identity: [{ column: "id", value: "1" }],
        originals: [
          { column: "id", value: "1" },
          { column: "email", value: null },
        ],
        cells: {
          email: { original: null, value: "ada@new.com" },
        },
      });
      expect(screen.getByLabelText("Review 1 staged changes")).toBeTruthy();

      fireEvent.click(screen.getByLabelText("Review 1 staged changes"));
      expect(
        screen.getByRole("complementary", { name: "Mutation review" }),
      ).toBeTruthy();
      expect(screen.queryByTestId("row-details-panel")).toBeNull();
    });

    it("refreshes every browse tab for the relation after apply", async () => {
      seedBrowse();
      useAppStore.setState((state) => {
        const first = state.tableBrowses["tab-1"];
        if (!first) throw new Error("Expected seeded table browse");
        return {
          tableBrowses: {
            ...state.tableBrowses,
            "tab-2": { ...first, tabId: "tab-2", generation: 2 },
          },
        };
      });
      const handle = seedMutationDraftAnalysis(mutationAnalysis());
      useAppStore.getState().stageMutationDraftUpdate(handle.scope, {
        table: { schema: "public", table: "users" },
        identityKind: "primaryKey",
        identity: [{ column: "id", value: "1" }],
        originals: [
          { column: "id", value: "1" },
          { column: "email", value: "ada@example.com" },
        ],
        cells: [
          {
            column: "email",
            original: "ada@example.com",
            value: "ada@new.example",
          },
        ],
        rowIndex: 0,
      });
      mockedInvoke.mockImplementation((command) => {
        if (command === "preview_result_mutations") {
          return Promise.resolve({
            statements: [
              { opIndex: 0, sql: "UPDATE public.users", params: [] },
            ],
          });
        }
        if (command === "apply_result_mutations") {
          return Promise.resolve({
            operations: [{ opIndex: 0, rowsAffected: 1 }],
            runtimeMs: 1,
          });
        }
        if (command === "browse_table_data") {
          return Promise.resolve(refreshedBrowseResult);
        }
        return new Promise(() => {});
      });
      render(<TableEditorPanel tab={tableTab} />);

      fireEvent.click(screen.getByLabelText("Review 1 staged changes"));
      const apply = await screen.findByRole("button", {
        name: "Apply 1 change",
      });
      await waitFor(() =>
        expect((apply as HTMLButtonElement).disabled).toBe(false),
      );
      fireEvent.click(apply);

      await waitFor(() => {
        const refreshCalls = mockedInvoke.mock.calls.filter(
          ([command]) => command === "browse_table_data",
        );
        expect(refreshCalls).toHaveLength(2);
        expect(refreshCalls).toEqual(
          expect.arrayContaining([
            [
              "browse_table_data",
              { payload: expect.objectContaining({ tabId: "tab-1" }) },
            ],
            [
              "browse_table_data",
              { payload: expect.objectContaining({ tabId: "tab-2" }) },
            ],
          ]),
        );
      });
    });

    it("stages multi-row deletes without confirmation or delete_rows and paginates silently", async () => {
      seedBrowse({
        result: {
          ...refreshedBrowseResult,
          requestId: 3,
          rows: [
            ["1", "ada@example.com"],
            ["2", "grace@example.com"],
          ],
          rowIdentity: [["1"], ["2"]],
          pageInfo: {
            mode: "keyset",
            page: 1,
            hasMore: true,
            nextCursor: { values: ["2"] },
          },
        },
      });
      seedMutationDraftAnalysis(mutationAnalysis());
      mockedRequestConfirm.mockClear();
      render(<TableEditorPanel tab={tableTab} />);
      fireEvent.click(screen.getAllByRole("checkbox")[0] as HTMLInputElement);
      mockedInvoke.mockClear();

      fireEvent.click(screen.getByRole("button", { name: "Delete selected" }));
      await waitFor(() =>
        expect(
          useAppStore.getState().mutationDrafts[
            tableMutationDraftScope("tab-1")
          ]?.changeOrder,
        ).toHaveLength(2),
      );
      expect(mockedRequestConfirm).not.toHaveBeenCalled();
      expect(mockedInvoke).not.toHaveBeenCalledWith(
        "delete_rows",
        expect.anything(),
      );

      fireEvent.click(screen.getByLabelText("Next page"));
      expect(screen.queryByText("Discard pending edits?")).toBeNull();
      expect(mockedRequestConfirm).not.toHaveBeenCalled();
      expect(useAppStore.getState().tableBrowses["tab-1"]?.loadStatus).toEqual({
        state: "loading",
      });
    });

    it("stages new rows, duplicate provenance, and a bulk edit without immediate commands", async () => {
      seedBrowse({
        result: {
          ...refreshedBrowseResult,
          requestId: 4,
          rows: [
            ["1", "ada@example.com"],
            ["2", "grace@example.com"],
          ],
          rowIdentity: [["1"], ["2"]],
        },
      });
      seedMutationDraftAnalysis(mutationAnalysis());
      render(<TableEditorPanel tab={tableTab} />);

      await act(async () => {
        fireEvent.click(screen.getByRole("button", { name: "Add row" }));
        await Promise.resolve();
      });
      fireEvent.change(screen.getByTestId("add-row-value-id"), {
        target: { value: "3" },
      });
      fireEvent.change(screen.getByTestId("add-row-value-email"), {
        target: { value: "lin@example.com" },
      });
      fireEvent.click(screen.getByRole("button", { name: "Stage row" }));
      await waitFor(() =>
        expect(screen.queryByTestId("add-row-form")).toBeNull(),
      );

      fireEvent.click(screen.getAllByRole("checkbox")[1] as HTMLInputElement);
      await act(async () => {
        fireEvent.click(
          screen.getByRole("button", { name: "Duplicate selected row" }),
        );
        await Promise.resolve();
      });
      expect(
        (screen.getByTestId("add-row-value-email") as HTMLInputElement).value,
      ).toBe("ada@example.com");
      fireEvent.click(screen.getByRole("button", { name: "Stage duplicate" }));
      await waitFor(() =>
        expect(screen.queryByTestId("add-row-form")).toBeNull(),
      );

      fireEvent.click(screen.getAllByRole("checkbox")[0] as HTMLInputElement);
      await act(async () => {
        fireEvent.click(
          screen.getByRole("button", { name: "Bulk edit selected rows" }),
        );
        await Promise.resolve();
      });
      fireEvent.change(screen.getByLabelText("Bulk edit column"), {
        target: { value: "email" },
      });
      fireEvent.change(screen.getByLabelText("Bulk edit value"), {
        target: { value: "team@example.com" },
      });
      fireEvent.click(screen.getByRole("button", { name: "Stage bulk edit" }));
      await waitFor(() =>
        expect(screen.queryByTestId("bulk-edit-form")).toBeNull(),
      );

      const draft =
        useAppStore.getState().mutationDrafts[tableMutationDraftScope("tab-1")];
      const changes = draft?.changeOrder.map((id) => draft.changes[id]);
      expect(changes?.filter((change) => change?.kind === "insertRow")).toEqual(
        expect.arrayContaining([
          expect.objectContaining({ source: "new" }),
          expect.objectContaining({ source: "duplicate" }),
        ]),
      );
      expect(
        changes?.filter((change) => change?.kind === "updateRow"),
      ).toHaveLength(2);
      expect(mockedInvoke).not.toHaveBeenCalledWith(
        "insert_row",
        expect.anything(),
      );
      expect(screen.getByText("lin@example.com")).toBeTruthy();
    });

    it("includes hidden ctid with the full visible row in staged update and delete plans", async () => {
      seedBrowse({
        result: {
          ...refreshedBrowseResult,
          requestId: 5,
          rows: [
            ["1", "generated@example.com"],
            ["2", null],
          ],
          identity: { kind: "virtual", columns: ["ctid"] },
          rowIdentity: [["(0,1)"], ["(0,2)"]],
        },
      });
      seedMutationDraftAnalysis(
        mutationAnalysis(
          { kind: "ctidFallback", columns: ["ctid"] },
          { emailWritability: "generated" },
        ),
      );
      render(<TableEditorPanel tab={tableTab} />);

      const generated = screen.getByRole("button", {
        name: "generated@example.com",
      });
      expect(generated.getAttribute("title")).toBe(
        "Generated columns are read-only.",
      );
      fireEvent.doubleClick(screen.getByRole("button", { name: "1" }));
      const input = screen.getByDisplayValue("1");
      fireEvent.change(input, { target: { value: "7" } });
      fireEvent.blur(input);

      fireEvent.click(screen.getAllByRole("checkbox")[2] as HTMLInputElement);
      fireEvent.click(screen.getByRole("button", { name: "Delete selected" }));

      await waitFor(() =>
        expect(
          useAppStore.getState().mutationDrafts[
            tableMutationDraftScope("tab-1")
          ]?.changeOrder,
        ).toHaveLength(2),
      );

      const draft =
        useAppStore.getState().mutationDrafts[tableMutationDraftScope("tab-1")];
      expect(draft?.changes[draft.changeOrder[0] ?? ""]).toMatchObject({
        identityKind: "ctidFallback",
        identity: [{ column: "ctid", value: "(0,1)" }],
        originals: [
          { column: "id", value: "1" },
          { column: "email", value: "generated@example.com" },
          { column: "ctid", value: "(0,1)" },
        ],
      });
      expect(draft?.changes[draft.changeOrder[1] ?? ""]).toMatchObject({
        identityKind: "ctidFallback",
        identity: [{ column: "ctid", value: "(0,2)" }],
        originals: [
          { column: "id", value: "2" },
          { column: "email", value: null },
          { column: "ctid", value: "(0,2)" },
        ],
      });
      expect(draft && buildMutationDraftPlan(draft).plan.operations).toEqual([
        {
          kind: "update",
          table: { schema: "public", table: "users" },
          identity: [{ column: "ctid", value: "(0,1)" }],
          guards: [
            { column: "id", value: "1" },
            { column: "email", value: "generated@example.com" },
            { column: "ctid", value: "(0,1)" },
          ],
          set: [{ column: "id", value: "7" }],
        },
        {
          kind: "delete",
          table: { schema: "public", table: "users" },
          identity: [{ column: "ctid", value: "(0,2)" }],
          guards: [
            { column: "id", value: "2" },
            { column: "email", value: null },
            { column: "ctid", value: "(0,2)" },
          ],
        },
      ]);
      expect(screen.getByTestId("table-mutation-status").textContent).toContain(
        "full-row guards",
      );
    });

    it("persists and clears a projected virtual key, then reanalyzes", async () => {
      seedBrowse({
        result: {
          ...refreshedBrowseResult,
          requestId: 6,
          identity: { kind: "none", columns: [] },
          rowIdentity: null,
        },
      });
      seedMutationDraftAnalysis(
        mutationAnalysis({ kind: "none", columns: [] }),
      );
      const virtualAnalysis = mutationAnalysis({
        kind: "virtualKey",
        columns: ["email"],
      });
      let analysisCalls = 0;
      mockedInvoke.mockImplementation((command) => {
        if (command === "save_virtual_key" || command === "clear_virtual_key") {
          return Promise.resolve(undefined);
        }
        if (command === "analyze_result_set") {
          analysisCalls += 1;
          return Promise.resolve(
            analysisCalls === 1
              ? virtualAnalysis
              : mutationAnalysis({ kind: "none", columns: [] }),
          );
        }
        return new Promise(() => {});
      });
      render(<TableEditorPanel tab={tableTab} />);

      fireEvent.click(
        screen.getByRole("button", { name: "Choose virtual key" }),
      );
      fireEvent.click(screen.getByRole("checkbox", { name: "email" }));
      fireEvent.click(screen.getByRole("button", { name: "Save virtual key" }));

      await waitFor(() =>
        expect(mockedInvoke).toHaveBeenCalledWith("save_virtual_key", {
          payload: {
            connectionId: "conn-1",
            schema: "public",
            table: "users",
            columns: ["email"],
          },
        }),
      );
      await waitFor(() =>
        expect(
          useAppStore.getState().mutationDrafts[
            tableMutationDraftScope("tab-1")
          ]?.analysis?.snapshot.tables[0]?.identity.kind,
        ).toBe("virtualKey"),
      );
      expect(screen.getByTestId("table-mutation-status").textContent).toContain(
        "Virtual key: email",
      );

      fireEvent.click(
        screen.getByRole("button", { name: "Clear virtual key" }),
      );
      await waitFor(() =>
        expect(mockedInvoke).toHaveBeenCalledWith("clear_virtual_key", {
          payload: {
            connectionId: "conn-1",
            schema: "public",
            table: "users",
          },
        }),
      );
      await waitFor(() =>
        expect(
          useAppStore.getState().mutationDrafts[
            tableMutationDraftScope("tab-1")
          ]?.analysis?.snapshot.tables[0]?.identity.kind,
        ).toBe("none"),
      );
    });
  });
});

// Sentinel: with the status-items ping-pong the passive-effect loop
// starves timers, so break the cycle by throwing a clear error instead
// of relying on React's own "Maximum update depth exceeded".
const RENDER_SENTINEL = 200;
let renderCount = 0;

function StatusItemsHarness() {
  // oxlint-disable-next-line react/globals -- deliberate render-count sentinel: it must increment during render to detect runaway loops
  renderCount += 1;
  if (renderCount > RENDER_SENTINEL) {
    throw new Error(
      `unbounded render loop: status items ping-pong (${renderCount} renders)`,
    );
  }
  const [items, setItems] = useState<StatusBarItem[]>([]);
  return (
    <>
      <TableEditorPanel tab={tableTab} onStatusItemsChange={setItems} />
      <output data-testid="status-count">{items.length}</output>
    </>
  );
}

describe("TableEditorPanel status items", () => {
  it("reports status items to a parent state setter without an update loop", async () => {
    // Regression: the panel rebuilds the items array on every render;
    // an unguarded effect looped notify → parent setState → re-render →
    // "Maximum update depth exceeded" when a table tab opened in the
    // workbench.
    seed({
      connectionId: "conn-1",
      schema: "public",
      table: "users",
      columns: ["id"],
      rows: [["1"]],
      page: 1,
      pageSize: 100,
      totalRows: 1,
      runtimeMs: 5,
    });

    renderCount = 0;
    render(<StatusItemsHarness />);
    await act(async () => {});

    // The callback still delivers items…
    expect(
      Number(screen.getByTestId("status-count").textContent),
    ).toBeGreaterThan(0);
    // …and rendering settles instead of looping (sentinel throws red
    // long before this when the bug is present).
    expect(renderCount).toBeLessThan(RENDER_SENTINEL);
  });
});
