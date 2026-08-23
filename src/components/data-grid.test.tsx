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

    const rowOf = (name: string) =>
      screen.getByRole("button", { name }).closest('[role="row"]');
    expect(rowOf("Ada")?.getAttribute("data-row-state")).toBe("deleted");
    expect(screen.getByRole("button", { name: "Ada" }).className).toContain(
      "line-through",
    );
    expect(rowOf("Grace")?.getAttribute("data-row-state")).toBe("inserted");
    expect(rowOf("Grace")?.className).toContain("bg-success/10");
    expect(rowOf("Edsger")?.getAttribute("data-row-state")).toBe("excluded");
    expect(rowOf("Edsger")?.className).toContain("opacity-50");
  });
});

describe("DataGrid cell display", () => {
  it("exposes edited and original values for dirty cells", () => {
    render(
      <DataGrid
        data={[["1", "old@example.test"]]}
        columns={sampleColumns}
        edits={{ 0: { 1: "new@example.test" } }}
        onEdit={vi.fn()}
      />,
    );

    const cell = screen.getByRole("button", { name: "new@example.test" });
    expect(cell.getAttribute("title")).toBe("Original: old@example.test");
    expect(cell.className).toContain("bg-warning/10");
    expect(cell.textContent).toBe("new@example.test");
  });

  it("limits long text previews while editing the full value", () => {
    const longValue = `${"x".repeat(120)}END`;
    const preview = longValue.slice(0, 100);

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

    // §5.4: double-click (not single click) opens the editor with the
    // full, untruncated value.
    fireEvent.doubleClick(cell);
    const input = screen.getByDisplayValue(longValue) as HTMLInputElement;
    expect(input.value).toBe(longValue);
  });

  it("renders NULL as a faint italic keyword and marks multi-line values", () => {
    render(
      <DataGrid
        data={[
          ["NULL", "line one\nline two"],
          ["7", "plain"],
        ]}
        columns={sampleColumns}
      />,
    );

    const nullCell = screen.getByText("NULL");
    expect(nullCell.className).toContain("italic");
    expect(nullCell.className).toContain("text-text-disabled");

    // The multi-line value collapses to its first line + ↵ indicator.
    const multiline = screen.getByText("line one");
    expect(multiline.querySelector("svg")).not.toBeNull();
    expect(screen.queryByText(/line two/)).toBeNull();
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

describe("DataGrid virtualization", () => {
  it("renders a bounded subset of a huge result set", () => {
    const bigRows = Array.from({ length: 10_000 }, (_row, index) => [
      String(index + 1),
      `name-${index + 1}`,
    ]);
    render(<DataGrid data={bigRows} columns={sampleColumns} />);

    const renderedRows = document.querySelectorAll(
      '[data-slot="data-grid-scroll"] [role="row"]',
    );
    // Header + visible rows + overscan — never the full 10k.
    expect(renderedRows.length).toBeGreaterThan(10);
    expect(renderedRows.length).toBeLessThan(100);
    // Row heights are fixed so total scroll height covers every row.
    const sizer = document.querySelector(
      '[data-slot="data-grid-scroll"] > div',
    ) as HTMLElement;
    expect(Number.parseInt(sizer.style.height, 10)).toBeGreaterThan(
      10_000 * 22,
    );
  });
});

describe("DataGrid keyboard model", () => {
  const clipboardText = { value: "" };

  beforeEach(() => {
    clipboardText.value = "";
    Object.defineProperty(navigator, "clipboard", {
      configurable: true,
      value: {
        writeText: (text: string) => {
          clipboardText.value = text;
          return Promise.resolve();
        },
      },
    });
  });

  const focusCell = (name: string) => {
    const cell = screen.getByRole("button", { name });
    fireEvent.click(cell);
    return cell;
  };

  const grid = () =>
    document.querySelector('[data-slot="data-grid-scroll"]') as HTMLElement;

  it("moves the focused cell with arrows and edits with Enter", () => {
    const onEdit = vi.fn();
    render(
      <DataGrid data={sampleRows} columns={sampleColumns} onEdit={onEdit} />,
    );

    focusCell("Ada");
    fireEvent.keyDown(grid(), { key: "ArrowDown" });
    const below = document.querySelector('[data-grid-cell="1-1"]');
    expect(below?.getAttribute("tabindex")).toBe("0");

    fireEvent.keyDown(grid(), { key: "Enter" });
    const input = screen.getByDisplayValue("Grace") as HTMLInputElement;
    fireEvent.change(input, { target: { value: "Hopper" } });
    fireEvent.keyDown(input, { key: "Enter" });
    expect(onEdit).toHaveBeenCalledWith(1, 1, "Hopper");
  });

  it("cancels an open editor when the data prop is swapped", () => {
    const onEdit = vi.fn();
    const { rerender } = render(
      <DataGrid data={sampleRows} columns={sampleColumns} onEdit={onEdit} />,
    );

    focusCell("Ada");
    fireEvent.keyDown(grid(), { key: "Enter" });
    const input = screen.getByDisplayValue("Ada") as HTMLInputElement;
    fireEvent.change(input, { target: { value: "Lovelace" } });

    // A server-browse page (or refresh) lands while the editor is open:
    // the same display slot now holds a different row.
    const swappedRows = [
      ["9", "Barbara"],
      ["10", "Katherine"],
      ["11", "Annie"],
    ];
    rerender(
      <DataGrid data={swappedRows} columns={sampleColumns} onEdit={onEdit} />,
    );

    // The editor must be gone; committing against the new row would stage
    // the typed value onto the wrong record.
    expect(screen.queryByDisplayValue("Lovelace")).toBeNull();
    fireEvent.keyDown(grid(), { key: "Enter" });
    const reopened = screen.getByDisplayValue("Barbara") as HTMLInputElement;
    fireEvent.keyDown(reopened, { key: "Escape" });
    expect(onEdit).not.toHaveBeenCalled();
  });

  it("copies the selection as TSV with Cmd+C", async () => {
    render(<DataGrid data={sampleRows} columns={sampleColumns} />);

    focusCell("Ada");
    fireEvent.keyDown(grid(), { key: "ArrowDown", shiftKey: true });
    fireEvent.keyDown(grid(), { key: "c", metaKey: true });
    await vi.waitFor(() => expect(clipboardText.value).toBe("Ada\nGrace"));
  });

  it("opens the value inspector with Space", () => {
    render(<DataGrid data={sampleRows} columns={sampleColumns} />);
    focusCell("Ada");
    fireEvent.keyDown(grid(), { key: " " });
    expect(screen.getByText("Copy value")).toBeTruthy();
  });

  it("jumps with Cmd+G via the go-to-row dialog", () => {
    render(<DataGrid data={sampleRows} columns={sampleColumns} />);
    focusCell("Ada");
    fireEvent.keyDown(grid(), { key: "g", metaKey: true });
    const input = screen.getByPlaceholderText("1–3");
    fireEvent.change(input, { target: { value: "3" } });
    fireEvent.keyDown(input, { key: "Enter" });
    const jumped = document.querySelector('[data-grid-cell="2-1"]');
    expect(jumped?.getAttribute("tabindex")).toBe("0");
  });

  it("stages deletion with Delete when rows are checkbox-selected", () => {
    const onDeleteSelectedRows = vi.fn();
    render(
      <DataGrid
        data={sampleRows}
        columns={sampleColumns}
        onDeleteSelectedRows={onDeleteSelectedRows}
      />,
    );
    const checkboxes = screen.getAllByRole("checkbox");
    fireEvent.click(checkboxes[1] as HTMLInputElement);
    fireEvent.keyDown(grid(), { key: "Delete" });
    expect(onDeleteSelectedRows).toHaveBeenCalledTimes(1);
  });
});

describe("DataGrid column layout", () => {
  beforeEach(() => {
    window.localStorage.clear();
  });

  it("persists pin-left from the header context menu per gridLayoutKey", async () => {
    render(
      <DataGrid
        data={sampleRows}
        columns={sampleColumns}
        gridLayoutKey="conn.public.users"
      />,
    );

    const header = document.querySelector('[data-grid-header="1"]');
    expect(header).not.toBeNull();
    fireEvent.contextMenu(header as HTMLElement);
    fireEvent.click(
      await screen.findByRole("menuitem", { name: "Pin column left" }),
    );

    await vi.waitFor(() => {
      const stored = JSON.parse(
        window.localStorage.getItem("dbunk.grid.layout.conn.public.users") ??
          "{}",
      );
      expect(stored.pinned).toEqual(["name"]);
    });
    // The pinned column moves to the front of the column order.
    const firstHeader = document.querySelector('[data-grid-header="0"]');
    expect(firstHeader?.textContent).toContain("name");
  });

  it("auto-fits every column from the header context menu", async () => {
    render(
      <DataGrid
        data={[["1", "a".repeat(80)]]}
        columns={sampleColumns}
        gridLayoutKey="conn.public.fit"
      />,
    );
    const header = document.querySelector('[data-grid-header="0"]');
    fireEvent.contextMenu(header as HTMLElement);
    fireEvent.click(
      await screen.findByRole("menuitem", { name: "Auto-fit all columns" }),
    );
    await vi.waitFor(() => {
      const stored = JSON.parse(
        window.localStorage.getItem("dbunk.grid.layout.conn.public.fit") ??
          "{}",
      );
      expect(stored.widths.name).toBeGreaterThan(400);
      expect(stored.widths.name).toBeLessThanOrEqual(500);
    });
  });
});
