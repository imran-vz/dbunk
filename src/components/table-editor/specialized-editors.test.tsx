// @vitest-environment jsdom
import { act, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it } from "vitest";

import {
  type Connection,
  type TableStructure,
  tableStructureKey,
  useAppStore,
} from "@/lib/store";

import { SpecializedEditors } from "./specialized-editors";

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

const structure: TableStructure = {
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
      nullable: true,
      defaultValue: null,
      isPrimaryKey: false,
      ordinalPosition: 2,
    },
  ],
  primaryKey: ["id"],
  foreignKeys: [],
  indexes: [],
  constraints: [],
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

const key = tableStructureKey("conn-1", "public", "users");

beforeEach(() => {
  useAppStore.setState(initialStoreState, true);
  useAppStore.setState({
    connections: [connection],
    activeConnectionId: connection.id,
  });
});

afterEach(() => {
  useAppStore.setState(initialStoreState, true);
});

const renderEditors = () =>
  render(
    <SpecializedEditors
      schema="public"
      table="users"
      connectionId="conn-1"
      structure={structure}
    />,
  );

describe("SpecializedEditors typed op queueing", () => {
  it("queues a createIndex op into the shared pending structure list", () => {
    renderEditors();
    act(() => {
      fireEvent.click(screen.getByRole("button", { name: "Queue index" }));
    });

    const pending = useAppStore.getState().pendingStructureChanges[key] ?? [];
    expect(pending).toHaveLength(1);
    expect(pending[0].change).toEqual({
      kind: "pg-op",
      op: {
        op: "createIndex",
        schema: "public",
        table: "users",
        name: "users_id_idx",
        unique: false,
        method: "btree",
        columns: [{ expression: "id", descending: false }],
        include: [],
        wherePredicate: null,
        concurrently: true,
      },
    });
    expect(screen.getByTestId("specialized-queue-notice").textContent).toMatch(
      /queued/i,
    );
  });

  it("queues an addForeignKey op with mapped referential actions", () => {
    renderEditors();
    act(() => {
      fireEvent.click(
        screen.getByRole("button", { name: "Queue foreign key" }),
      );
    });

    const pending = useAppStore.getState().pendingStructureChanges[key] ?? [];
    expect(pending).toHaveLength(1);
    expect(pending[0].change).toEqual({
      kind: "pg-op",
      op: {
        op: "addForeignKey",
        schema: "public",
        table: "users",
        name: "users_id_fk",
        columns: ["id"],
        referencedSchema: "public",
        referencedTable: "referenced_table",
        referencedColumns: ["id"],
        onUpdate: "no-action",
        onDelete: "no-action",
        deferrable: false,
        initiallyDeferred: false,
        notValid: false,
      },
    });
  });

  it("surfaces a failed notice when the pending list holds legacy entries", () => {
    useAppStore.setState({
      pendingStructureChanges: {
        [key]: [
          {
            id: "p-1",
            schema: "public",
            table: "users",
            change: {
              kind: "column",
              change: { kind: "drop", columnName: "email" },
            },
          },
        ],
      },
    });
    renderEditors();
    act(() => {
      fireEvent.click(screen.getByRole("button", { name: "Queue index" }));
    });

    expect(
      screen.getByTestId("specialized-queue-notice-failed").textContent,
    ).toMatch(/cannot mix/i);
    expect(useAppStore.getState().pendingStructureChanges[key]).toHaveLength(1);
  });

  it("keeps the GRANT panel on generate-only SQL strings", () => {
    renderEditors();
    act(() => {
      fireEvent.click(screen.getByRole("button", { name: "Generate GRANT" }));
    });
    expect(screen.getByText(/GRANT SELECT ON TABLE/)).toBeTruthy();
    expect(useAppStore.getState().pendingStructureChanges[key]).toBeUndefined();
  });

  it("keeps the RLS panel on generate-only SQL strings", () => {
    renderEditors();
    act(() => {
      fireEvent.click(screen.getByRole("button", { name: "Generate policy" }));
    });
    expect(screen.getByText(/CREATE POLICY/)).toBeTruthy();
    expect(useAppStore.getState().pendingStructureChanges[key]).toBeUndefined();
  });

  it("keeps the trigger panel on generate-only SQL strings", () => {
    renderEditors();
    act(() => {
      fireEvent.click(screen.getByRole("button", { name: "Generate trigger" }));
    });
    expect(screen.getByText(/CREATE TRIGGER/)).toBeTruthy();
    expect(useAppStore.getState().pendingStructureChanges[key]).toBeUndefined();
  });
});

describe("SpecializedEditors on non-PostgreSQL engines", () => {
  beforeEach(() => {
    const clickhouse: Connection = {
      id: "conn-1",
      name: "Local CH",
      database: "dbunk",
      status: "Connected",
      engine: "ClickHouse",
      host: "localhost",
      port: 8123,
      user: "default",
      password: "",
      role: "admin",
      latency: "10 ms",
      useHttps: false,
      urlPath: "",
    };
    useAppStore.setState({
      connections: [clickhouse],
      activeConnectionId: clickhouse.id,
    });
  });

  it("keeps the index panel on the generate-SQL fallback", () => {
    renderEditors();
    act(() => {
      fireEvent.click(screen.getByRole("button", { name: "Generate index" }));
    });
    expect(screen.getByText(/CREATE INDEX/)).toBeTruthy();
    expect(useAppStore.getState().pendingStructureChanges[key]).toBeUndefined();
  });

  it("keeps the FK panel on the generate-SQL fallback", () => {
    renderEditors();
    act(() => {
      fireEvent.click(screen.getByRole("button", { name: "Generate FK" }));
    });
    expect(screen.getByText(/ADD CONSTRAINT/)).toBeTruthy();
    expect(useAppStore.getState().pendingStructureChanges[key]).toBeUndefined();
  });
});
