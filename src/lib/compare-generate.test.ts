import { describe, expect, it } from "vitest";
import {
  compareData,
  compareSchemas,
  generateMockRows,
  migrationSqlForDiff,
} from "@/lib/compare-generate";

describe("compare and generate", () => {
  it("detects schema differences and migration SQL", () => {
    const diffs = compareSchemas(
      [
        {
          schema: "public",
          name: "users",
          columns: [{ name: "id", dataType: "integer", nullable: false }],
        },
      ],
      [
        {
          schema: "public",
          name: "users",
          columns: [{ name: "id", dataType: "bigint", nullable: false }],
        },
        { schema: "public", name: "orders", columns: [] },
      ],
    );
    expect(diffs.map((diff) => diff.kind)).toEqual([
      "changed-column",
      "extra-table",
    ]);
    expect(migrationSqlForDiff(diffs[0])).toBe(
      'ALTER TABLE "users" ALTER COLUMN "id" TYPE integer;',
    );
  });

  it("compares table data by key", () => {
    expect(
      compareData({
        keys: ["id"],
        source: [
          { id: 1, name: "Ada" },
          { id: 2, name: "Grace" },
        ],
        target: [
          { id: 1, name: "Ada" },
          { id: 2, name: "Grace Hopper" },
          { id: 3, name: "Katherine" },
        ],
      }),
    ).toMatchObject({
      missing: [],
      extra: [{ id: 3, name: "Katherine" }],
      changed: [{ key: "2" }],
    });
  });

  it("generates deterministic mock data from column types", () => {
    expect(
      generateMockRows(
        [
          { name: "id", dataType: "integer", nullable: false },
          { name: "active", dataType: "boolean", nullable: false },
          { name: "name", dataType: "text", nullable: false },
        ],
        2,
      ),
    ).toEqual([
      { id: 1, active: true, name: "name_1" },
      { id: 2, active: false, name: "name_2" },
    ]);
  });
});
