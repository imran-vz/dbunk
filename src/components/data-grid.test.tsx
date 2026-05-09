// @vitest-environment jsdom
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("@/lib/download", () => ({
  downloadFile: vi.fn(),
}));

import { DataGrid } from "@/components/data-grid";
import { downloadFile } from "@/lib/download";

const mockedDownloadFile = vi.mocked(downloadFile);

const sampleColumns = ["id", "name"];
const sampleRows = [
  ["1", "Ada"],
  ["2", "Grace"],
  ["3", "Edsger"],
];

const openMoreMenu = () => {
  // The grid's "more" trigger is the only icon button without an aria label —
  // grab it by the icon's data-slot path. The base-ui menu portals the popup
  // into the body so we click the trigger to expose the menu items.
  const triggers = screen.getAllByRole("button");
  // The dots trigger is the last button with no accessible name in the
  // toolbar. Find it by its empty aria-label/text and presence of an svg.
  const trigger = triggers.find(
    (btn) =>
      btn.getAttribute("aria-haspopup") === "menu" &&
      btn.querySelector("svg.tabler-icon-dots") !== null,
  );
  if (!trigger) {
    throw new Error("could not find more-actions trigger");
  }
  fireEvent.click(trigger);
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
  mockedDownloadFile.mockClear();
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

  it("downloads a CSV with header and all rows when 'Export all to .csv' is clicked", () => {
    render(
      <DataGrid
        data={sampleRows}
        columns={sampleColumns}
        exportFilenameBase="conn-public-users-2026-05-09"
      />,
    );
    openMoreMenu();
    fireEvent.click(findMenuItem(/Export all to \.csv/i));

    expect(mockedDownloadFile).toHaveBeenCalledTimes(1);
    const [filename, mime, content] = mockedDownloadFile.mock.calls[0] ?? [];
    expect(filename).toBe("conn-public-users-2026-05-09.csv");
    expect(mime).toContain("text/csv");
    expect(content).toBe("id,name\n1,Ada\n2,Grace\n3,Edsger");
  });

  it("downloads JSON with all rows when 'Export all to .json' is clicked", () => {
    render(
      <DataGrid
        data={sampleRows}
        columns={sampleColumns}
        exportFilenameBase="conn-public-users-2026-05-09"
      />,
    );
    openMoreMenu();
    fireEvent.click(findMenuItem(/Export all to \.json/i));

    expect(mockedDownloadFile).toHaveBeenCalledTimes(1);
    const [filename, mime, content] = mockedDownloadFile.mock.calls[0] ?? [];
    expect(filename).toBe("conn-public-users-2026-05-09.json");
    expect(mime).toContain("application/json");
    expect(content).toContain('"id": "1"');
    expect(content).toContain('"name": "Ada"');
    // Should be valid JSON of all 3 rows.
    const parsed = JSON.parse(content as string);
    expect(parsed).toHaveLength(3);
  });

  it("exports only selected rows when 'Export selected to .csv' is clicked", () => {
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

    expect(mockedDownloadFile).toHaveBeenCalledTimes(1);
    const [filename, , content] = mockedDownloadFile.mock.calls[0] ?? [];
    expect(filename).toBe("conn-public-users-2026-05-09-selected.csv");
    expect(content).toBe("id,name\n1,Ada\n3,Edsger");
  });

  it("exports only selected rows when 'Export selected to .json' is clicked", () => {
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

    expect(mockedDownloadFile).toHaveBeenCalledTimes(1);
    const [filename, , content] = mockedDownloadFile.mock.calls[0] ?? [];
    expect(filename).toBe("conn-public-users-2026-05-09-selected.json");
    const parsed = JSON.parse(content as string);
    expect(parsed).toEqual([{ id: "2", name: "Grace" }]);
  });
});
