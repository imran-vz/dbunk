import { describe, expect, it, vi } from "vitest";

import { buildQueryStatusItems } from "@/components/query-editor/status-items";
import { buildStatusItems } from "@/components/table-editor/status-items";

// SAFETY: the builder reads only the fields provided by this fixture.
const pagination = {
  page: 1,
  totalPages: 1,
  totalRows: 3,
  countApproximate: false,
  runtimeMs: 5,
  countLabel: "3 rows",
} as never;

describe("pending status badge (§3.1)", () => {
  it("table items surface the staged count as a clickable warning badge", () => {
    const onOpenReview = vi.fn();
    const items = buildStatusItems({
      errorMessage: null,
      isLoading: false,
      rowCount: 3,
      pagination,
      activeConnection: undefined,
      stagedChangeCount: 4,
      onOpenReview,
    });
    const badge = items.find((item) => item.id === "pending");
    expect(badge?.value).toBe("4 staged");
    expect(badge?.tone).toBe("warning");
    badge?.onClick?.();
    expect(onOpenReview).toHaveBeenCalledTimes(1);
  });

  it("omits the badge when nothing is staged", () => {
    const items = buildStatusItems({
      errorMessage: null,
      isLoading: false,
      rowCount: 3,
      pagination,
      activeConnection: undefined,
      stagedChangeCount: 0,
    });
    expect(items.some((item) => item.id === "pending")).toBe(false);
  });

  it("query items carry the same badge", () => {
    const items = buildQueryStatusItems({
      tabLabel: "q.sql",
      cursor: { lineNumber: 1, column: 1 },
      errorMessage: null,
      activeConnection: undefined,
      stagedChangeCount: 1,
    });
    expect(items.find((item) => item.id === "pending")?.value).toBe("1 staged");
  });
});
