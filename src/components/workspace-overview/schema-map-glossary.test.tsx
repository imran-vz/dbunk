// @vitest-environment jsdom
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

vi.mock("@/lib/tauri", () => ({
  isTauri: vi.fn(() => false),
  tauriInvoke: vi.fn(),
  errorToMessage: (error: unknown) =>
    error instanceof Error ? error.message : String(error),
}));

import { SchemaMapGlossaryButton } from "@/components/workspace-overview/schema-map-glossary";

afterEach(() => {
  cleanup();
  vi.clearAllMocks();
});

const CANONICAL_TERMS = [
  "Schema Map",
  "Table-Level Schema Map",
  "Table Card",
  "Junction Table Card",
  "Column Row",
  "Trigger Indicator",
  "Relationship Edge",
  "Relationship Cardinality",
  "Relationship Detail Popover",
  "Focused Table",
  "Focused Relationship Edge",
];

describe("SchemaMapGlossaryButton", () => {
  it("opens the glossary dialog from the toolbar button", () => {
    render(<SchemaMapGlossaryButton />);

    expect(screen.queryByTestId("schema-map-glossary")).toBeNull();

    fireEvent.click(screen.getByRole("button", { name: /glossary/i }));

    expect(screen.getByTestId("schema-map-glossary")).toBeTruthy();
  });

  it("lists every canonical Schema Map term as its own entry", () => {
    render(<SchemaMapGlossaryButton />);
    fireEvent.click(screen.getByRole("button", { name: /glossary/i }));

    const glossary = screen.getByTestId("schema-map-glossary");
    // <dt> elements — substring matching would pass with entries
    // deleted because several terms contain others ("Schema Map" is a
    // substring of "Table-Level Schema Map").
    const entries = [...glossary.querySelectorAll("dt")].map(
      (node) => node.textContent,
    );
    expect(entries).toEqual(CANONICAL_TERMS);
  });

  it("closes the glossary from its Close action", () => {
    render(<SchemaMapGlossaryButton />);
    fireEvent.click(screen.getByRole("button", { name: /glossary/i }));
    expect(screen.getByTestId("schema-map-glossary")).toBeTruthy();

    fireEvent.click(screen.getByRole("button", { name: "Close" }));

    expect(screen.queryByTestId("schema-map-glossary")).toBeNull();
  });
});
