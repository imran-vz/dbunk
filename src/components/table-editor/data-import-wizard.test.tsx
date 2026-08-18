/* oxlint-disable anti-slop/require-safety-comment-for-type-assertion -- Test fixtures use controlled mocks and assertions to exercise otherwise inaccessible boundaries. */
import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

import { DataImportWizard } from "@/components/table-editor/data-import-wizard";
import type { ColumnInfo } from "@/lib/store";

const column = (name: string, ordinalPosition: number): ColumnInfo => ({
  name,
  dataType: "text",
  nullable: true,
  defaultValue: null,
  isPrimaryKey: false,
  ordinalPosition,
});

describe("DataImportWizard", () => {
  it("loads a CSV file, maps matching columns, and submits rows", async () => {
    const onImportRows = vi.fn().mockResolvedValue(undefined);
    render(
      <DataImportWizard
        columns={[column("id", 1), column("name", 2)]}
        engine="PostgreSQL"
        isWriting={false}
        onClose={vi.fn()}
        onImportRows={onImportRows}
      />,
    );

    fireEvent.change(screen.getByLabelText("Import file"), {
      target: {
        files: [
          {
            name: "users.csv",
            text: async () => "id,name\n1,\\N",
          } as File,
        ],
      },
    });

    await screen.findByText(/1 rows ready/);
    fireEvent.click(screen.getByRole("button", { name: /import rows/i }));

    await waitFor(() =>
      expect(onImportRows).toHaveBeenCalledWith({
        columns: ["id", "name"],
        rows: [["1", null]],
        useCopy: false,
      }),
    );
  });
});
