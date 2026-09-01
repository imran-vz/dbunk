/* oxlint-disable anti-slop/no-chained-type-assertions anti-slop/require-safety-comment-for-type-assertion -- Test fixtures use controlled mocks and assertions to exercise otherwise inaccessible boundaries. */
import { describe, expect, it } from "vitest";

import type { TableStructure } from "@/lib/store";

import { snapshotFromStructure } from "./use-structure";

const baseStructure: TableStructure = {
  columns: [
    {
      name: "id",
      dataType: "int",
      nullable: false,
      defaultValue: null,
      isPrimaryKey: true,
      ordinalPosition: 1,
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

describe("snapshotFromStructure", () => {
  it("defaults every field when structure is undefined", () => {
    const snap = snapshotFromStructure(undefined);
    expect(snap.columns).toEqual([]);
    expect(snap.primaryKey).toBeNull();
    expect(snap.foreignKeys).toEqual([]);
    expect(snap.indexes).toEqual([]);
    expect(snap.constraints).toEqual([]);
    expect(snap.tableEngine).toBeUndefined();
    expect(snap.partitionBy).toBeNull();
    expect(snap.sampleBy).toBeNull();
    expect(snap.capabilities.canAlterSchema).toBe(false);
  });

  it("forwards every populated field from the structure", () => {
    const structure: TableStructure = {
      ...baseStructure,
      tableEngine: "MergeTree",
      partitionBy: "toYYYYMM(ts)",
      sampleBy: "id",
    };
    const snap = snapshotFromStructure(structure);
    expect(snap.columns).toBe(structure.columns);
    expect(snap.primaryKey).toEqual(["id"]);
    expect(snap.foreignKeys).toBe(structure.foreignKeys);
    expect(snap.indexes).toBe(structure.indexes);
    expect(snap.constraints).toBe(structure.constraints);
    expect(snap.capabilities).toBe(structure.capabilities);
    expect(snap.tableEngine).toBe("MergeTree");
    expect(snap.partitionBy).toBe("toYYYYMM(ts)");
    expect(snap.sampleBy).toBe("id");
  });

  it("handles a structure missing the optional CH fields", () => {
    const snap = snapshotFromStructure(baseStructure);
    expect(snap.tableEngine).toBeUndefined();
    expect(snap.partitionBy).toBeNull();
    expect(snap.sampleBy).toBeNull();
  });

  it("falls back when capabilities is undefined-shaped at runtime", () => {
    const malformed = {
      ...baseStructure,
      capabilities: undefined,
    } as unknown as TableStructure | undefined;
    const snap = snapshotFromStructure(malformed);
    expect(snap.capabilities.canAlterSchema).toBe(false);
  });
});
