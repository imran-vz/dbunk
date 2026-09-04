/* oxlint-disable anti-slop/no-module-mocking -- Monaco is replaced with a textarea in jsdom. */
// @vitest-environment jsdom
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

vi.mock("@monaco-editor/react", () => ({
  default: ({
    value,
    onChange,
    options,
  }: {
    value: string;
    onChange: (value: string) => void;
    options?: { readOnly?: boolean };
  }) => (
    <textarea
      aria-label="Routine body"
      value={value}
      readOnly={options?.readOnly}
      onChange={(event) => onChange(event.target.value)}
    />
  ),
}));

import { RoutineEditorDialog } from "./routine-editor-dialog";

afterEach(cleanup);

const facts = {
  kind: "routine" as const,
  language: "sql",
  returns: "integer",
  volatility: "stable",
  arguments: "value integer",
  body: "SELECT value + 1",
  strict: true,
  securityDefiner: true,
  parallel: "safe",
};

describe("routine editor", () => {
  it("emits the exact create-function operation", () => {
    const onOpenChange = vi.fn();
    const onOps = vi.fn();
    render(
      <RoutineEditorDialog
        open
        onOpenChange={onOpenChange}
        connectionId="conn-1"
        kind="function"
        schema="lifecycle"
        onOps={onOps}
      />,
    );

    fireEvent.change(screen.getByLabelText("Routine name"), {
      target: { value: " increment " },
    });
    fireEvent.change(screen.getByLabelText("Routine arguments"), {
      target: { value: " value integer " },
    });
    fireEvent.change(screen.getByLabelText("Routine returns"), {
      target: { value: " integer " },
    });
    fireEvent.change(screen.getByLabelText("Routine language"), {
      target: { value: " sql " },
    });
    fireEvent.change(screen.getByLabelText("Routine volatility"), {
      target: { value: "stable" },
    });
    fireEvent.change(screen.getByLabelText("Routine parallel"), {
      target: { value: "safe" },
    });
    fireEvent.click(screen.getByRole("checkbox", { name: "STRICT" }));
    fireEvent.click(screen.getByRole("checkbox", { name: "SECURITY DEFINER" }));
    fireEvent.change(screen.getByLabelText("Routine body"), {
      target: { value: " SELECT value + 1 " },
    });
    fireEvent.click(screen.getByRole("button", { name: "Review DDL" }));

    expect(onOps).toHaveBeenCalledWith([
      {
        op: "createFunction",
        schema: "lifecycle",
        name: "increment",
        orReplace: false,
        arguments: "value integer",
        returns: "integer",
        language: "sql",
        body: " SELECT value + 1 ",
        volatility: "stable",
        strict: true,
        securityDefiner: true,
        parallel: "safe",
      },
    ]);
    expect(onOpenChange).toHaveBeenCalledWith(false);
  });

  it("round-trips every function header, fixes schema, and submits signature edits", () => {
    const onOps = vi.fn();
    render(
      <RoutineEditorDialog
        open
        onOpenChange={vi.fn()}
        connectionId="conn-1"
        kind="function"
        schema="lifecycle"
        name="increment"
        facts={facts}
        onOps={onOps}
      />,
    );

    expect(
      screen.getByText(
        /PostgreSQL may create a different routine or reject signature and return-type changes/,
      ),
    ).toBeTruthy();
    expect(
      screen.getByLabelText("Routine schema").hasAttribute("disabled"),
    ).toBe(true);
    for (const name of ["Routine name", "Routine arguments"]) {
      expect(screen.getByLabelText(name).hasAttribute("disabled")).toBe(false);
    }
    expect(
      screen.getByLabelText("Routine returns").hasAttribute("disabled"),
    ).toBe(false);

    fireEvent.change(screen.getByLabelText("Routine name"), {
      target: { value: "increment_v2" },
    });
    fireEvent.change(screen.getByLabelText("Routine arguments"), {
      target: { value: "value bigint" },
    });
    fireEvent.change(screen.getByLabelText("Routine returns"), {
      target: { value: "bigint" },
    });

    fireEvent.click(screen.getByRole("button", { name: "Review DDL" }));
    expect(onOps).toHaveBeenCalledWith([
      {
        op: "createFunction",
        schema: "lifecycle",
        name: "increment_v2",
        orReplace: true,
        arguments: "value bigint",
        returns: "bigint",
        language: "sql",
        body: "SELECT value + 1",
        volatility: "stable",
        strict: true,
        securityDefiner: true,
        parallel: "safe",
      },
    ]);
  });

  it("hides returns for procedures and emits the exact procedure operation", () => {
    const onOps = vi.fn();
    render(
      <RoutineEditorDialog
        open
        onOpenChange={vi.fn()}
        connectionId="conn-1"
        kind="procedure"
        schema="lifecycle"
        onOps={onOps}
      />,
    );
    expect(
      screen.queryByRole("textbox", { name: "Routine returns" }),
    ).toBeNull();
    fireEvent.change(screen.getByRole("textbox", { name: "Routine name" }), {
      target: { value: "archive_orders" },
    });
    fireEvent.change(screen.getByLabelText("Routine arguments"), {
      target: { value: " before_date date " },
    });
    fireEvent.change(screen.getByLabelText("Routine language"), {
      target: { value: "sql" },
    });
    fireEvent.change(screen.getByLabelText("Routine body"), {
      target: { value: "DELETE FROM orders WHERE created_at < before_date" },
    });
    fireEvent.click(screen.getByRole("checkbox", { name: "SECURITY DEFINER" }));
    fireEvent.click(screen.getByRole("button", { name: "Review DDL" }));
    expect(onOps).toHaveBeenCalledWith([
      {
        op: "createProcedure",
        schema: "lifecycle",
        name: "archive_orders",
        orReplace: false,
        arguments: "before_date date",
        language: "sql",
        body: "DELETE FROM orders WHERE created_at < before_date",
        securityDefiner: true,
      },
    ]);
  });

  it.each(["c", "internal"])(
    "keeps original %s routines wholly read-only",
    (language) => {
      const props = {
        open: true,
        onOpenChange: vi.fn(),
        connectionId: "conn-1",
        kind: "function" as const,
        schema: "pg_catalog",
        name: "abs",
        onOps: vi.fn(),
      };
      const { rerender } = render(
        <RoutineEditorDialog {...props} facts={{ ...facts, language }} />,
      );

      expect(screen.getByRole("note").textContent).toContain(
        "not editable source",
      );
      for (const name of [
        "Routine schema",
        "Routine name",
        "Routine arguments",
        "Routine returns",
        "Routine language",
        "Routine volatility",
        "Routine parallel",
      ]) {
        expect(screen.getByLabelText(name).hasAttribute("disabled")).toBe(true);
      }
      for (const name of ["STRICT", "SECURITY DEFINER"]) {
        expect(
          screen.getByRole("checkbox", { name }).hasAttribute("disabled"),
        ).toBe(true);
      }
      expect(
        screen
          .getByRole("button", { name: "Review DDL" })
          .hasAttribute("disabled"),
      ).toBe(true);
      expect(
        screen
          .getByRole("textbox", { name: "Routine body" })
          .hasAttribute("readonly"),
      ).toBe(true);

      fireEvent.change(screen.getByLabelText("Routine language"), {
        target: { value: "sql" },
      });
      rerender(
        <RoutineEditorDialog
          {...props}
          facts={{ ...facts, language: "sql" }}
        />,
      );
      expect(
        screen
          .getByRole("button", { name: "Review DDL" })
          .hasAttribute("disabled"),
      ).toBe(true);
    },
  );
});
