import { describe, expect, it } from "vitest";

import type { PgObjectDescription, TableStructure } from "@/lib/store/types";
import {
  normalizePgObjectDescription,
  normalizeTableStructure,
} from "@/lib/table-structure-contract";

const legacyStructure = {
  columns: [],
  primaryKey: null,
  foreignKeys: [],
  indexes: [],
  constraints: [],
  capabilities: {
    columns: true,
    primaryKey: true,
    foreignKeys: true,
    indexes: true,
    constraints: true,
    canInsertRows: false,
    canUpdateRows: false,
    canDeleteRows: false,
    canAlterSchema: false,
    uniquenessGuarantee: "best-effort" as const,
  },
};

describe("normalizeTableStructure", () => {
  it("fills the table-security fields a legacy payload lacks", () => {
    const structure = normalizeTableStructure(legacyStructure);
    expect(structure.triggers).toEqual([]);
    expect(structure.policies).toEqual([]);
    expect(structure.privileges).toEqual([]);
    expect(structure.rowSecurity).toBeNull();
    expect(structure.capabilities.triggers).toBe(false);
    expect(structure.capabilities.policies).toBe(false);
    expect(structure.capabilities.privileges).toBe(false);
  });

  it("keeps backend-provided values untouched", () => {
    const full: TableStructure = {
      ...legacyStructure,
      triggers: [
        {
          name: "orders_touch",
          timing: "BEFORE",
          events: ["UPDATE"],
          updateColumns: [],
          level: "ROW",
          enabled: "disabled",
          functionSchema: "lifecycle",
          functionName: "touch_orders",
          definition: "CREATE TRIGGER …",
        },
      ],
      policies: [],
      privileges: [{ grantee: "PUBLIC", privilege: "SELECT", grantable: false }],
      rowSecurity: { enabled: true, forced: false },
      capabilities: {
        ...legacyStructure.capabilities,
        triggers: true,
        policies: true,
        privileges: true,
      },
    };
    expect(normalizeTableStructure(full)).toEqual(full);
  });
});

describe("normalizePgObjectDescription", () => {
  const reference = {
    kind: "function" as const,
    schema: "lifecycle",
    name: "order_total",
    identityArgs: "order_id integer",
  };

  it("fills routine source facts a legacy payload lacks", () => {
    const description = normalizePgObjectDescription({
      reference,
      owner: null,
      comment: null,
      definitionSql: null,
      facts: {
        kind: "routine",
        language: "plpgsql",
        returns: "numeric",
        volatility: "stable",
        arguments: "order_id integer",
      },
    });
    expect(description.facts).toEqual({
      kind: "routine",
      language: "plpgsql",
      returns: "numeric",
      volatility: "stable",
      arguments: "order_id integer",
      body: null,
      strict: false,
      securityDefiner: false,
      parallel: null,
    });
  });

  it("passes non-routine facts through unchanged", () => {
    const description: PgObjectDescription = {
      reference: { ...reference, kind: "schema", schema: null, identityArgs: null },
      owner: "dbunk",
      comment: null,
      definitionSql: null,
      facts: { kind: "schema" },
    };
    expect(normalizePgObjectDescription(description)).toBe(description);
  });
});
