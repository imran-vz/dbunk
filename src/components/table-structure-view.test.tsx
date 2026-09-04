/* oxlint-disable anti-slop/no-module-mocking anti-slop/no-unknown-parameters anti-slop/require-safety-comment-for-type-assertion -- Test fixtures use controlled mocks and assertions to exercise otherwise inaccessible boundaries. */
import { act, fireEvent, render, screen } from "@testing-library/react";
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

import { TableStructureView } from "@/components/table-structure-view";
import {
  type Connection,
  type TableStructure,
  pgObjectDescriptionKey,
  tableStructureKey,
  useAppStore,
} from "@/lib/store";
import { tauriInvoke } from "@/lib/tauri";

const mockedInvoke = vi.mocked(tauriInvoke);

const initialStoreState = useAppStore.getState();

const seedConnection = (engine: Connection["engine"] = "PostgreSQL") => {
  const common = {
    id: "conn-1",
    name: "Local",
    database: "dbunk",
    status: "Connected" as const,
    host: "localhost",
    port: 5432,
    user: "postgres",
    password: "",
    role: "admin",
    latency: "10 ms",
  };
  let connection: Connection;
  switch (engine) {
    case "PostgreSQL":
    case "MySQL":
      connection = { ...common, engine, ssl: true };
      break;
    case "SQLite":
      connection = { ...common, engine: "SQLite" };
      break;
    case "ClickHouse":
      connection = {
        ...common,
        engine: "ClickHouse",
        useHttps: false,
        urlPath: "",
      };
      break;
    case "Redis":
      connection = {
        ...common,
        engine: "Redis",
        dbNumber: 0,
        useTls: false,
        verifyTlsCert: true,
        readOnly: false,
      };
      break;
  }
  useAppStore.setState({
    connections: [connection],
    activeConnectionId: connection.id,
  });
  return connection;
};

const seedStructure = (structure: TableStructure) => {
  const key = tableStructureKey("conn-1", "public", "users");
  useAppStore.setState({
    tableStructure: { [key]: structure },
    tableStructureStatus: { [key]: { state: "success" } },
  });
  return key;
};

const baseStructure: TableStructure = {
  columns: [
    {
      name: "id",
      dataType: "integer",
      nullable: false,
      defaultValue: "nextval('users_id_seq'::regclass)",
      isPrimaryKey: true,
      ordinalPosition: 1,
    },
    {
      name: "email",
      dataType: "text",
      nullable: true,
      defaultValue: null,
      isPrimaryKey: false,
      ordinalPosition: 2,
    },
    {
      name: "is_active",
      dataType: "boolean",
      nullable: false,
      defaultValue: "true",
      isPrimaryKey: false,
      ordinalPosition: 3,
    },
  ],
  primaryKey: ["id"],
  foreignKeys: [
    {
      name: "users_org_id_fkey",
      columns: ["org_id"],
      referencedSchema: "public",
      referencedTable: "orgs",
      referencedColumns: ["id"],
      onUpdate: "NO ACTION",
      onDelete: "CASCADE",
    },
  ],
  indexes: [
    {
      name: "users_pkey",
      columns: ["id"],
      isUnique: true,
      isPrimary: true,
      method: "btree",
    },
    {
      name: "users_email_idx",
      columns: ["email"],
      isUnique: true,
      isPrimary: false,
      method: "btree",
    },
  ],
  constraints: [
    {
      name: "users_email_check",
      kind: "check",
      definition: "CHECK (email <> '')",
    },
  ],
  triggers: [],
  policies: [],
  privileges: [],
  rowSecurity: null,
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
    triggers: false,
    policies: false,
    privileges: false,
  },
};

beforeEach(() => {
  useAppStore.setState(initialStoreState, true);
  mockedInvoke.mockReset();
  // Default: hold the load_table_structure invocation so seeded state
  // remains observable in tests that don't assert on the network call.
  mockedInvoke.mockImplementation(() => new Promise(() => {}));
  seedConnection();
});

afterEach(() => {
  useAppStore.setState(initialStoreState, true);
  mockedInvoke.mockReset();
});

const settleSuccess = (key: string) => {
  act(() => {
    useAppStore.setState((state) => ({
      tableStructureStatus: {
        ...state.tableStructureStatus,
        [key]: { state: "success" },
      },
    }));
  });
};

describe("TableStructureView", () => {
  it("renders columns from store data using real data types", () => {
    const key = seedStructure(baseStructure);
    render(
      <TableStructureView
        connectionId="conn-1"
        schema="public"
        tableName="users"
      />,
    );
    settleSuccess(key);

    expect(screen.getByText("id")).toBeTruthy();
    expect(screen.getByText("email")).toBeTruthy();
    expect(screen.getByText("is_active")).toBeTruthy();
    // Real types come from the structure payload, not name inference.
    expect(screen.getAllByText("integer").length).toBeGreaterThan(0);
    expect(screen.getAllByText("text").length).toBeGreaterThan(0);
    expect(screen.getAllByText("boolean").length).toBeGreaterThan(0);
  });

  it("marks the primary key column based on isPrimaryKey, not the column name", () => {
    // The column called `id` is NOT the primary key here; `email` is.
    // A name-based inference would mismark `id`. The view must trust
    // the `isPrimaryKey` flag from the payload.
    const key = seedStructure({
      ...baseStructure,
      columns: [
        {
          name: "id",
          dataType: "text",
          nullable: false,
          defaultValue: null,
          isPrimaryKey: false,
          ordinalPosition: 1,
        },
        {
          name: "email",
          dataType: "text",
          nullable: false,
          defaultValue: null,
          isPrimaryKey: true,
          ordinalPosition: 2,
        },
      ],
      primaryKey: ["email"],
    });
    render(
      <TableStructureView
        connectionId="conn-1"
        schema="public"
        tableName="users"
      />,
    );
    settleSuccess(key);

    const idRow = screen.getByTestId("structure-column-id");
    const emailRow = screen.getByTestId("structure-column-email");
    expect(idRow.textContent).not.toContain("PK");
    expect(emailRow.textContent).toContain("PK");
  });

  it("shows the unsupported message and not an empty grid when foreignKeys capability is false", () => {
    const key = seedStructure({
      ...baseStructure,
      foreignKeys: [],
      capabilities: {
        ...baseStructure.capabilities,
        foreignKeys: false,
      },
    });
    render(
      <TableStructureView
        connectionId="conn-1"
        schema="public"
        tableName="users"
      />,
    );
    settleSuccess(key);

    const fkSection = screen.getByTestId("structure-foreign-keys");
    expect(fkSection.textContent).toMatch(/not supported/i);
    expect(fkSection.textContent).toContain("PostgreSQL");
    // Make sure we don't render a misleading "empty" placeholder.
    expect(fkSection.textContent).not.toMatch(/no foreign keys defined/i);
  });

  it("renders foreign keys when supported and present", () => {
    const key = seedStructure(baseStructure);
    render(
      <TableStructureView
        connectionId="conn-1"
        schema="public"
        tableName="users"
      />,
    );
    settleSuccess(key);

    const fkSection = screen.getByTestId("structure-foreign-keys");
    expect(fkSection.textContent).toContain("users_org_id_fkey");
    expect(fkSection.textContent).toContain("orgs");
  });

  it("renders indexes and constraints sections", () => {
    const key = seedStructure(baseStructure);
    render(
      <TableStructureView
        connectionId="conn-1"
        schema="public"
        tableName="users"
      />,
    );
    settleSuccess(key);

    expect(screen.getByTestId("structure-indexes").textContent).toContain(
      "users_email_idx",
    );
    expect(screen.getByTestId("structure-constraints").textContent).toContain(
      "users_email_check",
    );
  });

  it("shows a loading state while the structure is in flight", () => {
    // No seeded structure; status defaults to "loading" once the
    // mount effect kicks off the (held) invoke. Pre-seed loading
    // explicitly so the assertion does not depend on effect timing.
    const key = tableStructureKey("conn-1", "public", "users");
    useAppStore.setState({
      tableStructureStatus: { [key]: { state: "loading" } },
    });
    render(
      <TableStructureView
        connectionId="conn-1"
        schema="public"
        tableName="users"
      />,
    );

    expect(screen.getByTestId("structure-loading")).toBeTruthy();
  });

  it("shows an error banner with retry when the load fails", async () => {
    mockedInvoke.mockReset();
    mockedInvoke.mockRejectedValue(new Error("relation does not exist"));

    const key = tableStructureKey("conn-1", "public", "users");
    useAppStore.setState({
      tableStructureStatus: {
        [key]: { state: "error", error: "relation does not exist" },
      },
    });

    render(
      <TableStructureView
        connectionId="conn-1"
        schema="public"
        tableName="users"
      />,
    );

    await act(async () => {
      await Promise.resolve();
    });

    expect(screen.getByTestId("structure-error")).toBeTruthy();
    expect(screen.getByRole("alert").textContent).toContain(
      "relation does not exist",
    );

    // Clicking Retry should re-issue the invoke.
    mockedInvoke.mockClear();
    mockedInvoke.mockImplementation(() => new Promise(() => {}));
    fireEvent.click(screen.getByRole("button", { name: /retry/i }));
    expect(mockedInvoke).toHaveBeenCalledWith("load_table_structure", {
      payload: {
        connectionId: "conn-1",
        schema: "public",
        table: "users",
      },
    });
  });

  it("loads structure on mount when connection, schema, and table are provided", () => {
    mockedInvoke.mockReset();
    mockedInvoke.mockImplementation(() => new Promise(() => {}));

    render(
      <TableStructureView
        connectionId="conn-1"
        schema="public"
        tableName="users"
      />,
    );

    expect(mockedInvoke).toHaveBeenCalledWith("load_table_structure", {
      payload: {
        connectionId: "conn-1",
        schema: "public",
        table: "users",
      },
    });
  });

  it("shows engine-specific unsupported messages for non-PostgreSQL engines", () => {
    seedConnection("ClickHouse");
    const key = seedStructure({
      ...baseStructure,
      foreignKeys: [],
      indexes: [],
      constraints: [],
      capabilities: {
        columns: true,
        primaryKey: false,
        foreignKeys: false,
        indexes: false,
        constraints: false,
        canInsertRows: false,
        canUpdateRows: false,
        canDeleteRows: false,
        canAlterSchema: false,
        uniquenessGuarantee: "best-effort",
        triggers: false,
        policies: false,
        privileges: false,
      },
    });
    render(
      <TableStructureView
        connectionId="conn-1"
        schema="public"
        tableName="users"
      />,
    );
    settleSuccess(key);

    expect(screen.getByTestId("structure-foreign-keys").textContent).toContain(
      "ClickHouse",
    );
    expect(screen.getByTestId("structure-indexes").textContent).toContain(
      "ClickHouse",
    );
    expect(screen.getByTestId("structure-constraints").textContent).toContain(
      "ClickHouse",
    );
  });
});

describe("TableStructureView edit flow", () => {
  it("closes stale table forms before they can queue against the next table", () => {
    const usersKey = tableStructureKey("conn-1", "public", "users");
    const ordersKey = tableStructureKey("conn-1", "public", "orders");
    const structure = {
      ...baseStructure,
      foreignKeys: [],
      indexes: [],
      constraints: [],
      rowSecurity: { enabled: false, forced: false },
      capabilities: {
        ...baseStructure.capabilities,
        triggers: true,
        policies: true,
        privileges: true,
      },
    } satisfies TableStructure;
    useAppStore.setState({
      tableStructure: {
        [usersKey]: structure,
        [ordersKey]: structure,
      },
      tableStructureStatus: {
        [usersKey]: { state: "success" },
        [ordersKey]: { state: "success" },
      },
    });

    const { rerender } = render(
      <TableStructureView
        connectionId="conn-1"
        schema="public"
        tableName="users"
      />,
    );
    settleSuccess(usersKey);

    fireEvent.click(screen.getByTestId("structure-add-fk"));
    fireEvent.change(screen.getByTestId("structure-fk-name"), {
      target: { value: "stale_users_fk" },
    });
    fireEvent.click(screen.getByTestId("structure-add-index"));
    fireEvent.change(screen.getByTestId("structure-index-name"), {
      target: { value: "stale_users_idx" },
    });
    fireEvent.click(screen.getByRole("button", { name: "New trigger" }));
    fireEvent.change(screen.getByRole("textbox", { name: "Trigger name" }), {
      target: { value: "stale_users_trigger" },
    });
    fireEvent.click(screen.getByRole("button", { name: "New policy" }));
    fireEvent.change(screen.getByRole("textbox", { name: "Policy name" }), {
      target: { value: "stale_users_policy" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Grant" }));
    fireEvent.change(
      screen.getByRole("combobox", { name: "Privilege grantee kind" }),
      { target: { value: "role" } },
    );
    fireEvent.change(
      screen.getByRole("combobox", { name: "Privilege grantee" }),
      { target: { value: "stale_users_role" } },
    );
    fireEvent.click(screen.getByRole("checkbox", { name: "Enabled" }));
    expect(
      screen.getByRole<HTMLInputElement>("checkbox", { name: "Enabled" })
        .checked,
    ).toBe(true);

    rerender(
      <TableStructureView
        connectionId="conn-1"
        schema="public"
        tableName="orders"
      />,
    );
    settleSuccess(ordersKey);

    expect(screen.queryByTestId("structure-add-fk-form")).toBeNull();
    expect(screen.queryByTestId("structure-add-index-form")).toBeNull();
    expect(screen.queryByTestId("structure-trigger-form")).toBeNull();
    expect(screen.queryByTestId("structure-policy-form")).toBeNull();
    expect(screen.queryByTestId("structure-grant-form")).toBeNull();
    expect(
      screen.getByRole<HTMLInputElement>("checkbox", { name: "Enabled" })
        .checked,
    ).toBe(false);

    fireEvent.click(screen.getByRole("button", { name: "New policy" }));
    expect(
      screen.getByRole<HTMLInputElement>("textbox", { name: "Policy name" })
        .value,
    ).toBe("");
    fireEvent.change(screen.getByRole("textbox", { name: "Policy name" }), {
      target: { value: "orders_policy" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Queue policy" }));

    expect(
      useAppStore.getState().pendingStructureChanges[usersKey],
    ).toBeUndefined();
    expect(
      useAppStore
        .getState()
        .pendingStructureChanges[ordersKey]?.map((entry) =>
          entry.change.kind === "pg-op" ? entry.change.op : null,
        ),
    ).toEqual([
      expect.objectContaining({
        op: "createPolicy",
        schema: "public",
        table: "orders",
        name: "orders_policy",
      }),
    ]);
  });

  it("hides editing controls when the engine cannot alter schema", () => {
    seedConnection("MySQL");
    const key = seedStructure({
      ...baseStructure,
      capabilities: {
        ...baseStructure.capabilities,
        canAlterSchema: false,
        canInsertRows: false,
        canUpdateRows: false,
        canDeleteRows: false,
      },
    });
    render(
      <TableStructureView
        connectionId="conn-1"
        schema="public"
        tableName="users"
      />,
    );
    settleSuccess(key);

    expect(screen.queryByTestId("structure-add-column")).toBeNull();
    expect(screen.queryByTestId("structure-pending-section")).toBeNull();
  });

  it("shows the editing toolbar on PostgreSQL", () => {
    const key = seedStructure(baseStructure);
    render(
      <TableStructureView
        connectionId="conn-1"
        schema="public"
        tableName="users"
      />,
    );
    settleSuccess(key);

    expect(screen.getByTestId("structure-add-column")).toBeTruthy();
  });

  it("renders a queued PostgreSQL operation without entering legacy preview", () => {
    const key = seedStructure(baseStructure);
    act(() => {
      useAppStore.getState().addPendingStructureChange(key, {
        schema: "public",
        table: "users",
        change: {
          kind: "pg-op",
          op: {
            op: "dropColumn",
            schema: "public",
            table: "users",
            name: "email",
            cascade: false,
          },
        },
      });
    });

    render(
      <TableStructureView
        connectionId="conn-1"
        schema="public"
        tableName="users"
      />,
    );
    settleSuccess(key);

    expect(screen.getByText("Pending preview")).toBeTruthy();
    act(() => {
      fireEvent.click(screen.getByTestId("structure-preview-sql"));
    });
    expect(screen.queryByTestId("structure-sql-preview")).toBeNull();
  });

  it("queues a typed drop-column op when the drop button is clicked on PostgreSQL", () => {
    const key = seedStructure(baseStructure);
    render(
      <TableStructureView
        connectionId="conn-1"
        schema="public"
        tableName="users"
      />,
    );
    settleSuccess(key);

    act(() => {
      fireEvent.click(screen.getByTestId("structure-drop-column-email"));
    });

    const pending = useAppStore.getState().pendingStructureChanges[key] ?? [];
    expect(pending).toHaveLength(1);
    expect(pending[0].change).toEqual({
      kind: "pg-op",
      op: {
        op: "dropColumn",
        schema: "public",
        table: "users",
        name: "email",
        cascade: false,
      },
    });
    // The pending changes section is now visible.
    expect(screen.getByTestId("structure-pending-section")).toBeTruthy();
  });

  it("removes a queued change when its remove button is clicked", () => {
    const key = seedStructure(baseStructure);
    render(
      <TableStructureView
        connectionId="conn-1"
        schema="public"
        tableName="users"
      />,
    );
    settleSuccess(key);

    act(() => {
      fireEvent.click(screen.getByTestId("structure-drop-column-email"));
    });
    const pendingId =
      useAppStore.getState().pendingStructureChanges[key]?.[0]?.id ?? "";
    expect(pendingId).not.toBe("");

    act(() => {
      fireEvent.click(
        screen.getByTestId(`structure-remove-pending-${pendingId}`),
      );
    });

    expect(
      useAppStore.getState().pendingStructureChanges[key] ?? [],
    ).toHaveLength(0);
  });

  it("loads the backend preview, shows its summaries, and gates Commit on it", async () => {
    const key = seedStructure(baseStructure);
    let resolvePreview!: (value: unknown) => void;
    mockedInvoke.mockReset();
    mockedInvoke.mockImplementation((command) => {
      if (command === "preview_object_ddl") {
        return new Promise((resolve) => {
          resolvePreview = resolve;
        });
      }
      return new Promise(() => {});
    });
    render(
      <TableStructureView
        connectionId="conn-1"
        schema="public"
        tableName="users"
      />,
    );
    settleSuccess(key);

    act(() => {
      fireEvent.click(screen.getByTestId("structure-drop-column-email"));
    });

    // Preview in flight: neutral row label and Commit disabled.
    expect(screen.getByText("Pending preview")).toBeTruthy();
    expect(screen.getByTestId("structure-preview-loading")).toBeTruthy();
    expect(
      screen.getByTestId("structure-commit").hasAttribute("disabled"),
    ).toBe(true);
    const previewCall = mockedInvoke.mock.calls.find(
      ([name]) => name === "preview_object_ddl",
    );
    expect(previewCall).toBeDefined();

    await act(async () => {
      resolvePreview({
        statements: [
          {
            sql: 'ALTER TABLE "public"."users" DROP COLUMN "email";',
            summary: "Drop column email",
            destructive: true,
            transactional: true,
          },
        ],
        groups: [{ kind: "atomic", statementIndexes: [0] }],
      });
      await Promise.resolve();
    });

    // Row label comes from the loaded preview, never describeChange.
    expect(screen.getByText("Drop column email")).toBeTruthy();
    expect(
      screen.getByTestId("structure-commit").hasAttribute("disabled"),
    ).toBe(false);

    act(() => {
      fireEvent.click(screen.getByTestId("structure-preview-sql"));
    });
    expect(screen.getByTestId("structure-ddl-preview").textContent).toContain(
      'DROP COLUMN "email"',
    );
    // The legacy single-blob preview never renders for typed batches.
    expect(screen.queryByTestId("structure-sql-preview")).toBeNull();

    // Editing the pending list invalidates the loaded preview.
    act(() => {
      fireEvent.click(screen.getByTestId("structure-toggle-nullable-email"));
    });
    expect(
      screen.getByTestId("structure-commit").hasAttribute("disabled"),
    ).toBe(true);
    expect(screen.getByTestId("structure-preview-loading")).toBeTruthy();
  });

  it("invalidates a loaded preview when DDL is applied elsewhere", async () => {
    const key = seedStructure(baseStructure);
    mockedInvoke.mockReset();
    mockedInvoke.mockImplementation((command) => {
      if (command === "preview_object_ddl") {
        return Promise.resolve({
          statements: [
            {
              sql: 'ALTER TABLE "public"."users" DROP COLUMN "email";',
              summary: "Drop column email",
              destructive: true,
              transactional: true,
            },
          ],
          groups: [{ kind: "atomic", statementIndexes: [0] }],
        });
      }
      return new Promise(() => {});
    });
    render(
      <TableStructureView
        connectionId="conn-1"
        schema="public"
        tableName="users"
      />,
    );
    settleSuccess(key);

    act(() => {
      fireEvent.click(screen.getByTestId("structure-drop-column-email"));
    });
    await act(async () => {
      await Promise.resolve();
    });
    expect(
      screen.getByTestId("structure-commit").hasAttribute("disabled"),
    ).toBe(false);
    const previewCallsBefore = mockedInvoke.mock.calls.filter(
      ([name]) => name === "preview_object_ddl",
    ).length;

    // Another surface (object viewer, second window) applies DDL.
    act(() => {
      useAppStore.getState().markPgObjectDdlApplied();
    });

    // The reviewed preview is no longer trusted: a reload was issued.
    const previewCallsAfter = mockedInvoke.mock.calls.filter(
      ([name]) => name === "preview_object_ddl",
    ).length;
    expect(previewCallsAfter).toBe(previewCallsBefore + 1);
    await act(async () => {
      await Promise.resolve();
    });
    expect(
      screen.getByTestId("structure-commit").hasAttribute("disabled"),
    ).toBe(false);
  });

  it("aborts the typed commit when the destructive confirm dialog is dismissed", async () => {
    const key = seedStructure(baseStructure);
    mockedInvoke.mockReset();
    mockedInvoke.mockImplementation((command) => {
      if (command === "preview_object_ddl") {
        return Promise.resolve({
          statements: [
            {
              sql: 'ALTER TABLE "public"."users" DROP COLUMN "email";',
              summary: "Drop column email",
              destructive: true,
              transactional: true,
            },
          ],
          groups: [{ kind: "atomic", statementIndexes: [0] }],
        });
      }
      return new Promise(() => {});
    });
    render(
      <TableStructureView
        connectionId="conn-1"
        schema="public"
        tableName="users"
      />,
    );
    settleSuccess(key);

    act(() => {
      fireEvent.click(screen.getByTestId("structure-drop-column-email"));
    });
    await act(async () => {
      await Promise.resolve();
    });

    mockedRequestConfirm.mockReset();
    mockedRequestConfirm.mockResolvedValue(false);

    await act(async () => {
      fireEvent.click(screen.getByTestId("structure-commit"));
    });

    // The confirm detail comes from the preview's destructive summaries.
    expect(mockedRequestConfirm).toHaveBeenCalledWith(
      expect.objectContaining({ detail: "Drop column email" }),
    );
    expect(
      useAppStore.getState().pendingStructureChanges[key] ?? [],
    ).toHaveLength(1);
    expect(
      mockedInvoke.mock.calls.some(([name]) => name === "apply_object_ddl"),
    ).toBe(false);
  });

  it("applies a typed batch through apply_object_ddl when confirmed", async () => {
    const key = seedStructure(baseStructure);
    mockedInvoke.mockReset();
    mockedInvoke.mockImplementation((command) => {
      if (command === "preview_object_ddl") {
        return Promise.resolve({
          statements: [
            {
              sql: 'ALTER TABLE "public"."users" DROP COLUMN "email";',
              summary: "Drop column email",
              destructive: true,
              transactional: true,
            },
          ],
          groups: [{ kind: "atomic", statementIndexes: [0] }],
        });
      }
      if (command === "apply_object_ddl") {
        return Promise.resolve({ appliedStatements: 1, runtimeMs: 8 });
      }
      if (command === "load_table_structure") {
        return Promise.resolve(baseStructure);
      }
      return new Promise(() => {});
    });
    render(
      <TableStructureView
        connectionId="conn-1"
        schema="public"
        tableName="users"
      />,
    );
    settleSuccess(key);

    act(() => {
      fireEvent.click(screen.getByTestId("structure-drop-column-email"));
    });
    await act(async () => {
      await Promise.resolve();
    });

    mockedRequestConfirm.mockReset();
    mockedRequestConfirm.mockResolvedValue(true);

    await act(async () => {
      fireEvent.click(screen.getByTestId("structure-commit"));
    });
    await act(async () => {
      await Promise.resolve();
    });

    expect(mockedRequestConfirm).toHaveBeenCalled();
    const applyCall = mockedInvoke.mock.calls.find(
      ([name]) => name === "apply_object_ddl",
    );
    expect(applyCall).toBeDefined();
    if (applyCall) {
      expect(applyCall[1]).toEqual({
        payload: {
          connectionId: "conn-1",
          ops: [
            {
              op: "dropColumn",
              schema: "public",
              table: "users",
              name: "email",
              cascade: false,
            },
          ],
          confirmed: false,
        },
      });
    }
    expect(
      useAppStore.getState().pendingStructureChanges[key] ?? [],
    ).toHaveLength(0);
    expect(
      screen.getByTestId("structure-commit-success").textContent,
    ).toContain("8");
    expect(
      mockedInvoke.mock.calls.some(([name]) => name === "execute_ddl"),
    ).toBe(false);
  });

  it("keeps ClickHouse on the frontend generator and execute_ddl", async () => {
    seedConnection("ClickHouse");
    const key = seedStructure(baseStructure);
    render(
      <TableStructureView
        connectionId="conn-1"
        schema="public"
        tableName="users"
      />,
    );
    settleSuccess(key);

    // No PostgreSQL lifecycle affordances leak into ClickHouse.
    expect(screen.queryByTestId("structure-add-fk")).toBeNull();
    expect(screen.queryByTestId("structure-add-index")).toBeNull();
    expect(screen.queryByTestId("structure-add-check")).toBeNull();

    act(() => {
      fireEvent.click(screen.getByTestId("structure-drop-column-email"));
    });
    const pending = useAppStore.getState().pendingStructureChanges[key] ?? [];
    expect(pending[0]?.change).toEqual({
      kind: "column",
      change: { kind: "drop", columnName: "email" },
    });

    act(() => {
      fireEvent.click(screen.getByTestId("structure-preview-sql"));
    });
    expect(screen.getByTestId("structure-sql-preview").textContent).toContain(
      "DROP COLUMN",
    );

    mockedRequestConfirm.mockReset();
    mockedRequestConfirm.mockResolvedValue(true);
    mockedInvoke.mockReset();
    mockedInvoke
      .mockResolvedValueOnce({ runtimeMs: 8 })
      .mockResolvedValueOnce(baseStructure);

    await act(async () => {
      fireEvent.click(screen.getByTestId("structure-commit"));
    });
    await act(async () => {
      await Promise.resolve();
    });

    expect(mockedRequestConfirm).toHaveBeenCalled();
    const ddlCall = mockedInvoke.mock.calls.find(
      ([name]) => name === "execute_ddl",
    );
    expect(ddlCall).toBeDefined();
    if (ddlCall) {
      const payload = (ddlCall[1] as { payload: { sql: string } }).payload;
      expect(payload.sql).toContain("DROP COLUMN");
    }
    expect(
      mockedInvoke.mock.calls.some(([name]) => name === "apply_object_ddl"),
    ).toBe(false);
  });
});

describe("TableStructureView PostgreSQL typed forms", () => {
  const renderView = () => {
    const key = seedStructure(baseStructure);
    render(
      <TableStructureView
        connectionId="conn-1"
        schema="public"
        tableName="users"
      />,
    );
    settleSuccess(key);
    return key;
  };

  const lastQueuedOp = (key: string) => {
    const pending = useAppStore.getState().pendingStructureChanges[key] ?? [];
    const entry = pending[pending.length - 1];
    if (!entry || entry.change.kind !== "pg-op") {
      throw new Error("expected a queued pg-op");
    }
    return entry.change.op;
  };

  it("queues addColumn with a tagged expression default", () => {
    const key = renderView();
    act(() => {
      fireEvent.click(screen.getByTestId("structure-add-column"));
    });
    fireEvent.change(screen.getByTestId("structure-add-column-name"), {
      target: { value: "created_at" },
    });
    fireEvent.change(screen.getByTestId("structure-add-column-type"), {
      target: { value: "timestamptz" },
    });
    fireEvent.change(screen.getByTestId("structure-add-column-default-kind"), {
      target: { value: "expression" },
    });
    fireEvent.change(screen.getByTestId("structure-add-column-default"), {
      target: { value: "now()" },
    });
    act(() => {
      fireEvent.click(screen.getByTestId("structure-add-column-submit"));
    });

    expect(lastQueuedOp(key)).toEqual({
      op: "addColumn",
      schema: "public",
      table: "users",
      column: {
        name: "created_at",
        dataType: "timestamptz",
        nullable: true,
        default: { kind: "expression", sql: "now()" },
      },
    });
  });

  it("queues addColumn with a tagged literal default", () => {
    const key = renderView();
    act(() => {
      fireEvent.click(screen.getByTestId("structure-add-column"));
    });
    fireEvent.change(screen.getByTestId("structure-add-column-name"), {
      target: { value: "status" },
    });
    fireEvent.change(screen.getByTestId("structure-add-column-type"), {
      target: { value: "text" },
    });
    fireEvent.change(screen.getByTestId("structure-add-column-default"), {
      target: { value: "active" },
    });
    act(() => {
      fireEvent.click(screen.getByTestId("structure-add-column-submit"));
    });

    expect(lastQueuedOp(key)).toEqual({
      op: "addColumn",
      schema: "public",
      table: "users",
      column: {
        name: "status",
        dataType: "text",
        nullable: true,
        default: { kind: "literal", value: "active" },
      },
    });
  });

  it("threads USING through a queued type change", () => {
    const key = renderView();
    act(() => {
      fireEvent.click(screen.getByTestId("structure-edit-column-email"));
    });
    fireEvent.change(screen.getByTestId("structure-type-input-email"), {
      target: { value: "integer" },
    });
    fireEvent.change(screen.getByTestId("structure-using-input-email"), {
      target: { value: "email::integer" },
    });
    act(() => {
      fireEvent.click(screen.getByTestId("structure-type-confirm-email"));
    });

    expect(lastQueuedOp(key)).toEqual({
      op: "alterColumnType",
      schema: "public",
      table: "users",
      name: "email",
      newType: "integer",
      using: "email::integer",
    });
  });

  it("queues a tagged literal default from the edit panel", () => {
    const key = renderView();
    act(() => {
      fireEvent.click(screen.getByTestId("structure-edit-column-email"));
    });
    fireEvent.change(screen.getByTestId("structure-default-input-email"), {
      target: { value: "unknown@example.com" },
    });
    fireEvent.change(screen.getByTestId("structure-default-kind-email"), {
      target: { value: "literal" },
    });
    act(() => {
      fireEvent.click(screen.getByTestId("structure-default-confirm-email"));
    });

    expect(lastQueuedOp(key)).toEqual({
      op: "setColumnDefault",
      schema: "public",
      table: "users",
      name: "email",
      default: { kind: "literal", value: "unknown@example.com" },
    });
  });

  it("queues renameColumn from the edit panel", () => {
    const key = renderView();
    act(() => {
      fireEvent.click(screen.getByTestId("structure-edit-column-email"));
    });
    fireEvent.change(screen.getByTestId("structure-rename-input-email"), {
      target: { value: "contact_email" },
    });
    act(() => {
      fireEvent.click(screen.getByTestId("structure-rename-confirm-email"));
    });

    expect(lastQueuedOp(key)).toEqual({
      op: "renameColumn",
      schema: "public",
      table: "users",
      name: "email",
      newName: "contact_email",
    });
  });

  it("queues a default clear as setColumnDefault null", () => {
    const key = renderView();
    act(() => {
      fireEvent.click(screen.getByTestId("structure-edit-column-is_active"));
    });
    fireEvent.change(screen.getByTestId("structure-default-input-is_active"), {
      target: { value: "" },
    });
    act(() => {
      fireEvent.click(
        screen.getByTestId("structure-default-confirm-is_active"),
      );
    });

    expect(lastQueuedOp(key)).toEqual({
      op: "setColumnDefault",
      schema: "public",
      table: "users",
      name: "is_active",
      default: null,
    });
  });

  it("queues setColumnNullable from the nullable toggle", () => {
    const key = renderView();
    act(() => {
      fireEvent.click(screen.getByTestId("structure-toggle-nullable-email"));
    });
    expect(lastQueuedOp(key)).toEqual({
      op: "setColumnNullable",
      schema: "public",
      table: "users",
      name: "email",
      nullable: false,
    });
  });
});

describe("TableStructureView PostgreSQL lifecycle affordances", () => {
  const renderView = (structure = baseStructure) => {
    const key = seedStructure(structure);
    render(
      <TableStructureView
        connectionId="conn-1"
        schema="public"
        tableName="users"
      />,
    );
    settleSuccess(key);
    return key;
  };

  const queuedOps = (key: string) =>
    (useAppStore.getState().pendingStructureChanges[key] ?? []).map((entry) => {
      if (entry.change.kind !== "pg-op") {
        throw new Error("expected a queued pg-op");
      }
      return entry.change.op;
    });

  it("queues dropConstraint from foreign-key and constraint rows", () => {
    const key = renderView();
    act(() => {
      fireEvent.click(
        screen.getByTestId("structure-drop-fk-users_org_id_fkey"),
      );
    });
    act(() => {
      fireEvent.click(
        screen.getByTestId("structure-drop-constraint-users_email_check"),
      );
    });
    expect(queuedOps(key)).toEqual([
      {
        op: "dropConstraint",
        schema: "public",
        table: "users",
        name: "users_org_id_fkey",
        cascade: false,
      },
      {
        op: "dropConstraint",
        schema: "public",
        table: "users",
        name: "users_email_check",
        cascade: false,
      },
    ]);
  });

  it("queues dropIndex with the chosen concurrently flag and shields the primary index", () => {
    const key = renderView();
    expect(screen.queryByTestId("structure-drop-index-users_pkey")).toBeNull();

    act(() => {
      fireEvent.click(
        screen.getByTestId("structure-drop-index-users_email_idx"),
      );
    });
    act(() => {
      fireEvent.click(
        screen.getByTestId("structure-drop-index-concurrently-users_email_idx"),
      );
    });
    act(() => {
      fireEvent.click(
        screen.getByTestId("structure-drop-index-submit-users_email_idx"),
      );
    });

    expect(queuedOps(key)).toEqual([
      {
        op: "dropIndex",
        schema: "public",
        name: "users_email_idx",
        concurrently: true,
        cascade: false,
      },
    ]);
  });

  it("queues createIndex from the New index form", () => {
    const key = renderView();
    act(() => {
      fireEvent.click(screen.getByTestId("structure-add-index"));
    });
    fireEvent.change(screen.getByTestId("structure-index-name"), {
      target: { value: "users_active_idx" },
    });
    fireEvent.change(screen.getByTestId("structure-index-columns"), {
      target: { value: "is_active, email" },
    });
    fireEvent.change(screen.getByTestId("structure-index-where"), {
      target: { value: "is_active" },
    });
    act(() => {
      fireEvent.click(screen.getByTestId("structure-index-unique"));
    });
    act(() => {
      fireEvent.click(screen.getByTestId("structure-index-submit"));
    });

    expect(queuedOps(key)).toEqual([
      {
        op: "createIndex",
        schema: "public",
        table: "users",
        name: "users_active_idx",
        unique: true,
        method: "btree",
        columns: [
          { expression: "is_active", descending: false },
          { expression: "email", descending: false },
        ],
        include: [],
        wherePredicate: "is_active",
        concurrently: true,
      },
    ]);
  });

  it("queues addForeignKey from the Add foreign key form", () => {
    const key = renderView();
    act(() => {
      fireEvent.click(screen.getByTestId("structure-add-fk"));
    });
    fireEvent.change(screen.getByTestId("structure-fk-columns"), {
      target: { value: "org_id" },
    });
    fireEvent.change(screen.getByTestId("structure-fk-ref-table"), {
      target: { value: "orgs" },
    });
    fireEvent.change(screen.getByTestId("structure-fk-ref-columns"), {
      target: { value: "id" },
    });
    fireEvent.change(screen.getByTestId("structure-fk-on-delete"), {
      target: { value: "cascade" },
    });
    act(() => {
      fireEvent.click(screen.getByTestId("structure-fk-submit"));
    });

    expect(queuedOps(key)).toEqual([
      {
        op: "addForeignKey",
        schema: "public",
        table: "users",
        name: null,
        columns: ["org_id"],
        referencedSchema: "public",
        referencedTable: "orgs",
        referencedColumns: ["id"],
        onUpdate: "no-action",
        onDelete: "cascade",
        deferrable: false,
        initiallyDeferred: false,
        notValid: false,
      },
    ]);
  });

  it("queues addCheck and addUnique, and hides Add primary key when a PK exists", () => {
    const key = renderView();
    expect(screen.queryByTestId("structure-add-pk")).toBeNull();

    act(() => {
      fireEvent.click(screen.getByTestId("structure-add-check"));
    });
    fireEvent.change(screen.getByTestId("structure-check-expression"), {
      target: { value: "email <> ''" },
    });
    act(() => {
      fireEvent.click(screen.getByTestId("structure-check-submit"));
    });

    act(() => {
      fireEvent.click(screen.getByTestId("structure-add-unique"));
    });
    fireEvent.change(screen.getByTestId("structure-unique-columns"), {
      target: { value: "email" },
    });
    act(() => {
      fireEvent.click(screen.getByTestId("structure-unique-submit"));
    });

    expect(queuedOps(key)).toEqual([
      {
        op: "addCheck",
        schema: "public",
        table: "users",
        name: null,
        expression: "email <> ''",
        notValid: false,
      },
      {
        op: "addUnique",
        schema: "public",
        table: "users",
        name: null,
        columns: ["email"],
      },
    ]);
  });

  it("offers Add primary key only when the table has none", () => {
    const key = renderView({
      ...baseStructure,
      primaryKey: null,
      columns: baseStructure.columns.map((column) => ({
        ...column,
        isPrimaryKey: false,
      })),
      indexes: baseStructure.indexes.filter((index) => !index.isPrimary),
    });
    act(() => {
      fireEvent.click(screen.getByTestId("structure-add-pk"));
    });
    fireEvent.change(screen.getByTestId("structure-pk-columns"), {
      target: { value: "id" },
    });
    act(() => {
      fireEvent.click(screen.getByTestId("structure-pk-submit"));
    });

    expect(queuedOps(key)).toEqual([
      {
        op: "addPrimaryKey",
        schema: "public",
        table: "users",
        name: null,
        columns: ["id"],
      },
    ]);
  });

  it("offers zero-argument trigger-returning functions without conflating schemas", () => {
    const publicReference = {
      kind: "function",
      schema: "public",
      name: "route_event",
      identityArgs: "",
    } as const;
    const auditReference = {
      kind: "function",
      schema: "audit",
      name: "route_event",
      identityArgs: "",
    } as const;
    const parameterizedReference = {
      kind: "function",
      schema: "audit",
      name: "route_event",
      identityArgs: "integer",
    } as const;
    const numericReference = {
      kind: "function",
      schema: "public",
      name: "calculate_total",
      identityArgs: "",
    } as const;
    const description = (
      reference: {
        kind: "function";
        schema: string;
        name: string;
        identityArgs: string;
      },
      returns: string,
    ) => ({
      status: "ready" as const,
      generation: 0,
      description: {
        reference,
        owner: "postgres",
        comment: null,
        definitionSql: null,
        facts: {
          kind: "routine" as const,
          language: "plpgsql",
          returns,
          volatility: "volatile",
          arguments: reference.identityArgs,
          body: "BEGIN RETURN NEW; END;",
          strict: false,
          securityDefiner: false,
          parallel: "unsafe",
        },
      },
    });
    useAppStore.setState({
      pgObjectCatalog: {
        "conn-1": {
          status: "ready",
          generation: 0,
          catalog: {
            schemas: [
              {
                name: "public",
                tables: [],
                views: [],
                materializedViews: [],
                foreignTables: [],
                sequences: [],
                functions: [
                  { name: "route_event", identityArgs: "" },
                  { name: "calculate_total", identityArgs: "" },
                ],
                procedures: [],
                aggregates: [],
                types: [],
                domains: [],
                extensions: [],
              },
              {
                name: "audit",
                tables: [],
                views: [],
                materializedViews: [],
                foreignTables: [],
                sequences: [],
                functions: [
                  { name: "route_event", identityArgs: "" },
                  { name: "route_event", identityArgs: "integer" },
                ],
                procedures: [],
                aggregates: [],
                types: [],
                domains: [],
                extensions: [],
              },
            ],
            eventTriggers: [],
            roles: [],
            tablespaces: [],
            truncated: [],
          },
        },
      },
      pgObjectDescriptions: {
        [pgObjectDescriptionKey("conn-1", publicReference)]: description(
          publicReference,
          "trigger",
        ),
        [pgObjectDescriptionKey("conn-1", auditReference)]: description(
          auditReference,
          "trigger",
        ),
        [pgObjectDescriptionKey("conn-1", parameterizedReference)]: description(
          parameterizedReference,
          "trigger",
        ),
        [pgObjectDescriptionKey("conn-1", numericReference)]: description(
          numericReference,
          "numeric",
        ),
      },
    });
    renderView({
      ...baseStructure,
      capabilities: { ...baseStructure.capabilities, triggers: true },
    });
    fireEvent.click(screen.getByRole("button", { name: "New trigger" }));

    const picker = screen.getByRole("combobox", { name: "Trigger function" });
    const list = document.getElementById(
      picker.getAttribute("list") ?? "missing",
    );
    expect(
      [...(list?.querySelectorAll("option") ?? [])].map(
        (option) => option.value,
      ),
    ).toEqual([
      "public.route_event() → trigger",
      "audit.route_event() → trigger",
    ]);
  });
});
