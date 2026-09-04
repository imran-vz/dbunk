/* oxlint-disable anti-slop/no-module-mocking -- The component boundary is isolated from Tauri and the reviewed gate in jsdom. */
// @vitest-environment jsdom
import {
  act,
  cleanup,
  fireEvent,
  render,
  screen,
  waitFor,
} from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { ObjectDdlClientResult } from "@/lib/object-ddl";

const { previewObjectDdl } = vi.hoisted(() => ({
  previewObjectDdl: vi.fn(),
}));

vi.mock("@/lib/object-ddl", () => ({
  previewObjectDdl,
  formatObjectDdlError: (error: { kind: string; reason?: string }) =>
    error.kind === "invalidOp"
      ? `Operation is invalid: ${error.reason}`
      : "Preview failed",
}));

vi.mock("@/components/object-ddl", () => ({
  DdlPlanPreviewGroups: ({
    preview,
  }: {
    preview: { statements: unknown[] };
  }) => <div>Preview: {preview.statements.length}</div>,
  DdlReviewDialog: ({
    onApplied,
    onPartiallyApplied,
    onApplyingChange,
    onOpenChange,
  }: {
    onApplied?: () => void | Promise<void>;
    onPartiallyApplied?: () => void | Promise<void>;
    onApplyingChange?: (applying: boolean) => void;
    onOpenChange: (open: boolean) => void;
  }) => (
    <>
      <button
        onClick={() => {
          onApplyingChange?.(true);
          void Promise.resolve(onApplied?.()).finally(() =>
            onApplyingChange?.(false),
          );
        }}
      >
        Apply mocked DDL
      </button>
      <button onClick={() => void onPartiallyApplied?.()}>
        Mock partial apply
      </button>
      <button onClick={() => onOpenChange(false)}>Close mocked review</button>
    </>
  ),
}));

import {
  useAppStore,
  type Connection,
  type DdlPlanPreview,
  type WorkspaceTab,
} from "@/lib/store";
import {
  newTableDesignerForm,
  type TableDesignerForm,
} from "@/lib/table-designer";

import { TableDesigner } from "./table-designer";

const initialStoreState = useAppStore.getState();

const connection: Connection = {
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

const designerTab = (
  id: string,
  draft = newTableDesignerForm("public"),
): WorkspaceTab => ({
  id,
  kind: "table-designer",
  label: "New table · public",
  connectionId: "conn-1",
  schema: "public",
  tableDesignerDraft: draft,
});

const seedDesigner = (draft?: TableDesignerForm) => {
  const tab = designerTab("designer-1", draft);
  useAppStore.setState({ workspaceTabs: [tab], activeTabId: tab.id });
  return tab;
};

const renderDesigner = (draft?: TableDesignerForm) => {
  const tab = seedDesigner(draft);
  return render(
    <TableDesigner
      tabId={tab.id}
      connectionId={tab.connectionId}
      schema={tab.schema}
    />,
  );
};

function ActiveWorkspaceDesigner() {
  const activeTab = useAppStore((state) =>
    state.workspaceTabs.find((tab) => tab.id === state.activeTabId),
  );
  if (activeTab?.kind !== "table-designer") return <div>Other tab</div>;
  return (
    <TableDesigner
      tabId={activeTab.id}
      connectionId={activeTab.connectionId}
      schema={activeTab.schema}
    />
  );
}

beforeEach(() => {
  previewObjectDdl.mockReset();
  useAppStore.setState(initialStoreState, true);
  useAppStore.setState({
    connections: [connection],
    activeConnectionId: connection.id,
    connectionEpochs: { [connection.id]: 1 },
    pgObjectCatalog: {
      [connection.id]: { status: "ready", generation: 1 },
    },
  });
});

afterEach(() => {
  cleanup();
  useAppStore.setState(initialStoreState, true);
  vi.useRealTimers();
});

describe("TableDesigner", () => {
  it("debounces rapid valid edits into one preview request", async () => {
    vi.useFakeTimers();
    previewObjectDdl.mockResolvedValue({
      kind: "ok",
      value: { statements: [], groups: [] },
    });
    renderDesigner();

    const name = screen.getByRole("textbox", { name: "Table name" });
    fireEvent.change(name, { target: { value: "o" } });
    fireEvent.change(name, { target: { value: "or" } });
    fireEvent.change(name, { target: { value: "orders" } });

    expect(previewObjectDdl).not.toHaveBeenCalled();
    await act(() => vi.advanceTimersByTimeAsync(249));
    expect(previewObjectDdl).not.toHaveBeenCalled();
    await act(() => vi.advanceTimersByTimeAsync(1));

    expect(previewObjectDdl).toHaveBeenCalledTimes(1);
    expect(previewObjectDdl).toHaveBeenCalledWith({
      connectionId: "conn-1",
      ops: [expect.objectContaining({ op: "createTable", name: "orders" })],
    });
  });

  it("cancels a pending preview and shows local errors immediately", async () => {
    vi.useFakeTimers();
    previewObjectDdl.mockResolvedValue({
      kind: "ok",
      value: { statements: [], groups: [] },
    });
    const draft = newTableDesignerForm("public");
    draft.name = "orders";
    renderDesigner(draft);

    fireEvent.change(screen.getByRole("textbox", { name: "Table name" }), {
      target: { value: "" },
    });

    expect(
      screen.getAllByText("Table name is required.").length,
    ).toBeGreaterThan(0);
    await act(() => vi.advanceTimersByTimeAsync(250));
    expect(previewObjectDdl).not.toHaveBeenCalled();
  });

  it("ignores an in-flight preview result after the form changes", async () => {
    vi.useFakeTimers();
    let resolveFirst:
      | ((value: ObjectDdlClientResult<DdlPlanPreview>) => void)
      | undefined;
    previewObjectDdl.mockImplementationOnce(
      () =>
        new Promise<ObjectDdlClientResult<DdlPlanPreview>>((resolve) => {
          resolveFirst = resolve;
        }),
    );
    const draft = newTableDesignerForm("public");
    draft.name = "orders";
    renderDesigner(draft);

    await act(() => vi.advanceTimersByTimeAsync(250));
    expect(previewObjectDdl).toHaveBeenCalledTimes(1);
    fireEvent.change(screen.getByRole("textbox", { name: "Table name" }), {
      target: { value: "invoices" },
    });
    await act(async () => {
      resolveFirst?.({
        kind: "ok",
        value: {
          statements: [
            {
              sql: "CREATE TABLE orders",
              summary: "Create old table",
              destructive: false,
              transactional: true,
            },
          ],
          groups: [],
        },
      });
      await Promise.resolve();
    });

    expect(screen.queryByText("Preview: 1")).toBeNull();
    expect(
      screen.getByTestId("table-designer-create").hasAttribute("disabled"),
    ).toBe(true);
  });

  it("fences an in-flight preview across disconnect and reconnect", async () => {
    vi.useFakeTimers();
    let resolveOld:
      | ((value: ObjectDdlClientResult<DdlPlanPreview>) => void)
      | undefined;
    let resolveCurrent:
      | ((value: ObjectDdlClientResult<DdlPlanPreview>) => void)
      | undefined;
    previewObjectDdl
      .mockImplementationOnce(
        () =>
          new Promise<ObjectDdlClientResult<DdlPlanPreview>>((resolve) => {
            resolveOld = resolve;
          }),
      )
      .mockImplementationOnce(
        () =>
          new Promise<ObjectDdlClientResult<DdlPlanPreview>>((resolve) => {
            resolveCurrent = resolve;
          }),
      );
    const draft = newTableDesignerForm("public");
    draft.name = "orders";
    renderDesigner(draft);

    await act(() => vi.advanceTimersByTimeAsync(250));
    expect(previewObjectDdl).toHaveBeenCalledTimes(1);

    act(() => {
      useAppStore.setState({
        connections: [{ ...connection, status: "Disconnected" }],
        connectionTransitionIds: [connection.id],
        connectionEpochs: { [connection.id]: 2 },
        pgObjectCatalog: {
          [connection.id]: { status: "idle", generation: 2 },
        },
      });
    });
    await act(() => vi.advanceTimersByTimeAsync(250));
    expect(previewObjectDdl).toHaveBeenCalledTimes(1);
    expect(
      screen.getByTestId("table-designer-create").hasAttribute("disabled"),
    ).toBe(true);

    act(() => {
      useAppStore.setState({
        connections: [connection],
        connectionTransitionIds: [],
        connectionEpochs: { [connection.id]: 3 },
        pgObjectCatalog: {
          [connection.id]: { status: "ready", generation: 3 },
        },
      });
    });
    await act(() => vi.advanceTimersByTimeAsync(250));
    expect(previewObjectDdl).toHaveBeenCalledTimes(2);

    await act(async () => {
      resolveOld?.({
        kind: "error",
        error: { kind: "invalidOp", opIndex: 0, reason: "old lifetime" },
      });
      await Promise.resolve();
    });
    expect(screen.queryByText(/old lifetime/)).toBeNull();
    expect(
      screen.getByTestId("table-designer-create").hasAttribute("disabled"),
    ).toBe(true);

    await act(async () => {
      resolveCurrent?.({
        kind: "ok",
        value: {
          statements: [
            {
              sql: "CREATE TABLE orders",
              summary: "Create current table",
              destructive: false,
              transactional: true,
            },
          ],
          groups: [{ kind: "atomic", statementIndexes: [0] }],
        },
      });
      await Promise.resolve();
    });
    expect(screen.getByText("Preview: 1")).toBeTruthy();
    expect(
      screen.getByTestId("table-designer-create").hasAttribute("disabled"),
    ).toBe(false);
  });

  it("shows a backend invalidOp beside the mapped comment field", async () => {
    previewObjectDdl.mockResolvedValue({
      kind: "error",
      error: { kind: "invalidOp", opIndex: 1, reason: "bad comment" },
    });
    renderDesigner();
    fireEvent.change(screen.getByRole("textbox", { name: "Table name" }), {
      target: { value: "orders" },
    });
    fireEvent.change(screen.getByRole("textbox", { name: "Table comment" }), {
      target: { value: "Orders" },
    });
    await waitFor(() =>
      expect(screen.getAllByText(/bad comment/).length).toBeGreaterThan(0),
    );
    expect(
      screen.getByRole("textbox", { name: "Table comment" }).parentElement
        ?.textContent,
    ).toContain("bad comment");
  });

  it("shows the aggregate createTable invalidOp prominently", async () => {
    previewObjectDdl.mockResolvedValue({
      kind: "error",
      error: {
        kind: "invalidOp",
        opIndex: 0,
        reason: "bad foreign key",
      },
    });
    renderDesigner();
    fireEvent.change(screen.getByRole("textbox", { name: "Table name" }), {
      target: { value: "orders" },
    });

    expect(
      (await screen.findByTestId("table-designer-invalid-op")).textContent,
    ).toContain("Table definition: Operation is invalid: bad foreign key");
    expect(
      screen.getByRole("textbox", { name: "Table name" }).parentElement
        ?.textContent,
    ).not.toContain("bad foreign key");
  });

  it("maps an index invalidOp to the exact index row", async () => {
    const draft = newTableDesignerForm("public");
    draft.name = "orders";
    draft.indexes.push({
      id: "index-1",
      name: "orders_id_idx",
      columns: ["id"],
      unique: false,
      method: "btree",
      include: [],
      wherePredicate: "",
      concurrently: false,
    });
    previewObjectDdl.mockResolvedValue({
      kind: "error",
      error: { kind: "invalidOp", opIndex: 1, reason: "bad index" },
    });

    renderDesigner(draft);

    expect((await screen.findAllByText(/bad index/)).length).toBeGreaterThan(0);
    expect(screen.getByTestId("table-designer-index-0").textContent).toContain(
      "bad index",
    );
  });

  it("renders local constraint errors at their editable controls", async () => {
    previewObjectDdl.mockResolvedValue({
      kind: "ok",
      value: { statements: [], groups: [] },
    });
    const draft = newTableDesignerForm("public");
    draft.name = "orders";
    renderDesigner(draft);
    await waitFor(() => expect(previewObjectDdl).toHaveBeenCalledTimes(1));
    previewObjectDdl.mockClear();

    const primaryKey = screen.getByRole("textbox", {
      name: "Primary key columns",
    });
    fireEvent.change(primaryKey, { target: { value: "missing" } });
    expect(primaryKey.getAttribute("aria-invalid")).toBe("true");
    expect(primaryKey.parentElement?.textContent).toContain(
      "Column missing is not declared.",
    );

    fireEvent.click(screen.getByRole("button", { name: "Add unique" }));
    const unique = screen.getByRole("textbox", {
      name: "Unique 1 columns",
    });
    expect(unique.getAttribute("aria-invalid")).toBe("true");
    expect(unique.parentElement?.textContent).toContain(
      "Choose at least one column.",
    );

    fireEvent.click(screen.getByRole("button", { name: "Add check" }));
    const check = screen.getByRole("textbox", {
      name: "Check 1 expression",
    });
    expect(check.getAttribute("aria-invalid")).toBe("true");
    expect(check.parentElement?.textContent).toContain(
      "Check expression is required.",
    );

    fireEvent.click(screen.getByRole("button", { name: "Add foreign key" }));
    const foreignKeyColumns = screen.getByRole("textbox", {
      name: "Foreign key 1 columns",
    });
    const foreignKeyTable = screen.getByRole("textbox", {
      name: "Foreign key 1 table",
    });
    const referencedColumns = screen.getByRole("textbox", {
      name: "Foreign key 1 referenced columns",
    });
    expect(foreignKeyColumns.getAttribute("aria-invalid")).toBe("true");
    expect(foreignKeyTable.getAttribute("aria-invalid")).toBe("true");
    expect(referencedColumns.getAttribute("aria-invalid")).toBe("true");

    fireEvent.click(
      screen.getByRole("checkbox", { name: "Initially deferred" }),
    );
    expect(
      screen.getByRole("checkbox", { name: "Initially deferred" }).parentElement
        ?.parentElement?.textContent,
    ).toContain("Initially deferred requires a deferrable constraint.");

    fireEvent.click(screen.getByRole("button", { name: "Add index" }));
    const indexColumns = screen.getByRole("textbox", {
      name: "Index 1 columns",
    });
    const indexMethod = screen.getByRole("textbox", {
      name: "Index 1 method",
    });
    fireEvent.change(indexColumns, { target: { value: "" } });
    fireEvent.change(indexMethod, { target: { value: "" } });
    expect(indexColumns.getAttribute("aria-invalid")).toBe("true");
    expect(indexColumns.parentElement?.textContent).toContain(
      "Choose at least one index column.",
    );
    expect(indexMethod.getAttribute("aria-invalid")).toBe("true");
    expect(indexMethod.parentElement?.textContent).toContain(
      "Index method is required.",
    );
    expect(previewObjectDdl).not.toHaveBeenCalled();
  });

  it("preserves trailing commas while controlled list fields are being typed", () => {
    previewObjectDdl.mockImplementation(() => new Promise(() => {}));
    const draft = newTableDesignerForm("public");
    draft.name = "orders";
    renderDesigner(draft);

    const primaryKey = screen.getByRole<HTMLInputElement>("textbox", {
      name: "Primary key columns",
    });
    fireEvent.change(primaryKey, { target: { value: "id," } });
    expect(primaryKey.value).toBe("id, ");

    fireEvent.click(screen.getByRole("button", { name: "Add unique" }));
    const unique = screen.getByRole<HTMLInputElement>("textbox", {
      name: "Unique 1 columns",
    });
    fireEvent.change(unique, { target: { value: "id," } });
    expect(unique.value).toBe("id, ");

    fireEvent.click(screen.getByRole("button", { name: "Add foreign key" }));
    const foreignKeyColumns = screen.getByRole<HTMLInputElement>("textbox", {
      name: "Foreign key 1 columns",
    });
    const referencedColumns = screen.getByRole<HTMLInputElement>("textbox", {
      name: "Foreign key 1 referenced columns",
    });
    fireEvent.change(foreignKeyColumns, { target: { value: "id," } });
    fireEvent.change(referencedColumns, { target: { value: "id," } });
    expect(foreignKeyColumns.value).toBe("id, ");
    expect(referencedColumns.value).toBe("id, ");

    fireEvent.click(screen.getByRole("button", { name: "Add index" }));
    const indexColumns = screen.getByRole<HTMLInputElement>("textbox", {
      name: "Index 1 columns",
    });
    fireEvent.change(indexColumns, {
      target: { value: "coalesce(id, id)," },
    });
    expect(indexColumns.value).toBe("coalesce(id, id), ");
    expect(indexColumns.getAttribute("aria-invalid")).toBe("true");
    expect(
      screen.getByTestId("table-designer-create").hasAttribute("disabled"),
    ).toBe(true);
  });

  it("blocks preview and marks unsafe SQL fragment controls invalid", () => {
    previewObjectDdl.mockImplementation(() => new Promise(() => {}));
    const draft = newTableDesignerForm("public");
    draft.name = "orders";
    draft.columns[0] = {
      ...draft.columns[0]!,
      identity: "none",
      dataType: "integer NOT NULL",
      defaultKind: "expression",
      defaultValue: "0, DROP COLUMN secret",
    };
    draft.checks.push({
      id: "check-1",
      name: null,
      expression: "id > 0; DROP TABLE orders",
    });
    draft.indexes.push({
      id: "index-1",
      name: "orders_id_idx",
      columns: ["id; DROP TABLE orders"],
      unique: false,
      method: "btree",
      include: [],
      wherePredicate: "id > 0; DROP TABLE orders",
      concurrently: false,
    });

    renderDesigner(draft);

    const dataType = screen.getByRole("textbox", { name: "Column 1 type" });
    const defaultValue = screen.getByRole("textbox", {
      name: "Column 1 default",
    });
    const check = screen.getByRole("textbox", {
      name: "Check 1 expression",
    });
    const indexColumns = screen.getByRole("textbox", {
      name: "Index 1 columns",
    });
    const indexPredicate = screen.getByRole("textbox", {
      name: "Index 1 predicate",
    });
    expect(dataType.getAttribute("aria-invalid")).toBe("true");
    expect(dataType.parentElement?.textContent).toContain(
      "Data type cannot contain column options.",
    );
    expect(defaultValue.getAttribute("aria-invalid")).toBe("true");
    expect(defaultValue.parentElement?.textContent).toContain(
      "Default expression escapes its field.",
    );
    expect(check.getAttribute("aria-invalid")).toBe("true");
    expect(check.parentElement?.textContent).toContain(
      "Check expression cannot contain a statement boundary.",
    );
    expect(indexColumns.getAttribute("aria-invalid")).toBe("true");
    expect(indexColumns.parentElement?.textContent).toContain(
      "Index expression cannot contain a statement boundary.",
    );
    expect(indexPredicate.getAttribute("aria-invalid")).toBe("true");
    expect(indexPredicate.parentElement?.textContent).toContain(
      "Index predicate cannot contain a statement boundary.",
    );
    expect(previewObjectDdl).not.toHaveBeenCalled();
  });

  it("does not offer conditional table creation", () => {
    previewObjectDdl.mockImplementation(() => new Promise(() => {}));
    renderDesigner();

    expect(
      screen.queryByRole("checkbox", { name: "IF NOT EXISTS" }),
    ).toBeNull();
  });

  it("keeps independent drafts when tab switches unmount and remount the designer", () => {
    previewObjectDdl.mockImplementation(() => new Promise(() => {}));
    const first = designerTab("designer-1");
    const second = designerTab("designer-2");
    useAppStore.setState({
      workspaceTabs: [first, second],
      activeTabId: first.id,
    });
    render(<ActiveWorkspaceDesigner />);

    fireEvent.change(screen.getByRole("textbox", { name: "Table name" }), {
      target: { value: "orders" },
    });
    expect(
      useAppStore.getState().workspaceTabs[0]?.tableDesignerDraft?.name,
    ).toBe("orders");
    expect(
      useAppStore.getState().workspaceTabs[1]?.tableDesignerDraft?.name,
    ).toBe("");

    act(() => useAppStore.getState().setActiveTabId(second.id));
    const secondName = screen.getByRole("textbox", { name: "Table name" });
    expect(secondName).toBeInstanceOf(HTMLInputElement);
    if (!(secondName instanceof HTMLInputElement)) {
      throw new Error("Table name must render as an input.");
    }
    expect(secondName.value).toBe("");
    act(() => useAppStore.getState().setActiveTabId(first.id));
    const firstName = screen.getByRole("textbox", { name: "Table name" });
    expect(firstName).toBeInstanceOf(HTMLInputElement);
    if (!(firstName instanceof HTMLInputElement)) {
      throw new Error("Table name must render as an input.");
    }
    expect(firstName.value).toBe("orders");
  });

  it("disables and clears nullable/default controls when identity is selected", () => {
    previewObjectDdl.mockImplementation(() => new Promise(() => {}));
    const draft = newTableDesignerForm("public");
    draft.name = "accounts";
    renderDesigner(draft);

    const identity = screen.getByRole("combobox", {
      name: "Column 1 identity",
    });
    const nullable = screen.getByRole("checkbox", {
      name: "Column 1 nullable",
    });
    const defaultValue = screen.getByRole("textbox", {
      name: "Column 1 default",
    });
    const defaultKind = screen.getByRole("combobox", {
      name: "Column 1 default kind",
    });
    expect(nullable.hasAttribute("disabled")).toBe(true);
    expect(defaultValue.hasAttribute("disabled")).toBe(true);

    fireEvent.change(identity, { target: { value: "none" } });
    expect(nullable.hasAttribute("disabled")).toBe(false);
    expect(defaultValue.hasAttribute("disabled")).toBe(true);
    fireEvent.change(defaultKind, { target: { value: "expression" } });
    expect(defaultValue.hasAttribute("disabled")).toBe(false);
    fireEvent.click(nullable);
    fireEvent.change(defaultValue, { target: { value: "42" } });
    fireEvent.change(identity, { target: { value: "always" } });

    const storedColumn =
      useAppStore.getState().workspaceTabs[0]?.tableDesignerDraft?.columns[0];
    expect(storedColumn).toMatchObject({
      identity: "always",
      nullable: false,
      defaultValue: "",
    });
    expect(nullable.hasAttribute("disabled")).toBe(true);
    expect(defaultValue.hasAttribute("disabled")).toBe(true);
  });

  it("opens the created table and closes the designer after apply", async () => {
    previewObjectDdl.mockResolvedValue({
      kind: "ok",
      value: {
        statements: [
          {
            sql: "CREATE TABLE",
            summary: "Create table",
            destructive: false,
            transactional: true,
          },
        ],
        groups: [{ kind: "atomic", statementIndexes: [0] }],
      },
    });
    const openTableTab = vi.fn();
    const closeDesignerTab = useAppStore.getState().closeTab;
    const closeTab = vi.fn((tabId: string) => closeDesignerTab(tabId));
    seedDesigner();
    useAppStore.setState({ openTableTab, closeTab });
    render(
      <TableDesigner
        tabId="designer-1"
        connectionId="conn-1"
        schema="public"
      />,
    );
    fireEvent.change(screen.getByRole("textbox", { name: "Table name" }), {
      target: { value: "orders" },
    });
    await waitFor(() =>
      expect(
        screen.getByTestId("table-designer-create").hasAttribute("disabled"),
      ).toBe(false),
    );
    fireEvent.click(screen.getByTestId("table-designer-create"));
    fireEvent.click(screen.getByRole("button", { name: "Apply mocked DDL" }));
    await waitFor(() =>
      expect(openTableTab).toHaveBeenCalledWith("public", "orders"),
    );
    expect(closeTab).toHaveBeenCalledWith("designer-1");
    await waitFor(() =>
      expect(
        useAppStore
          .getState()
          .workspaceTabs.some((tab) => tab.id === "designer-1"),
      ).toBe(false),
    );
    expect(
      useAppStore
        .getState()
        .workspaceTabs.some((tab) => Boolean(tab.tableDesignerApplying)),
    ).toBe(false);
  });

  it("opens the created table after a partial apply review is closed", async () => {
    previewObjectDdl.mockResolvedValue({
      kind: "ok",
      value: {
        statements: [
          {
            sql: "CREATE TABLE",
            summary: "Create table",
            destructive: false,
            transactional: true,
          },
          {
            sql: "CREATE INDEX CONCURRENTLY",
            summary: "Create index",
            destructive: false,
            transactional: false,
          },
        ],
        groups: [
          { kind: "atomic", statementIndexes: [0] },
          { kind: "standalone", statementIndexes: [1] },
        ],
      },
    });
    const openTableTab = vi.fn();
    const closeDesignerTab = useAppStore.getState().closeTab;
    const closeTab = vi.fn((tabId: string) => closeDesignerTab(tabId));
    seedDesigner();
    useAppStore.setState({ openTableTab, closeTab });
    render(
      <TableDesigner
        tabId="designer-1"
        connectionId="conn-1"
        schema="public"
      />,
    );
    fireEvent.change(screen.getByRole("textbox", { name: "Table name" }), {
      target: { value: "orders" },
    });
    await waitFor(() =>
      expect(
        screen.getByTestId("table-designer-create").hasAttribute("disabled"),
      ).toBe(false),
    );

    fireEvent.click(screen.getByTestId("table-designer-create"));
    fireEvent.click(screen.getByRole("button", { name: "Mock partial apply" }));
    expect(openTableTab).not.toHaveBeenCalled();
    fireEvent.click(
      screen.getByRole("button", { name: "Close mocked review" }),
    );

    await waitFor(() =>
      expect(openTableTab).toHaveBeenCalledWith("public", "orders"),
    );
    expect(closeTab).toHaveBeenCalledWith("designer-1");
  });
});
