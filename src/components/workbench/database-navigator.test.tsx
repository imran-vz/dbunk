/* oxlint-disable anti-slop/no-module-mocking, anti-slop/no-unknown-parameters -- Test fixtures use controlled mocks to exercise otherwise inaccessible boundaries. */
// @vitest-environment jsdom
import {
  cleanup,
  fireEvent,
  render,
  screen,
  waitFor,
} from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("@/lib/tauri", () => ({
  isTauri: vi.fn(() => true),
  tauriInvoke: vi.fn(),
  errorToMessage: (error: unknown) =>
    error instanceof Error ? error.message : String(error),
}));

import { DatabaseNavigator } from "@/components/workbench/database-navigator";
import {
  type Connection,
  type PgObjectCatalog,
  useAppStore,
} from "@/lib/store";
import { tauriInvoke } from "@/lib/tauri";

const mockedInvoke = vi.mocked(tauriInvoke);

const initialStoreState = useAppStore.getState();

beforeEach(() => {
  useAppStore.setState(initialStoreState, true);
  useAppStore.setState({
    activeConnectionId: "conn-1",
    expandedSchemas: [],
    expandedNavigatorGroups: [],
  });
  mockedInvoke.mockReset();
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

const pgConnection: Connection = {
  id: "conn-1",
  name: "Postgres",
  database: "postgres",
  host: "localhost",
  port: 5432,
  user: "postgres",
  password: "",
  role: "admin",
  engine: "PostgreSQL",
  ssl: false,
  status: "Connected",
  latency: "1 ms",
};

const pgCatalog = (): PgObjectCatalog => ({
  schemas: [
    {
      name: "lifecycle",
      tables: Array.from({ length: 250 }, (_, index) => ({
        name: `table_${String(index + 1).padStart(3, "0")}`,
      })),
      views: [{ name: "orders_view" }],
      materializedViews: [{ name: "orders_mat" }],
      foreignTables: [],
      sequences: [{ name: "order_number_seq" }],
      functions: [{ name: "add_nums", identityArgs: "integer, integer" }],
      procedures: [],
      aggregates: [],
      types: [{ name: "order_status", typeClass: "enum" }],
      domains: [{ name: "positive_amount" }],
      extensions: [{ name: "pgcrypto" }],
    },
  ],
  eventTriggers: [{ name: "audit_ddl" }],
  roles: [{ name: "dbunk" }],
  tablespaces: [],
  truncated: [{ schema: "lifecycle", kind: "table" }],
});

function renderPgNavigator(options?: {
  onOpenTable?: ReturnType<typeof vi.fn>;
  onOpenView?: ReturnType<typeof vi.fn>;
  onOpenObject?: ReturnType<typeof vi.fn>;
  onCreateObject?: ReturnType<typeof vi.fn>;
  catalog?: PgObjectCatalog;
  connection?: Connection;
}) {
  const catalog = options?.catalog ?? pgCatalog();
  useAppStore.setState({
    connections: [options?.connection ?? pgConnection],
    pgObjectCatalog: {
      "conn-1": { status: "ready", catalog, generation: 0 },
    },
  });
  const onOpenTable = options?.onOpenTable ?? vi.fn();
  const onOpenView = options?.onOpenView ?? vi.fn();
  const onOpenObject = options?.onOpenObject ?? vi.fn();
  const onCreateObject = options?.onCreateObject;
  render(
    <DatabaseNavigator
      connectionId="conn-1"
      schemas={[]}
      activeTableKey={null}
      onOpenTable={onOpenTable}
      onOpenView={onOpenView}
      onOpenObject={onOpenObject}
      onCreateObject={onCreateObject}
    />,
  );
  return { onOpenTable, onOpenView, onOpenObject, onCreateObject };
}

describe("DatabaseNavigator", () => {
  it("does not expose PostgreSQL object actions for another engine", () => {
    useAppStore.setState({
      connections: [
        {
          id: "conn-1",
          name: "MySQL",
          database: "app",
          status: "Connected",
          engine: "MySQL",
          host: "localhost",
          port: 3306,
          user: "root",
          password: "",
          role: "admin",
          latency: "1 ms",
          ssl: false,
        },
      ],
    });
    render(
      <DatabaseNavigator
        connectionId="conn-1"
        schemas={schemas}
        activeTableKey={null}
        onOpenTable={vi.fn()}
        onOpenObject={vi.fn()}
        onCreateObject={vi.fn()}
      />,
    );

    expect(screen.queryByRole("button", { name: "New schema" })).toBeNull();
    expect(
      screen.queryByRole("button", {
        name: "Actions for schema public",
      }),
    ).toBeNull();
  });

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

describe("DatabaseNavigator PostgreSQL Object Catalog", () => {
  it("renders non-empty groups, defaults Tables open, and caps initial rows", () => {
    renderPgNavigator();
    fireEvent.click(screen.getByRole("treeitem", { name: /lifecycle/ }));

    expect(
      screen
        .getByRole("treeitem", { name: "Tables" })
        .getAttribute("aria-expanded"),
    ).toBe("true");
    expect(screen.getAllByRole("treeitem", { name: /table_/ })).toHaveLength(
      200,
    );
    expect(screen.getByRole("treeitem", { name: "Show 50 more" })).toBeTruthy();
    expect(
      screen.getByText("Tables list cut at 2000 on the server"),
    ).toBeTruthy();
    expect(
      screen.queryByRole("treeitem", { name: /Foreign Tables/ }),
    ).toBeNull();

    fireEvent.click(screen.getByRole("treeitem", { name: "Show 50 more" }));
    expect(screen.getAllByRole("treeitem", { name: /table_/ })).toHaveLength(
      250,
    );
  });

  it("filters across collapsed kinds and activates typed objects", () => {
    const { onOpenObject } = renderPgNavigator();

    fireEvent.change(screen.getByRole("textbox", { name: "Filter objects" }), {
      target: { value: "add_nums" },
    });

    const routine = screen.getByRole("treeitem", {
      name: /add_nums\(integer, integer\)/,
    });
    fireEvent.click(routine);
    expect(onOpenObject).toHaveBeenCalledWith({
      kind: "function",
      schema: "lifecycle",
      name: "add_nums",
      identityArgs: "integer, integer",
    });
  });

  it("keeps view activation on the browse path", () => {
    const { onOpenView, onOpenObject } = renderPgNavigator();
    fireEvent.click(screen.getByRole("treeitem", { name: /lifecycle/ }));
    fireEvent.click(screen.getByRole("treeitem", { name: "Views" }));
    fireEvent.click(screen.getByRole("treeitem", { name: "orders_view" }));

    expect(onOpenView).toHaveBeenCalledWith("lifecycle", "orders_view");
    expect(onOpenObject).not.toHaveBeenCalled();
  });

  it("opens schema and table viewers from their secondary actions", () => {
    const { onOpenObject } = renderPgNavigator();

    fireEvent.click(
      screen.getByRole("button", {
        name: "Actions for schema lifecycle",
      }),
    );
    fireEvent.click(
      screen.getByRole("menuitem", { name: "Open schema viewer" }),
    );
    expect(onOpenObject).toHaveBeenLastCalledWith({
      kind: "schema",
      schema: null,
      name: "lifecycle",
      identityArgs: null,
    });

    fireEvent.click(screen.getByRole("treeitem", { name: /lifecycle/ }));
    fireEvent.click(
      screen.getByRole("button", {
        name: "Open object viewer for table_001",
      }),
    );
    expect(onOpenObject).toHaveBeenLastCalledWith({
      kind: "table",
      schema: "lifecycle",
      name: "table_001",
      identityArgs: null,
    });
  });

  it("exposes schema and typed group create affordances", () => {
    const onCreateObject = vi.fn();
    renderPgNavigator({ onCreateObject });

    fireEvent.click(screen.getByRole("button", { name: "New schema" }));
    expect(onCreateObject).toHaveBeenLastCalledWith("schema");

    fireEvent.click(screen.getByRole("treeitem", { name: /lifecycle/ }));
    fireEvent.click(screen.getByRole("button", { name: "New view" }));
    expect(onCreateObject).toHaveBeenLastCalledWith("view", "lifecycle");
    fireEvent.click(
      screen.getByRole("button", { name: "New materialized view" }),
    );
    expect(onCreateObject).toHaveBeenLastCalledWith(
      "materialized-view",
      "lifecycle",
    );
    fireEvent.click(screen.getByRole("button", { name: "New sequence" }));
    expect(onCreateObject).toHaveBeenLastCalledWith("sequence", "lifecycle");
    fireEvent.click(screen.getByRole("button", { name: "New enum" }));
    expect(onCreateObject).toHaveBeenLastCalledWith("enum", "lifecycle");
  });

  it("hides object creation while the connection is disconnected", () => {
    renderPgNavigator({
      onCreateObject: vi.fn(),
      connection: { ...pgConnection, status: "Disconnected" },
    });

    expect(screen.queryByRole("button", { name: "New schema" })).toBeNull();
    fireEvent.click(
      screen.getByRole("button", { name: "Actions for schema lifecycle" }),
    );
    expect(screen.queryByRole("menuitem", { name: "New view" })).toBeNull();
    expect(
      screen.getByRole("menuitem", { name: "Open schema viewer" }),
    ).toBeTruthy();
  });

  it("hides empty groups and keeps creation reachable from the schema row", () => {
    const onCreateObject = vi.fn();
    const catalog = pgCatalog();
    const schema = catalog.schemas[0];
    if (!schema) throw new Error("catalog fixture is missing its schema");
    schema.views = [];
    schema.materializedViews = [];
    schema.sequences = [];
    schema.types = [];

    renderPgNavigator({ catalog, onCreateObject });
    fireEvent.click(screen.getByRole("treeitem", { name: /lifecycle/ }));

    expect(screen.queryByRole("treeitem", { name: "Views" })).toBeNull();
    expect(
      screen.queryByRole("treeitem", { name: "Materialized Views" }),
    ).toBeNull();
    expect(screen.queryByRole("treeitem", { name: "Sequences" })).toBeNull();
    expect(screen.queryByRole("treeitem", { name: "Types" })).toBeNull();
    expect(
      screen.queryByRole("treeitem", { name: "Foreign Tables" }),
    ).toBeNull();

    fireEvent.click(
      screen.getByRole("button", {
        name: "Actions for schema lifecycle",
      }),
    );
    fireEvent.click(screen.getByRole("menuitem", { name: "New sequence" }));
    expect(onCreateObject).toHaveBeenCalledWith("sequence", "lifecycle");
  });

  it("traverses group headers with the roving keyboard model", async () => {
    renderPgNavigator();
    const schemaRow = screen.getByRole("treeitem", { name: /lifecycle/ });
    fireEvent.click(schemaRow);
    schemaRow.focus();
    const tree = screen.getByRole("tree");
    const tabbableButtons = Array.from(tree.querySelectorAll("button")).filter(
      (button) => button.tabIndex >= 0,
    );
    expect(tabbableButtons).toEqual([schemaRow]);

    const tables = screen.getByRole("treeitem", { name: "Tables" });
    fireEvent.keyDown(tree, { key: "ArrowDown" });
    await waitFor(() => expect(document.activeElement).toBe(tables));

    fireEvent.keyDown(tree, { key: "Enter" });
    expect(screen.queryByRole("treeitem", { name: "table_001" })).toBeNull();
  });

  it("opens required row actions from the roving focus with Shift+Enter", async () => {
    const onCreateObject = vi.fn();
    const { onOpenObject } = renderPgNavigator({ onCreateObject });
    const schemaRow = screen.getByRole("treeitem", { name: /lifecycle/ });
    schemaRow.focus();
    const tree = screen.getByRole("tree");

    fireEvent.keyDown(tree, { key: "Enter", shiftKey: true });
    fireEvent.click(screen.getByRole("menuitem", { name: "New sequence" }));
    expect(onCreateObject).toHaveBeenCalledWith("sequence", "lifecycle");
    await waitFor(() => expect(document.activeElement).toBe(schemaRow));

    fireEvent.click(schemaRow);
    const table = screen.getByRole("treeitem", { name: "table_001" });
    fireEvent.focus(table);
    await waitFor(() => expect(table.getAttribute("tabindex")).toBe("0"));
    fireEvent.keyDown(tree, { key: "Enter", shiftKey: true });
    expect(onOpenObject).toHaveBeenCalledWith({
      kind: "table",
      schema: "lifecycle",
      name: "table_001",
      identityArgs: null,
    });
    await waitFor(() => expect(document.activeElement).toBe(table));
    fireEvent.keyDown(table, { key: "ArrowDown" });
    await waitFor(() =>
      expect(document.activeElement).toBe(
        screen.getByRole("treeitem", { name: "table_002" }),
      ),
    );
  });

  it("shows list-only database entries as inert rows", () => {
    const { onOpenObject } = renderPgNavigator();
    fireEvent.click(screen.getByRole("treeitem", { name: /Event Triggers/ }));
    const trigger = screen.getByRole("treeitem", { name: "audit_ddl" });

    expect(trigger.getAttribute("aria-disabled")).toBe("true");
    fireEvent.click(trigger);
    expect(onOpenObject).not.toHaveBeenCalled();
  });

  it("renders a catalog error row and retries the typed fetch", async () => {
    const catalog = pgCatalog();
    mockedInvoke.mockResolvedValueOnce(catalog);
    useAppStore.setState({
      connections: [pgConnection],
      pgObjectCatalog: {
        "conn-1": {
          status: "error",
          generation: 2,
          error: { kind: "connection", message: "socket closed" },
        },
      },
    });
    render(
      <DatabaseNavigator
        connectionId="conn-1"
        schemas={[]}
        activeTableKey={null}
        onOpenTable={vi.fn()}
      />,
    );

    expect(screen.getByRole("alert").textContent).toContain("socket closed");
    fireEvent.click(screen.getByRole("button", { name: /Retry/ }));

    await waitFor(() =>
      expect(mockedInvoke).toHaveBeenCalledWith("load_pg_object_catalog", {
        payload: { connectionId: "conn-1" },
      }),
    );
  });
});
