/* oxlint-disable anti-slop/no-module-mocking anti-slop/require-safety-comment-for-type-assertion -- Test fixtures use controlled mocks and assertions to exercise otherwise inaccessible boundaries. */
// @vitest-environment jsdom
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("@/lib/download", () => ({
  downloadBlob: vi.fn(),
}));

import { DataGrid } from "@/components/data-grid";
import { downloadBlob } from "@/lib/download";

const mockedDownloadBlob = vi.mocked(downloadBlob);

/** Extract text content from the Blob passed to downloadBlob. */
async function blobText(blob: Blob): Promise<string> {
  // jsdom's Blob may not support .text() or .arrayBuffer(), but
  // FileReader-style access works via the underlying buffer.
  return new Promise((resolve, reject) => {
    const reader = new FileReader();
    reader.onload = () => resolve(reader.result as string);
    reader.onerror = () => reject(reader.error);
    reader.readAsText(blob);
  });
}

const sampleColumns = ["id", "name"];
const sampleRows = [
  ["1", "Ada"],
  ["2", "Grace"],
  ["3", "Edsger"],
];

const openMoreMenu = () => {
  fireEvent.click(screen.getByRole("button", { name: /export/i }));
};

const findMenuItem = (label: RegExp): HTMLElement => {
  const items = screen.getAllByRole("menuitem");
  const match = items.find((item) => label.test(item.textContent ?? ""));
  if (!match) {
    throw new Error(`menu item not found: ${label}`);
  }
  return match;
};

beforeEach(() => {
  mockedDownloadBlob.mockClear();
});

afterEach(() => {
  cleanup();
});

describe("DataGrid export menu", () => {
  it("disables Export selected items when no rows are selected", () => {
    render(
      <DataGrid
        data={sampleRows}
        columns={sampleColumns}
        exportFilenameBase="conn-public-users-2026-05-09"
      />,
    );
    openMoreMenu();

    const exportSelectedCsv = findMenuItem(/Export selected to \.csv/i);
    const exportSelectedJson = findMenuItem(/Export selected to \.json/i);
    expect(exportSelectedCsv.getAttribute("data-disabled")).not.toBeNull();
    expect(exportSelectedJson.getAttribute("data-disabled")).not.toBeNull();
  });

  it("enables Export selected items once a row is selected", () => {
    render(
      <DataGrid
        data={sampleRows}
        columns={sampleColumns}
        exportFilenameBase="conn-public-users-2026-05-09"
      />,
    );

    // Select the first data row's checkbox. There's one header checkbox plus
    // one per row, so checkboxes[1] is the first row.
    const checkboxes = screen.getAllByRole("checkbox");
    fireEvent.click(checkboxes[1] as HTMLInputElement);

    openMoreMenu();
    const exportSelectedCsv = findMenuItem(/Export selected to \.csv/i);
    expect(exportSelectedCsv.getAttribute("data-disabled")).toBeNull();
  });

  it("downloads a CSV with header and all rows when 'Export all to .csv' is clicked", async () => {
    render(
      <DataGrid
        data={sampleRows}
        columns={sampleColumns}
        exportFilenameBase="conn-public-users-2026-05-09"
      />,
    );
    openMoreMenu();
    fireEvent.click(findMenuItem(/Export all to \.csv/i));

    await vi.waitFor(() => expect(mockedDownloadBlob).toHaveBeenCalledTimes(1));
    const [filename, blob] = mockedDownloadBlob.mock.calls[0] ?? [];
    expect(filename).toBe("conn-public-users-2026-05-09.csv");
    const content = await blobText(blob as Blob);
    expect(content).toBe("id,name\n1,Ada\n2,Grace\n3,Edsger");
  });

  it("downloads JSON with all rows when 'Export all to .json' is clicked", async () => {
    render(
      <DataGrid
        data={sampleRows}
        columns={sampleColumns}
        exportFilenameBase="conn-public-users-2026-05-09"
      />,
    );
    openMoreMenu();
    fireEvent.click(findMenuItem(/Export all to \.json/i));

    await vi.waitFor(() => expect(mockedDownloadBlob).toHaveBeenCalledTimes(1));
    const [filename, blob] = mockedDownloadBlob.mock.calls[0] ?? [];
    expect(filename).toBe("conn-public-users-2026-05-09.json");
    const content = await blobText(blob as Blob);
    expect(content).toContain('"id": "1"');
    expect(content).toContain('"name": "Ada"');
    const parsed = JSON.parse(content);
    expect(parsed).toHaveLength(3);
  });

  it("exports only selected rows when 'Export selected to .csv' is clicked", async () => {
    render(
      <DataGrid
        data={sampleRows}
        columns={sampleColumns}
        exportFilenameBase="conn-public-users-2026-05-09"
      />,
    );

    const checkboxes = screen.getAllByRole("checkbox");
    // Select rows 1 and 3 (data indices 0 and 2).
    fireEvent.click(checkboxes[1] as HTMLInputElement);
    fireEvent.click(checkboxes[3] as HTMLInputElement);

    openMoreMenu();
    fireEvent.click(findMenuItem(/Export selected to \.csv/i));

    await vi.waitFor(() => expect(mockedDownloadBlob).toHaveBeenCalledTimes(1));
    const [filename, blob] = mockedDownloadBlob.mock.calls[0] ?? [];
    expect(filename).toBe("conn-public-users-2026-05-09-selected.csv");
    const content = await blobText(blob as Blob);
    expect(content).toBe("id,name\n1,Ada\n3,Edsger");
  });

  it("exports only selected rows when 'Export selected to .json' is clicked", async () => {
    render(
      <DataGrid
        data={sampleRows}
        columns={sampleColumns}
        exportFilenameBase="conn-public-users-2026-05-09"
      />,
    );

    const checkboxes = screen.getAllByRole("checkbox");
    fireEvent.click(checkboxes[2] as HTMLInputElement);

    openMoreMenu();
    fireEvent.click(findMenuItem(/Export selected to \.json/i));

    await vi.waitFor(() => expect(mockedDownloadBlob).toHaveBeenCalledTimes(1));
    const [filename, blob] = mockedDownloadBlob.mock.calls[0] ?? [];
    expect(filename).toBe("conn-public-users-2026-05-09-selected.json");
    const content = await blobText(blob as Blob);
    const parsed = JSON.parse(content);
    expect(parsed).toEqual([{ id: "2", name: "Grace" }]);
  });
});

describe("DataGrid read-only mode", () => {
  it("does not invoke onEdit when readOnly is true", () => {
    const onEdit = vi.fn();
    render(
      <DataGrid
        data={sampleRows}
        columns={sampleColumns}
        onEdit={onEdit}
        readOnly
      />,
    );

    // Try to click into a cell — read-only cells render as non-editable buttons
    // with `tabIndex=-1` and no click handler that engages the editor.
    const cells = screen.getAllByRole("button");
    const dataCell = cells.find((btn) => btn.textContent === "Ada");
    expect(dataCell).toBeDefined();
    if (dataCell) {
      fireEvent.click(dataCell);
      // No input should appear — the cell remains a plain button.
      expect(dataCell.tagName.toLowerCase()).toBe("button");
    }
    expect(onEdit).not.toHaveBeenCalled();
  });

  it("disables the Save button when isSaving is true", () => {
    render(
      <DataGrid
        data={sampleRows}
        columns={sampleColumns}
        onEdit={vi.fn()}
        hasEdits
        isSaving
        onSave={vi.fn()}
        onDiscard={vi.fn()}
      />,
    );

    const save = screen.getByRole("button", {
      name: /saving|save changes/i,
    }) as HTMLButtonElement;
    expect(save.disabled).toBe(true);
  });

  it("starts lazy analysis from pointer and keyboard edit intent", () => {
    const onEditIntent = vi.fn();
    render(
      <DataGrid
        data={sampleRows}
        columns={sampleColumns}
        readOnly
        onEditIntent={onEditIntent}
      />,
    );

    const cell = screen.getByRole("button", { name: "Ada" });
    fireEvent.click(cell);
    fireEvent.keyDown(cell, { key: "Enter" });

    expect(onEditIntent).toHaveBeenNthCalledWith(1, 0, 1);
    expect(onEditIntent).toHaveBeenNthCalledWith(2, 0, 1);
  });

  it("keeps blocked columns read only and exposes the reason", () => {
    const onEdit = vi.fn();
    render(
      <DataGrid
        data={sampleRows}
        columns={sampleColumns}
        onEdit={onEdit}
        getCellReadOnlyReason={(_rowIndex, columnIndex) =>
          columnIndex === 1 ? "Generated columns are read only." : undefined
        }
      />,
    );

    const blocked = screen.getByRole("button", { name: "Ada" });
    expect(blocked.getAttribute("title")).toBe(
      "Generated columns are read only.",
    );
    fireEvent.click(blocked);
    expect(screen.queryByDisplayValue("Ada")).toBeNull();
    expect(onEdit).not.toHaveBeenCalled();
  });

  it("renders presentation-only staged row states", () => {
    render(
      <DataGrid
        data={sampleRows}
        columns={sampleColumns}
        getRowState={(rowIndex) =>
          (["deleted", "inserted", "excluded"] as const)[rowIndex]
        }
      />,
    );

    expect(
      screen.getByRole("button", { name: "Ada" }).closest("tr")?.className,
    ).toContain("line-through");
    expect(
      screen.getByRole("button", { name: "Grace" }).closest("tr")?.className,
    ).toContain("bg-success/5");
    expect(
      screen.getByRole("button", { name: "Edsger" }).closest("tr")?.className,
    ).toContain("opacity-50");
  });
});

describe("DataGrid cell display", () => {
  it("limits long text previews while editing the full value", () => {
    const longValue =
      "This address is intentionally longer than fifty characters for display testing.";
    const preview = longValue.slice(0, 50);

    render(
      <DataGrid
        data={[["1", longValue]]}
        columns={sampleColumns}
        onEdit={vi.fn()}
      />,
    );

    const cell = screen.getByRole("button", { name: preview });
    expect(cell.textContent).toBe(preview);
    expect(screen.queryByText(longValue)).toBeNull();
    expect(cell.getAttribute("title")).toBe(longValue);

    fireEvent.click(cell);

    const input = screen.getByDisplayValue(longValue) as HTMLInputElement;
    expect(input.value).toBe(longValue);
  });
});

describe("DataGrid server browse mode", () => {
  const browse = {
    typedFilters: [],
    rawFilterText: "",
    filterMode: "typed" as const,
    sort: [],
    pageSize: 100,
    loadStatus: { state: "success" as const },
    error: null,
    inspection: {
      sql: "SELECT 1",
      params: [{ kind: "text" as const, value: "ada" }],
    },
    omittedRows: 1,
    truncatedCells: 2,
    count: { kind: "estimated" as const, value: 50 },
    exactCount: null,
    countStatus: { state: "idle" as const },
    pageInfo: {
      mode: "keyset" as const,
      page: null,
      hasMore: true,
      nextCursor: { values: ["1"] },
    },
    history: [],
    presets: [],
    onApplyTypedFilter: vi.fn(),
    onRemoveTypedFilter: vi.fn(),
    onClearTypedFilters: vi.fn(),
    onRawFilterApply: vi.fn(),
    onFilterModeChange: vi.fn(),
    onSortChange: vi.fn(),
    onPageSizeChange: vi.fn(),
    onHeaderSort: vi.fn(),
    onCountRows: vi.fn(),
    onCancel: vi.fn(),
    onApplyPreset: vi.fn(),
    onSavePreset: vi.fn(),
    onApplyHistory: vi.fn(),
  };

  beforeEach(() => {
    browse.onApplyTypedFilter.mockClear();
    browse.onPageSizeChange.mockClear();
    browse.onHeaderSort.mockClear();
    browse.onCancel.mockClear();
  });

  it("wires page size options and expand controls", () => {
    const onExpandGrid = vi.fn();
    render(
      <DataGrid
        data={sampleRows}
        columns={sampleColumns}
        serverBrowse={browse}
        onExpandGrid={onExpandGrid}
      />,
    );
    fireEvent.click(screen.getByRole("button", { name: "Expand grid" }));
    expect(onExpandGrid).toHaveBeenCalledTimes(1);
    fireEvent.click(screen.getByRole("combobox", { name: "Page size" }));
    expect(screen.getByRole("option", { name: "10 rows" })).toBeTruthy();
    expect(screen.getByRole("option", { name: "250 rows" })).toBeTruthy();
    expect(screen.getByRole("option", { name: "1000 rows" })).toBeTruthy();
  });

  it("shows inspection SQL, parameters, and truncation notice", () => {
    render(
      <DataGrid
        data={sampleRows}
        columns={sampleColumns}
        serverBrowse={browse}
      />,
    );
    fireEvent.click(screen.getByRole("button", { name: "Inspect query" }));
    expect(screen.getByText("SELECT 1")).toBeTruthy();
    expect(screen.getByText(/\$1 ada/)).toBeTruthy();
    expect(screen.getAllByText(/Partial result/)).toHaveLength(2);
  });

  it("shows partial-result status without opening SQL inspection", () => {
    render(
      <DataGrid
        data={sampleRows}
        columns={sampleColumns}
        serverBrowse={browse}
      />,
    );
    expect(screen.getByText(/Partial result: 1 omitted rows/)).toBeTruthy();
    expect(screen.queryByTestId("browse-inspection")).toBeNull();
  });

  it("exposes null operators in the typed filter menu", () => {
    render(
      <DataGrid
        data={sampleRows}
        columns={sampleColumns}
        serverBrowse={browse}
      />,
    );
    fireEvent.click(screen.getByRole("button", { name: "Filter" }));
    fireEvent.click(screen.getByRole("combobox", { name: "Filter operator" }));
    expect(screen.getByRole("option", { name: "is null" })).toBeTruthy();
    expect(screen.getByRole("option", { name: "is not null" })).toBeTruthy();
    expect(screen.getByRole("option", { name: "in list" })).toBeTruthy();
  });

  it("shows typed database errors with position", () => {
    render(
      <DataGrid
        data={sampleRows}
        columns={sampleColumns}
        serverBrowse={{
          ...browse,
          error: {
            kind: "database",
            code: "42601",
            message: 'syntax error at or near "SELEC"',
            severity: "ERROR",
            position: 14,
          },
        }}
      />,
    );
    fireEvent.click(screen.getByRole("button", { name: "Filter" }));
    const alert = screen.getByRole("alert");
    expect(alert.textContent).toMatch(/syntax error at or near "SELEC"/);
    expect(alert.textContent).toMatch(/position 14/);
  });

  it("sorts from column headers in browse mode", () => {
    render(
      <DataGrid
        data={sampleRows}
        columns={sampleColumns}
        serverBrowse={browse}
      />,
    );
    fireEvent.click(screen.getByRole("button", { name: "id" }));
    expect(browse.onHeaderSort).toHaveBeenCalledWith("id", false);
  });

  it("does not make column headers sort buttons without serverBrowse", () => {
    render(<DataGrid data={sampleRows} columns={sampleColumns} />);
    expect(screen.queryByRole("button", { name: "id" })).toBeNull();
  });

  it("exposes Cancel browse while a browse load is in flight", () => {
    render(
      <DataGrid
        data={sampleRows}
        columns={sampleColumns}
        serverBrowse={{ ...browse, loadStatus: { state: "loading" } }}
      />,
    );
    fireEvent.click(screen.getByRole("button", { name: "Cancel browse" }));
    expect(browse.onCancel).toHaveBeenCalledTimes(1);
  });
});
