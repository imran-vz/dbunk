/* oxlint-disable anti-slop/require-safety-comment-for-type-assertion -- Test fixtures use controlled mocks and assertions to exercise otherwise inaccessible boundaries. */
import { render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

import { SeedTableForm } from "@/components/table-editor/seed-table-form";
import type { ColumnInfo } from "@/lib/store";

const columns: ColumnInfo[] = [
  {
    name: "email",
    dataType: "text",
    nullable: false,
    defaultValue: null,
    isPrimaryKey: false,
    ordinalPosition: 1,
  },
];

describe("SeedTableForm progress", () => {
  it("renders completed rows and percentage while a seed is running", () => {
    render(
      <SeedTableForm
        columns={columns}
        isSeeding
        progress={{ rowsCompleted: 250, totalRows: 1_000 }}
        onSubmit={vi.fn()}
        onClose={vi.fn()}
      />,
    );

    const progress = screen.getByRole("progressbar", {
      name: "Table seeding progress",
    });
    expect(progress.getAttribute("aria-valuenow")).toBe("250");
    expect(progress.getAttribute("aria-valuemax")).toBe("1000");
    expect(screen.getByText("250 / 1,000")).not.toBeNull();
    expect(
      (
        screen.getByRole("button", {
          name: "Seeding 25%",
        }) as HTMLButtonElement
      ).disabled,
    ).toBe(true);
  });
});
