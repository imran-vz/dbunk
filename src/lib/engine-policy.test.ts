import { describe, expect, it } from "vitest";

import type { DatabaseEngine } from "@/lib/store";

import {
  enginePolicy,
  keyvaluePolicy,
  relationalPolicy,
  storageClassFor,
} from "./engine-policy";

/**
 * Every test here is a small assertion against the static policy
 * table — there's no logic to exercise beyond "the right cell has
 * the right value." We test the shape of the contract:
 *
 *   - exhaustiveness over `DatabaseEngine`
 *   - the per-engine knobs that real components read
 *   - storage-class fork (ADR-0008): Redis carries keyvalue-shaped
 *     fields, relational engines carry relational-shaped fields
 *   - narrowing helpers reject the wrong class
 *
 * The Rust mirror (`src-tauri/src/dispatch.rs::storage_class_is_stable_per_engine`)
 * asserts the same `engine → storage class` classification; drift
 * between the two breaks CI.
 */

const ALL_ENGINES: DatabaseEngine[] = [
  "PostgreSQL",
  "MySQL",
  "SQLite",
  "ClickHouse",
  "Redis",
];

const RELATIONAL_ENGINES = [
  "PostgreSQL",
  "MySQL",
  "SQLite",
  "ClickHouse",
] as const;

describe("enginePolicy", () => {
  it("returns a policy for every engine variant", () => {
    for (const engine of ALL_ENGINES) {
      const policy = enginePolicy(engine);
      expect(policy.engine).toBe(engine);
    }
  });

  it("classifies relational vs keyvalue engines correctly", () => {
    for (const engine of RELATIONAL_ENGINES) {
      expect(storageClassFor(engine)).toBe("relational");
    }
    expect(storageClassFor("Redis")).toBe("keyvalue");
  });

  it("identifies ClickHouse as the only relational engine without foreign keys", () => {
    expect(relationalPolicy("PostgreSQL").hasForeignKeys).toBe(true);
    expect(relationalPolicy("MySQL").hasForeignKeys).toBe(true);
    expect(relationalPolicy("SQLite").hasForeignKeys).toBe(true);
    expect(relationalPolicy("ClickHouse").hasForeignKeys).toBe(false);
  });

  it("renders ClickHouse FK-unsupported copy as an engine fact, not a coverage gap", () => {
    expect(relationalPolicy("ClickHouse").foreignKeysUnsupportedCopy).toBe(
      "ClickHouse does not support foreign keys.",
    );
    expect(relationalPolicy("PostgreSQL").foreignKeysUnsupportedCopy).toContain(
      "this engine",
    );
  });

  it("uses 'Sorting key' / 'ORDER BY' for ClickHouse, 'Primary key' / 'PK' for other relational engines", () => {
    expect(relationalPolicy("ClickHouse").labels.primaryKey).toBe(
      "Sorting key",
    );
    expect(relationalPolicy("ClickHouse").labels.primaryKeyBadge).toBe(
      "ORDER BY",
    );
    for (const engine of ["PostgreSQL", "MySQL", "SQLite"] as const) {
      expect(relationalPolicy(engine).labels.primaryKey).toBe("Primary key");
      expect(relationalPolicy(engine).labels.primaryKeyBadge).toBe("PK");
    }
  });

  it("uses 'Skip indices' for ClickHouse, 'Indexes' elsewhere", () => {
    expect(relationalPolicy("ClickHouse").labels.indexes).toBe("Skip indices");
    expect(relationalPolicy("PostgreSQL").labels.indexes).toBe("Indexes");
  });

  it("reports ClickHouse row counts as exact, others as estimates", () => {
    expect(relationalPolicy("ClickHouse").rowCountKind).toBe("exact");
    expect(relationalPolicy("PostgreSQL").rowCountKind).toBe("estimate");
    expect(relationalPolicy("MySQL").rowCountKind).toBe("estimate");
    expect(relationalPolicy("SQLite").rowCountKind).toBe("estimate");
  });

  it("exposes a non-null schema-map FK banner only for ClickHouse", () => {
    expect(relationalPolicy("ClickHouse").schemaMapNoForeignKeysCopy).toMatch(
      /ClickHouse/,
    );
    expect(
      relationalPolicy("PostgreSQL").schemaMapNoForeignKeysCopy,
    ).toBeNull();
    expect(relationalPolicy("MySQL").schemaMapNoForeignKeysCopy).toBeNull();
    expect(relationalPolicy("SQLite").schemaMapNoForeignKeysCopy).toBeNull();
  });

  it("disables host/auth on SQLite and surfaces engine-specific connection-form toggles", () => {
    expect(enginePolicy("SQLite").connectionForm.requiresHostAndAuth).toBe(
      false,
    );
    expect(enginePolicy("ClickHouse").connectionForm.showClickHouseHttp).toBe(
      true,
    );
    expect(enginePolicy("Redis").connectionForm.showRedisTls).toBe(true);
    expect(enginePolicy("Redis").connectionForm.showRedisDbNumber).toBe(true);
    expect(enginePolicy("PostgreSQL").connectionForm.showClickHouseHttp).toBe(
      false,
    );
    expect(enginePolicy("PostgreSQL").connectionForm.showRedisTls).toBe(false);
    expect(enginePolicy("MySQL").connectionForm.showClickHouseHttp).toBe(false);
    expect(enginePolicy("MySQL").connectionForm.showRedisTls).toBe(false);
  });

  it("knows the canonical default port per engine", () => {
    expect(enginePolicy("PostgreSQL").connectionForm.defaultPort).toBe(5432);
    expect(enginePolicy("MySQL").connectionForm.defaultPort).toBe(3306);
    expect(enginePolicy("ClickHouse").connectionForm.defaultPort).toBe(8123);
    expect(enginePolicy("Redis").connectionForm.defaultPort).toBe(6379);
  });

  it("returns Redis-shaped keyvalue policy with destructive-command lists", () => {
    const redis = keyvaluePolicy("Redis");
    expect(redis.storageClass).toBe("keyvalue");
    expect(redis.defaultDbNumber).toBe(0);
    expect(redis.maxDbNumber).toBe(15);
    expect(redis.defaultSeparator).toBe(":");
    expect(redis.pubSubSupported).toBe(true);
    expect(redis.transactionsSupported).toBe(true);
    expect(redis.destructiveCommandsHard).toContain("FLUSHDB");
    expect(redis.destructiveCommandsHard).toContain("CONFIG SET");
    expect(redis.destructiveCommandsSoft).toContain("KEYS");
  });

  it("relationalPolicy throws when called with a keyvalue engine", () => {
    expect(() => relationalPolicy("Redis")).toThrow(/keyvalue/);
  });

  it("keyvaluePolicy throws when called with a relational engine", () => {
    expect(() => keyvaluePolicy("PostgreSQL")).toThrow(/relational/);
  });
});
