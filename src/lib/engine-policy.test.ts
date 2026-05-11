import { describe, expect, it } from "vitest";

import type { DatabaseEngine } from "@/lib/store";

import { enginePolicy } from "./engine-policy";

/**
 * Every test here is a small assertion against the static policy
 * table — there's no logic to exercise beyond "the right cell has
 * the right value." We test the shape of the contract:
 *
 *   - exhaustiveness over `DatabaseEngine`
 *   - the per-engine knobs that real components read
 *   - that ClickHouse's distinctive policies (sorting key, exact
 *     count, no FKs) are correctly set
 */

const ALL_ENGINES: DatabaseEngine[] = [
  "PostgreSQL",
  "MySQL",
  "SQLite",
  "ClickHouse",
];

describe("enginePolicy", () => {
  it("returns a policy for every engine variant", () => {
    for (const engine of ALL_ENGINES) {
      const policy = enginePolicy(engine);
      expect(policy.engine).toBe(engine);
    }
  });

  it("identifies ClickHouse as the only engine without foreign keys", () => {
    expect(enginePolicy("PostgreSQL").hasForeignKeys).toBe(true);
    expect(enginePolicy("MySQL").hasForeignKeys).toBe(true);
    expect(enginePolicy("SQLite").hasForeignKeys).toBe(true);
    expect(enginePolicy("ClickHouse").hasForeignKeys).toBe(false);
  });

  it("renders ClickHouse FK-unsupported copy as an engine fact, not a coverage gap", () => {
    expect(enginePolicy("ClickHouse").foreignKeysUnsupportedCopy).toBe(
      "ClickHouse does not support foreign keys.",
    );
    expect(enginePolicy("PostgreSQL").foreignKeysUnsupportedCopy).toContain(
      "this engine",
    );
  });

  it("uses 'Sorting key' / 'ORDER BY' for ClickHouse, 'Primary key' / 'PK' for others", () => {
    expect(enginePolicy("ClickHouse").labels.primaryKey).toBe("Sorting key");
    expect(enginePolicy("ClickHouse").labels.primaryKeyBadge).toBe("ORDER BY");
    for (const engine of ["PostgreSQL", "MySQL", "SQLite"] as const) {
      expect(enginePolicy(engine).labels.primaryKey).toBe("Primary key");
      expect(enginePolicy(engine).labels.primaryKeyBadge).toBe("PK");
    }
  });

  it("uses 'Skip indices' for ClickHouse, 'Indexes' elsewhere", () => {
    expect(enginePolicy("ClickHouse").labels.indexes).toBe("Skip indices");
    expect(enginePolicy("PostgreSQL").labels.indexes).toBe("Indexes");
  });

  it("reports ClickHouse row counts as exact, others as estimates", () => {
    expect(enginePolicy("ClickHouse").rowCountKind).toBe("exact");
    expect(enginePolicy("PostgreSQL").rowCountKind).toBe("estimate");
    expect(enginePolicy("MySQL").rowCountKind).toBe("estimate");
    expect(enginePolicy("SQLite").rowCountKind).toBe("estimate");
  });

  it("exposes a non-null schema-map FK banner only for ClickHouse", () => {
    expect(enginePolicy("ClickHouse").schemaMapNoForeignKeysCopy).toMatch(
      /ClickHouse/,
    );
    expect(enginePolicy("PostgreSQL").schemaMapNoForeignKeysCopy).toBeNull();
    expect(enginePolicy("MySQL").schemaMapNoForeignKeysCopy).toBeNull();
    expect(enginePolicy("SQLite").schemaMapNoForeignKeysCopy).toBeNull();
  });

  it("disables host/auth on SQLite and surfaces CH HTTP fields only on ClickHouse", () => {
    expect(enginePolicy("SQLite").connectionForm.requiresHostAndAuth).toBe(
      false,
    );
    expect(enginePolicy("ClickHouse").connectionForm.showClickHouseHttp).toBe(
      true,
    );
    expect(enginePolicy("PostgreSQL").connectionForm.showClickHouseHttp).toBe(
      false,
    );
    expect(enginePolicy("MySQL").connectionForm.showClickHouseHttp).toBe(false);
  });

  it("knows the canonical default port per engine", () => {
    expect(enginePolicy("PostgreSQL").connectionForm.defaultPort).toBe(5432);
    expect(enginePolicy("MySQL").connectionForm.defaultPort).toBe(3306);
    expect(enginePolicy("ClickHouse").connectionForm.defaultPort).toBe(8123);
  });
});
