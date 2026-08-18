/* oxlint-disable anti-slop/no-module-mocking anti-slop/no-unknown-parameters -- Test fixtures use controlled mocks and assertions to exercise otherwise inaccessible boundaries. */
// @vitest-environment jsdom
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

vi.mock("@/lib/tauri", () => ({
  isTauri: vi.fn(() => false),
  tauriInvoke: vi.fn(),
  errorToMessage: (error: unknown) =>
    error instanceof Error ? error.message : String(error),
}));

import { SchemaMapToolbar } from "@/components/workspace-overview/schema-map-toolbar";
import { DEFAULT_SCHEMA_MAP_PREFS } from "@/lib/schema-graph";

afterEach(() => {
  cleanup();
  vi.clearAllMocks();
});

const renderToolbar = () =>
  render(
    <SchemaMapToolbar
      schemas={["public"]}
      selectedSchema="public"
      prefs={DEFAULT_SCHEMA_MAP_PREFS}
      onSchemaChange={vi.fn()}
      onPrefsChange={vi.fn()}
      onResetLayout={vi.fn()}
      onExport={vi.fn()}
    />,
  );

describe("SchemaMapToolbar", () => {
  it("includes a Glossary button", () => {
    renderToolbar();
    expect(screen.getByRole("button", { name: /glossary/i })).toBeTruthy();
  });

  it("opens the Schema Map glossary with the canonical terms", () => {
    renderToolbar();

    fireEvent.click(screen.getByRole("button", { name: /glossary/i }));

    const glossary = screen.getByTestId("schema-map-glossary");
    expect(glossary.textContent).toContain("Relationship Cardinality");
    expect(glossary.textContent).toContain("Focused Relationship Edge");
  });
});
