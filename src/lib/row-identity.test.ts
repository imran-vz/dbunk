import { describe, expect, it } from "vitest";

import { pickRowIdentity } from "@/lib/row-identity";
import type { ColumnInfo, IndexInfo, TableStructure } from "@/lib/store";

const buildColumn = (
  overrides: Partial<ColumnInfo> & { name: string },
): ColumnInfo => ({
  name: overrides.name,
  dataType: overrides.dataType ?? "text",
  nullable: overrides.nullable ?? false,
  defaultValue: overrides.defaultValue ?? null,
  isPrimaryKey: overrides.isPrimaryKey ?? false,
  ordinalPosition: overrides.ordinalPosition ?? 1,
});

const buildIndex = (
  overrides: Partial<IndexInfo> & { name: string },
): IndexInfo => ({
  name: overrides.name,
  columns: overrides.columns ?? [],
  isUnique: overrides.isUnique ?? false,
  isPrimary: overrides.isPrimary ?? false,
  method: overrides.method ?? null,
});

const buildStructure = (
  overrides: Partial<TableStructure> = {},
): TableStructure => ({
  columns: overrides.columns ?? [],
  primaryKey: overrides.primaryKey ?? null,
  foreignKeys: overrides.foreignKeys ?? [],
  indexes: overrides.indexes ?? [],
  constraints: overrides.constraints ?? [],
  capabilities: overrides.capabilities ?? {
    columns: true,
    primaryKey: true,
    foreignKeys: true,
    indexes: true,
    constraints: true,
    canInsertRows: true,
    canUpdateRows: true,
    canDeleteRows: true,
    canAlterSchema: true,
    updateSemantics: "synchronous",
    uniquenessGuarantee: "exact",
  },
});

describe("pickRowIdentity", () => {
  it("returns null when no structure is provided", () => {
    expect(pickRowIdentity(undefined)).toBeNull();
  });

  it("prefers the primary key when present", () => {
    const structure = buildStructure({
      columns: [
        buildColumn({ name: "id", isPrimaryKey: true, ordinalPosition: 1 }),
        buildColumn({ name: "email", nullable: false, ordinalPosition: 2 }),
      ],
      primaryKey: ["id"],
      indexes: [
        buildIndex({
          name: "users_email_key",
          columns: ["email"],
          isUnique: true,
          isPrimary: false,
        }),
      ],
    });

    expect(pickRowIdentity(structure)).toEqual({ columns: ["id"] });
  });

  it("supports composite primary keys", () => {
    const structure = buildStructure({
      columns: [
        buildColumn({
          name: "tenant_id",
          isPrimaryKey: true,
          ordinalPosition: 1,
        }),
        buildColumn({
          name: "user_id",
          isPrimaryKey: true,
          ordinalPosition: 2,
        }),
      ],
      primaryKey: ["tenant_id", "user_id"],
    });

    expect(pickRowIdentity(structure)).toEqual({
      columns: ["tenant_id", "user_id"],
    });
  });

  it("falls back to a unique non-null index when no primary key", () => {
    const structure = buildStructure({
      columns: [
        buildColumn({ name: "id", nullable: true, ordinalPosition: 1 }),
        buildColumn({ name: "email", nullable: false, ordinalPosition: 2 }),
      ],
      primaryKey: null,
      indexes: [
        buildIndex({
          name: "users_email_key",
          columns: ["email"],
          isUnique: true,
          isPrimary: false,
        }),
      ],
    });

    expect(pickRowIdentity(structure)).toEqual({ columns: ["email"] });
  });

  it("ignores unique indexes that include a nullable column", () => {
    const structure = buildStructure({
      columns: [
        buildColumn({ name: "email", nullable: true, ordinalPosition: 1 }),
      ],
      primaryKey: null,
      indexes: [
        buildIndex({
          name: "users_email_key",
          columns: ["email"],
          isUnique: true,
          isPrimary: false,
        }),
      ],
    });

    expect(pickRowIdentity(structure)).toBeNull();
  });

  it("ignores non-unique indexes", () => {
    const structure = buildStructure({
      columns: [
        buildColumn({ name: "email", nullable: false, ordinalPosition: 1 }),
      ],
      primaryKey: null,
      indexes: [
        buildIndex({
          name: "users_email_idx",
          columns: ["email"],
          isUnique: false,
          isPrimary: false,
        }),
      ],
    });

    expect(pickRowIdentity(structure)).toBeNull();
  });

  it("ignores primary indexes when primaryKey field is missing (uses fallback only on non-primary unique)", () => {
    const structure = buildStructure({
      columns: [
        buildColumn({ name: "id", nullable: false, ordinalPosition: 1 }),
      ],
      primaryKey: null,
      indexes: [
        buildIndex({
          name: "users_pkey",
          columns: ["id"],
          isUnique: true,
          isPrimary: true,
        }),
      ],
    });

    expect(pickRowIdentity(structure)).toBeNull();
  });

  it("returns null when there is no PK and no eligible unique index", () => {
    const structure = buildStructure({
      columns: [
        buildColumn({ name: "name", nullable: false, ordinalPosition: 1 }),
      ],
      primaryKey: null,
      indexes: [],
    });

    expect(pickRowIdentity(structure)).toBeNull();
  });

  it("picks the unique index with the fewest columns when multiple qualify", () => {
    const structure = buildStructure({
      columns: [
        buildColumn({ name: "a", nullable: false, ordinalPosition: 1 }),
        buildColumn({ name: "b", nullable: false, ordinalPosition: 2 }),
        buildColumn({ name: "c", nullable: false, ordinalPosition: 3 }),
      ],
      primaryKey: null,
      indexes: [
        buildIndex({
          name: "ab_unique",
          columns: ["a", "b"],
          isUnique: true,
          isPrimary: false,
        }),
        buildIndex({
          name: "c_unique",
          columns: ["c"],
          isUnique: true,
          isPrimary: false,
        }),
      ],
    });

    expect(pickRowIdentity(structure)).toEqual({ columns: ["c"] });
  });

  it("ignores unique indexes that reference unknown columns", () => {
    const structure = buildStructure({
      columns: [
        buildColumn({ name: "email", nullable: false, ordinalPosition: 1 }),
      ],
      primaryKey: null,
      indexes: [
        buildIndex({
          name: "ghost_unique",
          columns: ["ghost"],
          isUnique: true,
          isPrimary: false,
        }),
      ],
    });

    expect(pickRowIdentity(structure)).toBeNull();
  });
});
