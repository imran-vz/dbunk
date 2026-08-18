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
