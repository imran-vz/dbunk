// @vitest-environment jsdom
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

import {
  NewEnumDialog,
  NewMaterializedViewDialog,
  NewSchemaDialog,
  NewSequenceDialog,
  NewViewDialog,
} from "@/components/object-ddl/create-object-dialogs";

afterEach(cleanup);

const commonProps = (onOps: ReturnType<typeof vi.fn>) => ({
  open: true,
  connectionId: "conn-1",
  schema: "lifecycle",
  onOpenChange: vi.fn(),
  onOps,
});

describe("create object dialogs", () => {
  it("produces createSchema ops", () => {
    const onOps = vi.fn();
    render(<NewSchemaDialog {...commonProps(onOps)} />);
    fireEvent.change(screen.getByRole("textbox", { name: "New schema name" }), {
      target: { value: " reports " },
    });
    fireEvent.click(screen.getByRole("button", { name: "Review DDL" }));
    expect(onOps).toHaveBeenCalledWith([
      { op: "createSchema", name: "reports" },
    ]);
  });

  it("produces createSequence ops with optional defaults", () => {
    const onOps = vi.fn();
    render(<NewSequenceDialog {...commonProps(onOps)} />);
    fireEvent.change(
      screen.getByRole("textbox", { name: "New sequence name" }),
      { target: { value: "invoice_number" } },
    );
    fireEvent.change(
      screen.getByRole("textbox", { name: "Sequence increment" }),
      {
        target: { value: "10" },
      },
    );
    fireEvent.change(screen.getByRole("combobox", { name: "Sequence cycle" }), {
      target: { value: "yes" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Review DDL" }));
    expect(onOps).toHaveBeenCalledWith([
      {
        op: "createSequence",
        schema: "lifecycle",
        name: "invoice_number",
        dataType: null,
        start: null,
        increment: "10",
        minValue: null,
        maxValue: null,
        cycle: true,
        cache: null,
      },
    ]);
  });

  it("produces createEnum ops in label order", () => {
    const onOps = vi.fn();
    render(<NewEnumDialog {...commonProps(onOps)} />);
    fireEvent.change(screen.getByRole("textbox", { name: "New enum name" }), {
      target: { value: "order_state" },
    });
    fireEvent.change(screen.getByRole("textbox", { name: "Enum labels" }), {
      target: { value: "new\npaid\nshipped" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Review DDL" }));
    expect(onOps).toHaveBeenCalledWith([
      {
        op: "createEnum",
        schema: "lifecycle",
        name: "order_state",
        labels: ["new", "paid", "shipped"],
      },
    ]);
  });

  it("produces createView ops", () => {
    const onOps = vi.fn();
    render(<NewViewDialog {...commonProps(onOps)} orReplace />);
    fireEvent.change(screen.getByRole("textbox", { name: "New view name" }), {
      target: { value: "recent_orders" },
    });
    fireEvent.change(screen.getByRole("textbox", { name: "SQL body" }), {
      target: { value: " SELECT * FROM lifecycle.orders " },
    });
    fireEvent.click(screen.getByRole("button", { name: "Review DDL" }));
    expect(onOps).toHaveBeenCalledWith([
      {
        op: "createView",
        schema: "lifecycle",
        name: "recent_orders",
        sqlBody: "SELECT * FROM lifecycle.orders",
        orReplace: true,
      },
    ]);
  });

  it("produces createMaterializedView ops", () => {
    const onOps = vi.fn();
    render(<NewMaterializedViewDialog {...commonProps(onOps)} />);
    fireEvent.change(
      screen.getByRole("textbox", { name: "New materialized view name" }),
      { target: { value: "order_totals" } },
    );
    fireEvent.change(screen.getByRole("textbox", { name: "SQL body" }), {
      target: { value: "SELECT sum(total) FROM lifecycle.orders" },
    });
    fireEvent.click(screen.getByRole("checkbox"));
    fireEvent.click(screen.getByRole("button", { name: "Review DDL" }));
    expect(onOps).toHaveBeenCalledWith([
      {
        op: "createMaterializedView",
        schema: "lifecycle",
        name: "order_totals",
        sqlBody: "SELECT sum(total) FROM lifecycle.orders",
        withData: false,
      },
    ]);
  });
});
