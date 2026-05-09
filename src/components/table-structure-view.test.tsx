import { act, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("@/lib/tauri", () => ({
  isTauri: vi.fn(() => true),
  tauriInvoke: vi.fn(),
}));

import { TableStructureView } from "@/components/table-structure-view";
import {
  type Connection,
  type TableStructure,
  tableStructureKey,
  useAppStore,
} from "@/lib/store";
import { tauriInvoke } from "@/lib/tauri";

const mockedInvoke = vi.mocked(tauriInvoke);

const initialStoreState = useAppStore.getState();

const seedConnection = (engine: Connection["engine"] = "PostgreSQL") => {
  const connection: Connection = {
    id: "conn-1",
    name: "Local",
    database: "dbunk",
    status: "Connected",
    engine,
    host: "localhost",
    port: 5432,
    user: "postgres",
    password: "",
    role: "admin",
    latency: "10 ms",
    lastSync: "Just now",
  };
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
  capabilities: {
    columns: true,
    primaryKey: true,
    foreignKeys: true,
    indexes: true,
    constraints: true,
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
