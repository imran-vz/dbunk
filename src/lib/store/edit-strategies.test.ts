/* oxlint-disable anti-slop/no-chained-type-assertions anti-slop/require-safety-comment-for-type-assertion -- Test fixtures use controlled mocks and assertions to exercise otherwise inaccessible boundaries. */
import { describe, expect, it } from "vitest";

import {
  buildDeleteRowsPayload,
  buildEditPayload,
  findTableData,
  resolveEditContext,
  resolveStructureCommitContext,
} from "./edit-strategies";
import type {
  Connection,
  StructureCapabilities,
  TableDataState,
  TableStructure,
} from "./types";

const baseCapabilities: StructureCapabilities = {
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
};

const makeStructure = (
  caps: Partial<StructureCapabilities> = {},
): TableStructure => ({
  columns: [
    {
      name: "id",
      dataType: "int",
      nullable: false,
      defaultValue: null,
      isPrimaryKey: true,
      ordinalPosition: 1,
    },
    {
      name: "name",
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
  capabilities: { ...baseCapabilities, ...caps },
});

const makeData = (overrides: Partial<TableDataState> = {}): TableDataState => ({
  connectionId: "conn-1",
  schema: "public",
  table: "users",
  columns: ["id", "name"],
  rows: [
    ["1", "alice"],
    ["2", "bob"],
  ],
  page: 1,
  pageSize: 100,
  runtimeMs: 5,
  ...overrides,
});

const pgConnection: Connection = {
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

describe("edit-strategies.findTableData", () => {
  it("returns the matching key/data pair", () => {
    const data = makeData();
    const found = findTableData({ "conn-1::public::users": data }, "users");
    expect(found).not.toBeNull();
    if (found) {
      expect(found[0]).toBe("conn-1::public::users");
      expect(found[1]).toBe(data);
    }
  });

  it("returns null when no entry has a matching table name", () => {
    const data = makeData();
    expect(findTableData({ k: data }, "missing")).toBeNull();
  });
});

describe("edit-strategies.buildEditPayload", () => {
  const columnIndexByName = new Map([
    ["id", 0],
    ["name", 1],
  ]);
  const identity = { columns: ["id"] };

  it("returns one payload per changed row with identity and set entries", () => {
    const data = makeData();
    const edits = { 0: { 1: "ALICE" } };

    const payload = buildEditPayload(edits, data, identity, columnIndexByName);

    expect(payload).toHaveLength(1);
    expect(payload[0]).toEqual({
      rowIndex: 0,
      identity: [{ column: "id", value: "1" }],
      set: [{ column: "name", value: "ALICE" }],
    });
  });

  it("skips columns whose new value equals the original", () => {
    const data = makeData();
    // colIndex 1 ("name") matches the existing row value — should be filtered.
    const edits = { 0: { 1: "alice" } };

    const payload = buildEditPayload(edits, data, identity, columnIndexByName);
    expect(payload).toHaveLength(0);
  });

  it("skips rows that vanished from the loaded data", () => {
    const data = makeData();
    const edits = { 99: { 1: "ghost" } };

    const payload = buildEditPayload(edits, data, identity, columnIndexByName);
    expect(payload).toHaveLength(0);
  });

  it("skips unknown column indices and undefined values", () => {
    const data = makeData();
    const edits = {
      0: {
        // unknown column index → filtered
        99: "x",
        // undefined value → filtered
        1: undefined as unknown as string,
      },
    };

    const payload = buildEditPayload(edits, data, identity, columnIndexByName);
    expect(payload).toHaveLength(0);
  });

  it("emits rows in ascending order and columns in ascending order", () => {
    const data = makeData({
      rows: [
        ["1", "a"],
        ["2", "b"],
      ],
    });
    const edits = {
      // Out-of-order keys
      1: { 1: "B" },
      0: { 1: "A" },
    };

    const payload = buildEditPayload(edits, data, identity, columnIndexByName);
    expect(payload.map((p) => p.rowIndex)).toEqual([0, 1]);
  });

  it("keeps the legacy commit payload byte-stable", () => {
    const data = makeData({ rows: [["1"]] });
    const payload = buildEditPayload(
      { 0: { 1: "Ada" } },
      data,
      identity,
      columnIndexByName,
    );

    expect(JSON.stringify(payload)).toBe(
      '[{"rowIndex":0,"identity":[{"column":"id","value":"1"}],"set":[{"column":"name","value":"Ada"}]}]',
    );
  });
});

describe("edit-strategies.buildDeleteRowsPayload", () => {
  const columnIndexByName = new Map([
    ["id", 0],
    ["name", 1],
  ]);
  const identity = { columns: ["id"] };

  it("produces one identity tuple per existing row in ascending order", () => {
    const data = makeData();
    const payload = buildDeleteRowsPayload(
      [1, 0],
      data,
      identity,
      columnIndexByName,
    );
    expect(payload).toEqual([
      [{ column: "id", value: "1" }],
      [{ column: "id", value: "2" }],
    ]);
  });

  it("skips indices that point past the loaded rows", () => {
    const data = makeData();
    const payload = buildDeleteRowsPayload(
      [0, 99],
      data,
      identity,
      columnIndexByName,
    );
    expect(payload).toHaveLength(1);
  });

  it("returns [] for an empty input list", () => {
    expect(
      buildDeleteRowsPayload([], makeData(), identity, columnIndexByName),
    ).toEqual([]);
  });

  it("keeps the legacy delete payload byte-stable", () => {
    const payload = buildDeleteRowsPayload(
      [1, 0],
      makeData(),
      identity,
      columnIndexByName,
    );

    expect(JSON.stringify(payload)).toBe(
      '[[{"column":"id","value":"1"}],[{"column":"id","value":"2"}]]',
    );
  });
});

describe("edit-strategies.resolveEditContext", () => {
  const tableName = "users";
  const dataKey = "conn-1::public::users";
  const structureKey = "conn-1::public::users";

  it("returns failed when no table data is loaded", () => {
    const result = resolveEditContext({
      tableData: {},
      tableStructure: {},
      connections: [pgConnection],
      tableName,
      capability: "canUpdateRows",
      action: "cell edits",
    });
    expect(result.ok).toBe(false);
    if (!result.ok) {
      expect(result.reason).toMatch(/not loaded/i);
    }
  });

  it("uses the delete-specific reason message for canDeleteRows", () => {
    const result = resolveEditContext({
      tableData: {},
      tableStructure: {},
      connections: [pgConnection],
      tableName,
      capability: "canDeleteRows",
      action: "row deletes",
    });
    if (result.ok) throw new Error("expected error");
    expect(result.reason).toMatch(/delete rows/i);
  });

  it("returns failed when the table has no row identity", () => {
    const result = resolveEditContext({
      tableData: { [dataKey]: makeData() },
      // No structure → pickRowIdentity returns null.
      tableStructure: {},
      connections: [pgConnection],
      tableName,
      capability: "canUpdateRows",
      action: "cell edits",
    });
    if (result.ok) throw new Error("expected error");
    expect(result.reason).toMatch(/primary key|unique index/i);
  });

  it("returns failed when the connection has been removed", () => {
    const result = resolveEditContext({
      tableData: { [dataKey]: makeData() },
      tableStructure: { [structureKey]: makeStructure() },
      connections: [],
      tableName,
      capability: "canUpdateRows",
      action: "cell edits",
    });
    if (result.ok) throw new Error("expected error");
    expect(result.reason).toMatch(/connection not found/i);
  });

  it("returns the connection-scoped read-only refusal before editing", () => {
    const result = resolveEditContext({
      tableData: { [dataKey]: makeData() },
      tableStructure: { [structureKey]: makeStructure() },
      connections: [{ ...pgConnection, readOnly: true }],
      tableName,
      capability: "canUpdateRows",
      action: "cell edits",
    });
    if (result.ok) throw new Error("expected error");
    expect(result.reason).toBe(
      "Local is a read-only connection. Edit the connection to unlock writes.",
    );
  });

  it("returns failed when canUpdateRows is false", () => {
    const result = resolveEditContext({
      tableData: { [dataKey]: makeData() },
      tableStructure: {
        [structureKey]: makeStructure({ canUpdateRows: false }),
      },
      connections: [pgConnection],
      tableName,
      capability: "canUpdateRows",
      action: "cell edits",
    });
    if (result.ok) throw new Error("expected error");
    expect(result.reason).toMatch(/does not support/i);
  });

  it("returns failed when canDeleteRows is false (and a structure exists)", () => {
    const result = resolveEditContext({
      tableData: { [dataKey]: makeData() },
      tableStructure: {
        [structureKey]: makeStructure({ canDeleteRows: false }),
      },
      connections: [pgConnection],
      tableName,
      capability: "canDeleteRows",
      action: "row deletes",
    });
    if (result.ok) throw new Error("expected error");
    expect(result.reason).toMatch(/does not support/i);
  });

  it("returns failed when identity columns are missing from the loaded data", () => {
    const result = resolveEditContext({
      tableData: { [dataKey]: makeData({ columns: ["name"] }) },
      tableStructure: { [structureKey]: makeStructure() },
      connections: [pgConnection],
      tableName,
      capability: "canUpdateRows",
      action: "cell edits",
    });
    if (result.ok) throw new Error("expected error");
    expect(result.reason).toMatch(/identity column/i);
  });

  it("returns ok with resolved context on the happy path", () => {
    const data = makeData();
    const structure = makeStructure();
    const result = resolveEditContext({
      tableData: { [dataKey]: data },
      tableStructure: { [structureKey]: structure },
      connections: [pgConnection],
      tableName,
      capability: "canUpdateRows",
      action: "cell edits",
    });
    if (!result.ok) {
      throw new Error(`expected ok, got: ${result.reason}`);
    }
    expect(result.data).toBe(data);
    expect(result.dataKey).toBe(dataKey);
    expect(result.connection).toBe(pgConnection);
    expect(result.identity.columns).toEqual(["id"]);
    expect(result.columnIndexByName.get("id")).toBe(0);
    expect(result.columnIndexByName.get("name")).toBe(1);
  });

  it("uses authoritative unique-index identity despite PK-only structure capabilities", () => {
    const result = resolveEditContext({
      tableData: {},
      tableStructure: {
        [structureKey]: makeStructure({
          canUpdateRows: false,
          canDeleteRows: false,
        }),
      },
      connections: [pgConnection],
      ref: { connectionId: "conn-1", schema: "public", table: "users" },
      dataSource: {
        connectionId: "conn-1",
        schema: "public",
        table: "users",
        columns: ["email", "id"],
        rows: [["ada@x", "1"]],
        identityKind: "uniqueIndex",
        identityColumns: ["email"],
      },
      capability: "canUpdateRows",
      action: "cell edits",
    });
    if (!result.ok) throw new Error(`expected ok, got: ${result.reason}`);
    expect(result.identity.columns).toEqual(["email"]);
    expect(result.data.rows[0]?.[0]).toBe("ada@x");

    const deleteResult = resolveEditContext({
      tableData: {},
      tableStructure: {
        [structureKey]: makeStructure({ canDeleteRows: false }),
      },
      connections: [pgConnection],
      ref: { connectionId: "conn-1", schema: "public", table: "users" },
      dataSource: {
        connectionId: "conn-1",
        schema: "public",
        table: "users",
        columns: ["email", "id"],
        rows: [["ada@x", "1"]],
        identityKind: "uniqueIndex",
        identityColumns: ["email"],
      },
      capability: "canDeleteRows",
      action: "row deletes",
    });
    if (!deleteResult.ok) {
      throw new Error(`expected delete context, got: ${deleteResult.reason}`);
    }
    expect(deleteResult.identity.columns).toEqual(["email"]);
  });

  it("rejects virtual browse identity with honest copy", () => {
    const result = resolveEditContext({
      tableData: {},
      tableStructure: { [structureKey]: makeStructure() },
      connections: [pgConnection],
      ref: { connectionId: "conn-1", schema: "public", table: "heap" },
      dataSource: {
        connectionId: "conn-1",
        schema: "public",
        table: "heap",
        columns: ["note"],
        rows: [["x"]],
        identityKind: "virtual",
        identityColumns: ["ctid"],
      },
      capability: "canUpdateRows",
      action: "cell edits",
    });
    if (result.ok) throw new Error("expected error");
    expect(result.reason).toMatch(/virtual identity/i);
  });

  it("rejects none browse identity with honest copy", () => {
    const result = resolveEditContext({
      tableData: {},
      tableStructure: { [structureKey]: makeStructure() },
      connections: [pgConnection],
      ref: { connectionId: "conn-1", schema: "public", table: "foreign" },
      dataSource: {
        connectionId: "conn-1",
        schema: "public",
        table: "foreign",
        columns: ["note"],
        rows: [["x"]],
        identityKind: "none",
        identityColumns: [],
      },
      capability: "canDeleteRows",
      action: "row deletes",
    });
    if (result.ok) throw new Error("expected error");
    expect(result.reason).toMatch(/no usable row identity/i);
  });
});

describe("edit-strategies.resolveStructureCommitContext", () => {
  const key = "conn-1::public::users";
  const pending = [{ schema: "public", table: "users" }];

  it("returns failed when the connection is missing", () => {
    const result = resolveStructureCommitContext({
      pending,
      key,
      connections: [],
      tableStructure: {},
    });
    expect(result.ok).toBe(false);
    if (!result.ok) {
      expect(result.reason).toMatch(/connection not found/i);
    }
  });

  it("returns failed when the structure forbids schema edits", () => {
    const result = resolveStructureCommitContext({
      pending,
      key,
      connections: [pgConnection],
      tableStructure: { [key]: makeStructure({ canAlterSchema: false }) },
    });
    if (result.ok) throw new Error("expected error");
    expect(result.reason).toMatch(/schema edits/i);
  });

  it("returns ok when ddlStructure is absent (backend is the source of truth)", () => {
    const result = resolveStructureCommitContext({
      pending,
      key,
      connections: [pgConnection],
      tableStructure: {},
    });
    if (!result.ok) {
      throw new Error(`expected ok, got: ${result.reason}`);
    }
    expect(result.ddlStructure).toBeUndefined();
    expect(result.schema).toBe("public");
    expect(result.table).toBe("users");
    expect(result.connectionId).toBe("conn-1");
  });

  it("returns ok with the resolved structure on the happy path", () => {
    const structure = makeStructure();
    const result = resolveStructureCommitContext({
      pending,
      key,
      connections: [pgConnection],
      tableStructure: { [key]: structure },
    });
    if (!result.ok) {
      throw new Error(`expected ok, got: ${result.reason}`);
    }
    expect(result.ddlStructure).toBe(structure);
    expect(result.connection).toBe(pgConnection);
  });

  it("treats a key with no '::' delimiter as an empty connectionId (preserves prior behaviour)", () => {
    const result = resolveStructureCommitContext({
      pending,
      key: "bogus-key",
      connections: [pgConnection],
      tableStructure: {},
    });
    // No connection matches an empty connectionId, so we fall through to
    // the connection-not-found branch.
    expect(result.ok).toBe(false);
  });
});
