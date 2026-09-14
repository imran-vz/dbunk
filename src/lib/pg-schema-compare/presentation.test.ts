import { describe, expect, it } from "vitest";

import { decodeSchemaCompareFailure } from "./failure";
import {
  formatExcludedCount,
  formatFieldPath,
  formatPagePosition,
  formatSchemaCompareFailure,
  formatSchemaCompareJobFailure,
} from "./presentation";

describe("schema comparison presentation", () => {
  it("keeps constraint-owned index identities and key positions visible", () => {
    expect(
      formatFieldPath({
        kind: "indexKey",
        name: "orders_pkey",
        owner: "orders_pkey",
        position: 0,
        field: "sortOptions",
      }),
    ).toBe(
      "index key / orders_pkey (constraint orders_pkey) #1 / sort options",
    );
    expect(
      formatFieldPath({
        kind: "index",
        name: "orders_created_idx",
        owner: null,
        field: "unique",
      }),
    ).toBe("index / orders_created_idx / unique");
    expect(formatFieldPath({ kind: "table", field: "persistence" })).toBe(
      "table / persistence",
    );
  });

  it("never renders an incomplete category count as an exact total or zero", () => {
    expect(formatExcludedCount(0, false)).toBe("Not compared");
    expect(formatExcludedCount(4, false)).toBe("At least 4");
    expect(formatExcludedCount(0, true)).toBe("0");
  });

  it("labels explicit server page positions", () => {
    expect(formatPagePosition(100, 37, 137)).toBe("101–137 of 137");
    expect(formatPagePosition(0, 0, 0)).toBe("0 of 0");
  });

  it("maps native failures to actionable text and keeps observation failures separate", () => {
    expect(
      formatSchemaCompareFailure({
        kind: "unsupportedVersion",
        side: "target",
        version: "17.11",
      }),
    ).toBe(
      "Target is running PostgreSQL 17.11. Schema comparison supports PostgreSQL 16 endpoints only.",
    );
    expect(formatSchemaCompareFailure({ kind: "unavailable" })).not.toMatch(
      /expired\./,
    );
    expect(decodeSchemaCompareFailure(new Error("zod"))).toEqual({
      kind: "invalidResponse",
    });
    expect(decodeSchemaCompareFailure("lost")).toEqual({ kind: "transport" });
    expect(decodeSchemaCompareFailure({ kind: "captureChanged" })).toEqual({
      kind: "captureChanged",
    });
  });

  it("describes a failed job's unavailable endpoints without claiming expiry, and says once that no result exists", () => {
    const unavailable = formatSchemaCompareJobFailure({ kind: "unavailable" });
    expect(unavailable).toMatch(/schema may not exist/);
    expect(unavailable).toMatch(/privileges/);
    expect(unavailable).not.toMatch(/expired|dismissed/);
    expect(unavailable.match(/result was produced\./g)).toHaveLength(1);
    const limit = formatSchemaCompareJobFailure({
      kind: "limitExceeded",
      limit: "fieldBytes",
    });
    expect(limit.match(/result was produced\./g)).toHaveLength(1);
    expect(formatSchemaCompareJobFailure({ kind: "busy" })).toMatch(
      /No result was produced\.$/,
    );
  });
});
