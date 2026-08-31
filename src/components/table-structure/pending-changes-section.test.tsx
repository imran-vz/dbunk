// @vitest-environment jsdom
import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

import type { ColumnChangeKind, PendingChange } from "@/lib/ddl";
import type { DDLOutcome, StructureCommitStatus } from "@/lib/store";

import { PendingChangesSection } from "./pending-changes-section";

const change = (overrides: Partial<PendingChange> = {}): PendingChange => ({
  id: "p-1",
  schema: "public",
  table: "users",
  change: { kind: "column", change: { kind: "drop", columnName: "email" } },
  ...overrides,
});

const renderSection = (
  overrides: Partial<React.ComponentProps<typeof PendingChangesSection>> = {},
) => {
  const onTogglePreview = vi.fn();
  const onRemove = vi.fn();
  const onCommit = vi.fn();
  const props: React.ComponentProps<typeof PendingChangesSection> = {
    pending: [],
    previewSql: "",
    showPreview: false,
    commitStatus: undefined,
    lastOutcome: null,
    onTogglePreview,
    onRemove,
    onCommit,
    ...overrides,
  };
  render(<PendingChangesSection {...props} />);
  return { onTogglePreview, onRemove, onCommit };
};

describe("PendingChangesSection", () => {
  it("disables Preview and Commit when there are no pending changes", () => {
    renderSection();
    expect(
      screen.getByTestId("structure-preview-sql").hasAttribute("disabled"),
    ).toBe(true);
    expect(
      screen.getByTestId("structure-commit").hasAttribute("disabled"),
    ).toBe(true);
    expect(screen.getByText("No pending changes.")).toBeTruthy();
  });

  it("enables Commit when there are pending changes and not running", () => {
    renderSection({ pending: [change()] });
    expect(
      screen.getByTestId("structure-commit").hasAttribute("disabled"),
    ).toBe(false);
  });

  it("disables Commit while running", () => {
    const status: StructureCommitStatus = { state: "running" };
    renderSection({ pending: [change()], commitStatus: status });
    const button = screen.getByTestId("structure-commit");
    expect(button.hasAttribute("disabled")).toBe(true);
    expect(button.textContent).toContain("Committing");
  });

  it("invokes onRemove when a pending row's X button is clicked", () => {
    const { onRemove } = renderSection({ pending: [change()] });
    fireEvent.click(screen.getByTestId("structure-remove-pending-p-1"));
    expect(onRemove).toHaveBeenCalledWith("p-1");
  });

  it("invokes onTogglePreview when Preview SQL is clicked", () => {
    const { onTogglePreview } = renderSection({ pending: [change()] });
    fireEvent.click(screen.getByTestId("structure-preview-sql"));
    expect(onTogglePreview).toHaveBeenCalledTimes(1);
  });

  it("renders SQL when preview is shown and previewSql is non-empty", () => {
    renderSection({
      pending: [change()],
      showPreview: true,
      previewSql: "ALTER TABLE users DROP COLUMN email;",
    });
    const preview = screen.getByTestId("structure-sql-preview");
    expect(preview.textContent).toContain("ALTER TABLE");
  });

  it("does not render preview when previewSql is empty", () => {
    renderSection({
      pending: [change()],
      showPreview: true,
      previewSql: "",
    });
    expect(screen.queryByTestId("structure-sql-preview")).toBeNull();
  });

  it("shows the success banner when lastOutcome is completed", () => {
    const outcome: DDLOutcome = { kind: "completed", runtimeMs: 42 };
    renderSection({ pending: [change()], lastOutcome: outcome });
    const banner = screen.getByTestId("structure-commit-success");
    expect(banner.textContent).toContain("42");
  });

  it("shows the error banner when lastOutcome is failed", () => {
    const outcome: DDLOutcome = { kind: "failed", reason: "syntax error" };
    renderSection({ pending: [change()], lastOutcome: outcome });
    const banner = screen.getByTestId("structure-commit-error");
    expect(banner.textContent).toContain("syntax error");
  });

  it("does not show any banner when lastOutcome is noop", () => {
    const outcome: DDLOutcome = { kind: "noop" };
    renderSection({ pending: [change()], lastOutcome: outcome });
    expect(screen.queryByTestId("structure-commit-success")).toBeNull();
    expect(screen.queryByTestId("structure-commit-error")).toBeNull();
  });

  it("describes each pending change variant", () => {
    const entry = (
      id: string,
      columnChange: ColumnChangeKind,
    ): PendingChange => ({
      id,
      schema: "public",
      table: "users",
      change: { kind: "column", change: columnChange },
    });
    const pending: PendingChange[] = [
      entry("p-1", {
        kind: "add",
        column: {
          name: "age",
          dataType: "int",
          nullable: true,
          defaultValue: null,
        },
      }),
      entry("p-2", { kind: "rename", columnName: "old", newName: "new" }),
      entry("p-3", { kind: "set_type", columnName: "age", newType: "bigint" }),
      entry("p-4", {
        kind: "set_nullable",
        columnName: "age",
        nullable: false,
      }),
      entry("p-5", { kind: "set_default", columnName: "age", default: "0" }),
      entry("p-6", { kind: "set_default", columnName: "age", default: null }),
    ];
    renderSection({ pending });
    expect(screen.getByText(/Add column age int/)).toBeTruthy();
    expect(screen.getByText(/Rename old -> new/)).toBeTruthy();
    expect(screen.getByText(/Change type of age to bigint/)).toBeTruthy();
    expect(screen.getByText(/Make age NOT NULL/)).toBeTruthy();
    expect(screen.getByText(/Set default of age to 0/)).toBeTruthy();
    expect(screen.getByText(/Drop default for age/)).toBeTruthy();
  });
});
