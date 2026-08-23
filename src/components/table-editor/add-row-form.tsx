import { useState } from "react";

import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import {
  buildInsertValuesPayload,
  type InsertRowFieldMode,
  type InsertRowFormState,
  type InsertRowPayloadEntry,
  initialFormState,
} from "@/lib/insert-row-form";
import type { ColumnInfo } from "@/lib/store";

interface AddRowFormProps {
  columns: ColumnInfo[];
  isWriting: boolean;
  initialValues?: InsertRowPayloadEntry[];
  title?: string;
  submitLabel?: string;
  onSubmit: (values: InsertRowPayloadEntry[]) => Promise<void>;
  onClose: () => void;
}

function formStateWithInitialValues(
  columns: ColumnInfo[],
  initialValues: InsertRowPayloadEntry[] | undefined,
): InsertRowFormState {
  const state = initialFormState(columns);
  for (const { column, value } of initialValues ?? []) {
    if (!state[column]) continue;
    state[column] =
      value === null ? { mode: "null", value: "" } : { mode: "value", value };
  }
  return state;
}

export function AddRowForm({
  columns,
  isWriting,
  initialValues,
  title = "Add row",
  submitLabel = "Insert",
  onSubmit,
  onClose,
}: AddRowFormProps) {
  const [form, setForm] = useState<InsertRowFormState>(() =>
    formStateWithInitialValues(columns, initialValues),
  );

  const setMode = (column: string, mode: InsertRowFieldMode) => {
    setForm((prev) => ({
      ...prev,
      [column]: { mode, value: prev[column]?.value ?? "" },
    }));
  };

  const setValue = (column: string, value: string) => {
    setForm((prev) => ({
      ...prev,
      [column]: { mode: prev[column]?.mode ?? "value", value },
    }));
  };

  const handleSubmit = async () => {
    await onSubmit(buildInsertValuesPayload(form, columns));
  };

  return (
    <div
      data-testid="add-row-form"
      className="flex flex-col gap-2 border-b border-border-subtle bg-surface-panel px-3 py-2"
    >
      <div className="flex items-center justify-between">
        <span className="text-xs font-semibold">{title}</span>
        <Button variant="ghost" size="sm" onClick={onClose}>
          Cancel
        </Button>
      </div>
      <div className="grid grid-cols-1 gap-2 md:grid-cols-2">
        {columns.map((column) => (
          <AddRowField
            key={column.name}
            column={column}
            field={form[column.name] ?? { mode: "value", value: "" }}
            onModeChange={(mode) => setMode(column.name, mode)}
            onValueChange={(value) => setValue(column.name, value)}
          />
        ))}
      </div>
      <div className="flex items-center justify-end gap-2">
        <Button
          size="sm"
          disabled={isWriting}
          onClick={() => {
            void handleSubmit();
          }}
        >
          {isWriting ? "Staging…" : submitLabel}
        </Button>
      </div>
    </div>
  );
}

interface AddRowFieldProps {
  column: ColumnInfo;
  field: { mode: InsertRowFieldMode; value: string };
  onModeChange: (mode: InsertRowFieldMode) => void;
  onValueChange: (value: string) => void;
}

function AddRowField({
  column,
  field,
  onModeChange,
  onValueChange,
}: AddRowFieldProps) {
  const hasDefault =
    column.defaultValue !== null && column.defaultValue !== undefined;

  return (
    <div className="flex flex-col gap-1 rounded-sm border border-border-subtle bg-surface-app px-2 py-1.5">
      <div className="flex items-center justify-between">
        <span className="text-xs font-medium">{column.name}</span>
        <span className="text-2xs text-text-muted">{column.dataType}</span>
      </div>
      <Input
        data-testid={`add-row-value-${column.name}`}
        className="h-6 text-xs"
        value={field.value}
        placeholder={hasDefault ? `default: ${column.defaultValue}` : ""}
        disabled={field.mode !== "value"}
        onChange={(e) => onValueChange(e.target.value)}
      />
      <div className="flex items-center gap-3 text-2xs text-text-muted">
        <label className="flex items-center gap-1">
          <input
            type="radio"
            name={`add-row-mode-${column.name}`}
            data-testid={`add-row-mode-value-${column.name}`}
            checked={field.mode === "value"}
            onChange={() => onModeChange("value")}
          />
          value
        </label>
        <label className="flex items-center gap-1">
          <input
            type="radio"
            name={`add-row-mode-${column.name}`}
            data-testid={`add-row-mode-null-${column.name}`}
            disabled={!column.nullable}
            checked={field.mode === "null"}
            onChange={() => onModeChange("null")}
          />
          NULL
        </label>
        <label className="flex items-center gap-1">
          <input
            type="radio"
            name={`add-row-mode-${column.name}`}
            data-testid={`add-row-mode-default-${column.name}`}
            disabled={!hasDefault}
            checked={field.mode === "default"}
            onChange={() => onModeChange("default")}
          />
          default
        </label>
      </div>
    </div>
  );
}
