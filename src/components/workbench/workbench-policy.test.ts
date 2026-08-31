import { describe, expect, it } from "vitest";

import { relationalRailForTab } from "@/components/workbench/workbench-policy";

describe("relationalRailForTab", () => {
  it("routes object and table tabs to Tables and query tabs to Queries", () => {
    expect(relationalRailForTab("object")).toBe("tables");
    expect(relationalRailForTab("table")).toBe("tables");
    expect(relationalRailForTab("query")).toBe("queries");
  });
});
