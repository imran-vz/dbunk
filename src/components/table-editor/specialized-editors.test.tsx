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
  it("resets every table-scoped form before queueing against a new table", () => {
    const { rerender } = renderEditors();

    fireEvent.change(screen.getByDisplayValue("users_id_idx"), {
      target: { value: "stale_users_idx" },
    });
    fireEvent.change(screen.getByDisplayValue("users_id_fk"), {
      target: { value: "stale_users_fk" },
    });
    fireEvent.change(
      screen.getByRole("combobox", { name: "Grant grantee kind" }),
      { target: { value: "role" } },
    );
    fireEvent.change(screen.getByRole("textbox", { name: "Grant role" }), {
      target: { value: "stale_users_role" },
    });
    fireEvent.change(screen.getByDisplayValue("users_access_policy"), {
      target: { value: "stale_users_policy" },
    });
    fireEvent.change(screen.getByDisplayValue("users_updated_at"), {
      target: { value: "stale_users_trigger" },
    });

    rerender(
      <SpecializedEditors
        schema="public"
        table="orders"
        connectionId="conn-1"
        structure={structure}
      />,
    );

    expect(screen.getByDisplayValue("orders_id_idx")).toBeTruthy();
    expect(screen.getByDisplayValue("orders_id_fk")).toBeTruthy();
    expect(
      screen.getByRole<HTMLSelectElement>("combobox", {
        name: "Grant grantee kind",
      }).value,
    ).toBe("public");
    expect(screen.queryByRole("textbox", { name: "Grant role" })).toBeNull();
    expect(screen.getByDisplayValue("orders_access_policy")).toBeTruthy();
    expect(screen.getByDisplayValue("orders_updated_at")).toBeTruthy();

    fireEvent.click(screen.getByRole("button", { name: "Queue index" }));

    const usersKey = tableStructureKey("conn-1", "public", "users");
    const ordersKey = tableStructureKey("conn-1", "public", "orders");
    expect(
      useAppStore.getState().pendingStructureChanges[usersKey],
    ).toBeUndefined();
    expect(
      useAppStore.getState().pendingStructureChanges[ordersKey]?.[0]?.change,
    ).toMatchObject({
      kind: "pg-op",
      op: { table: "orders", name: "orders_id_idx" },
    });
  });

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

  it("queues a table grant to PUBLIC by default and hides sequence targets", () => {
    renderEditors();
    const target = screen.getByRole("combobox", {
      name: "Grant object type",
    });
    expect(
      [...target.querySelectorAll("option")].map((option) => option.value),
    ).toEqual(["TABLE"]);
    act(() => {
      fireEvent.click(screen.getByRole("button", { name: "Queue grant" }));
    });
    expect(
      useAppStore.getState().pendingStructureChanges[key]?.[0]?.change,
    ).toMatchObject({
      kind: "pg-op",
      op: {
        op: "grantPrivileges",
        target: { kind: "table", schema: "public", name: "users" },
        privileges: ["select"],
        grantee: { kind: "public" },
      },
    });
  });

  it("disables typed grant and trigger queueing when every choice is removed", () => {
    renderEditors();
    const queueGrant = screen.getByRole<HTMLButtonElement>("button", {
      name: "Queue grant",
    });
    const queueTrigger = screen.getByRole<HTMLButtonElement>("button", {
      name: "Queue trigger",
    });

    fireEvent.click(screen.getByRole("button", { name: "SELECT" }));
    const triggerUpdate = screen
      .getAllByRole("button", { name: "UPDATE" })
      .at(-1);
    expect(triggerUpdate).toBeDefined();
    if (!triggerUpdate) throw new Error("Trigger UPDATE choice is missing.");
    fireEvent.click(triggerUpdate);

    expect(queueGrant.disabled).toBe(true);
    expect(queueTrigger.disabled).toBe(true);
    fireEvent.click(queueGrant);
    fireEvent.click(queueTrigger);
    expect(useAppStore.getState().pendingStructureChanges[key]).toBeUndefined();
  });

  it("keeps lowercase public addressable as a quoted role", () => {
    renderEditors();
    fireEvent.change(
      screen.getByRole("combobox", { name: "Grant grantee kind" }),
      { target: { value: "role" } },
    );
    fireEvent.change(screen.getByRole("textbox", { name: "Grant role" }), {
      target: { value: "public" },
    });
    act(() => {
      fireEvent.click(screen.getByRole("button", { name: "Queue grant" }));
    });

    expect(
      useAppStore.getState().pendingStructureChanges[key]?.[0]?.change,
    ).toMatchObject({
      kind: "pg-op",
      op: { grantee: { kind: "role", name: "public" } },
    });
  });

  it("keeps an uppercase PUBLIC role distinct from the pseudo-role", () => {
    renderEditors();
    fireEvent.change(
      screen.getByRole("combobox", { name: "Grant grantee kind" }),
      { target: { value: "role" } },
    );
    fireEvent.change(screen.getByRole("textbox", { name: "Grant role" }), {
      target: { value: "PUBLIC" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Queue grant" }));

    expect(
      useAppStore.getState().pendingStructureChanges[key]?.[0]?.change,
    ).toMatchObject({
      kind: "pg-op",
      op: { grantee: { kind: "role", name: "PUBLIC" } },
    });
  });

  it("queues RLS and policy ops in order", () => {
    renderEditors();
    act(() => {
      fireEvent.click(screen.getByRole("button", { name: "Queue policy" }));
    });
    expect(
      useAppStore
        .getState()
        .pendingStructureChanges[key]?.map((entry) =>
          entry.change.kind === "pg-op" ? entry.change.op.op : "column",
        ),
    ).toEqual(["setRowLevelSecurity", "createPolicy"]);
  });

  it("omits USING from INSERT policies and keeps WITH CHECK", () => {
    renderEditors();
    fireEvent.change(screen.getByRole("combobox", { name: "Policy command" }), {
      target: { value: "INSERT" },
    });
    const using = screen.getByRole<HTMLTextAreaElement>("textbox", {
      name: "Policy USING expression",
    });
    const withCheck = screen.getByRole<HTMLTextAreaElement>("textbox", {
      name: "Policy WITH CHECK expression",
    });
    expect(using.disabled).toBe(true);
    expect(withCheck.disabled).toBe(false);
    fireEvent.change(withCheck, { target: { value: "tenant_id = 7" } });
    fireEvent.click(screen.getByRole("button", { name: "Queue policy" }));

    expect(
      useAppStore
        .getState()
        .pendingStructureChanges[key]?.find(
          (entry) =>
            entry.change.kind === "pg-op" &&
            entry.change.op.op === "createPolicy",
        )?.change,
    ).toMatchObject({
      kind: "pg-op",
      op: {
        command: "insert",
        using: null,
        withCheck: "tenant_id = 7",
      },
    });
  });

  it("requires statement orientation for typed TRUNCATE triggers", () => {
    renderEditors();
    const truncate = screen.getAllByRole("button", { name: "TRUNCATE" }).at(-1);
    if (!truncate) throw new Error("Trigger TRUNCATE choice is missing.");
    fireEvent.click(truncate);

    const queueTrigger = screen.getByRole<HTMLButtonElement>("button", {
      name: "Queue trigger",
    });
    expect(queueTrigger.disabled).toBe(true);
    const timing = screen.getByRole<HTMLSelectElement>("combobox", {
      name: "Trigger timing",
    });
    expect([...timing.options].map((option) => option.value)).toEqual([
      "BEFORE",
      "AFTER",
    ]);
    fireEvent.change(
      screen.getByRole("combobox", { name: "Trigger orientation" }),
      { target: { value: "STATEMENT" } },
    );
    expect(queueTrigger.disabled).toBe(false);
    fireEvent.click(queueTrigger);

    expect(
      useAppStore
        .getState()
        .pendingStructureChanges[key]?.find(
          (entry) =>
            entry.change.kind === "pg-op" &&
            entry.change.op.op === "createTrigger",
        )?.change,
    ).toMatchObject({
      kind: "pg-op",
      op: {
        forEach: "statement",
        events: expect.arrayContaining([{ kind: "truncate" }]),
      },
    });
  });

  it("keeps a quoted PUBLIC policy role distinct from the pseudo-role", () => {
    renderEditors();
    fireEvent.change(
      screen.getByRole("combobox", { name: "Policy grantee kind" }),
      { target: { value: "role" } },
    );
    fireEvent.change(screen.getByRole("textbox", { name: "Policy role" }), {
      target: { value: "PUBLIC" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Queue policy" }));

    expect(
      useAppStore
        .getState()
        .pendingStructureChanges[key]?.find(
          (entry) =>
            entry.change.kind === "pg-op" &&
            entry.change.op.op === "createPolicy",
        )?.change,
    ).toMatchObject({
      kind: "pg-op",
      op: { roles: [{ kind: "role", name: "PUBLIC" }] },
    });
  });

  it("clears and locks FORCE RLS when RLS is disabled", () => {
    renderEditors();
    const force = screen.getByRole<HTMLInputElement>("checkbox", {
      name: "FORCE RLS",
    });
    const enabled = screen.getByRole<HTMLInputElement>("checkbox", {
      name: "Enable RLS",
    });

    fireEvent.click(force);
    expect(force.checked).toBe(true);
    fireEvent.click(enabled);
    expect(enabled.checked).toBe(false);
    expect(force.checked).toBe(false);
    expect(force.disabled).toBe(true);

    act(() => {
      fireEvent.click(screen.getByRole("button", { name: "Queue policy" }));
    });
    expect(
      useAppStore.getState().pendingStructureChanges[key]?.[0]?.change,
    ).toEqual({
      kind: "pg-op",
      op: {
        op: "setRowLevelSecurity",
        schema: "public",
        table: "users",
        enabled: false,
        force: false,
      },
    });
  });

  it("queues trigger function and trigger ops in order", () => {
    renderEditors();
    act(() => {
      fireEvent.click(screen.getByRole("button", { name: "Queue trigger" }));
    });
    expect(
      useAppStore
        .getState()
        .pendingStructureChanges[key]?.map((entry) =>
          entry.change.kind === "pg-op" ? entry.change.op.op : "column",
        ),
    ).toEqual(["createFunction", "createTrigger"]);
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

  it("preserves FORCE RLS with disabled RLS and generates the legacy SQL byte-for-byte", () => {
    renderEditors();
    const force = screen.getByRole<HTMLInputElement>("checkbox", {
      name: "FORCE RLS",
    });
    const enabled = screen.getByRole<HTMLInputElement>("checkbox", {
      name: "Enable RLS",
    });

    fireEvent.click(force);
    fireEvent.click(enabled);

    expect(enabled.checked).toBe(false);
    expect(force.checked).toBe(true);
    expect(force.disabled).toBe(false);

    act(() => {
      fireEvent.click(screen.getByRole("button", { name: "Generate policy" }));
    });
    expect(
      screen
        .getByText("Generated SQL / literal")
        .closest("section")
        ?.querySelector("pre")?.textContent,
    ).toBe(
      [
        'ALTER TABLE "public"."users" DISABLE ROW LEVEL SECURITY;',
        'ALTER TABLE "public"."users" FORCE ROW LEVEL SECURITY;',
        'CREATE POLICY "users_access_policy" ON "public"."users"',
        "  AS PERMISSIVE FOR ALL",
        "  TO PUBLIC",
        "  USING (true);",
      ].join("\n"),
    );
    expect(useAppStore.getState().pendingStructureChanges[key]).toBeUndefined();
  });

  it("keeps GRANT, RLS, and trigger SQL fallbacks byte-for-byte", () => {
    renderEditors();
    const generatedSql = () =>
      screen
        .getByText("Generated SQL / literal")
        .closest("section")
        ?.querySelector("pre")?.textContent;
    expect(
      [
        ...screen
          .getByRole("combobox", { name: "Grant object type" })
          .querySelectorAll("option"),
      ].map((option) => option.value),
    ).toEqual(["TABLE", "SEQUENCE"]);
    act(() => {
      fireEvent.click(screen.getByRole("button", { name: "Generate GRANT" }));
    });
    expect(generatedSql()).toBe(
      'GRANT SELECT ON TABLE "public"."users" TO PUBLIC;',
    );
    act(() => {
      fireEvent.click(screen.getByRole("button", { name: "Generate policy" }));
    });
    expect(generatedSql()).toBe(
      [
        'ALTER TABLE "public"."users" ENABLE ROW LEVEL SECURITY;',
        'CREATE POLICY "users_access_policy" ON "public"."users"',
        "  AS PERMISSIVE FOR ALL",
        "  TO PUBLIC",
        "  USING (true);",
      ].join("\n"),
    );
    act(() => {
      fireEvent.click(screen.getByRole("button", { name: "Generate trigger" }));
    });
    expect(generatedSql()).toBe(
      [
        'CREATE OR REPLACE FUNCTION "public"."users_set_updated_at"()',
        "RETURNS trigger AS $$",
        "BEGIN",
        "  NEW.updated_at = now();",
        "  RETURN NEW;",
        "END;",
        "$$ LANGUAGE plpgsql;",
        "",
        'CREATE TRIGGER "users_updated_at"',
        'BEFORE UPDATE ON "public"."users"',
        "FOR EACH ROW",
        'EXECUTE FUNCTION "public"."users_set_updated_at"();',
      ].join("\n"),
    );
    expect(useAppStore.getState().pendingStructureChanges[key]).toBeUndefined();
  });
});
