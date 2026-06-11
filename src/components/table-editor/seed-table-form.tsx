import { useState } from "react";

import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import {
  buildSeedColumnsPayload,
  DEFAULT_SEED_ROW_COUNT,
  initialSeedColumnState,
  initialSeedFormState,
  SEED_GENERATOR_OPTIONS,
  type SeedColumnFormState,
  type SeedColumnSpecPayload,
  type SeedFormState,
} from "@/lib/seed-form";
import type { ColumnInfo } from "@/lib/store";

interface SeedTableFormProps {
  columns: ColumnInfo[];
  isSeeding: boolean;
  onSubmit: (params: {
    rowCount: number;
    seed?: number;
    columns: SeedColumnSpecPayload[];
  }) => Promise<void>;
  onClose: () => void;
}

// Select item values for the per-column source dropdown: the four
// modes plus generator ids prefixed with `gen:`.
const MODE_SKIP = "skip";
const MODE_AUTO = "auto";
const MODE_CONSTANT = "constant";
const MODE_VALUES = "values";

export function SeedTableForm({
  columns,
  isSeeding,
  onSubmit,
  onClose,
}: SeedTableFormProps) {
  const [rowCountText, setRowCountText] = useState(
    String(DEFAULT_SEED_ROW_COUNT),
  );
  const [seedText, setSeedText] = useState("");
  const [form, setForm] = useState<SeedFormState>(() =>
    initialSeedFormState(columns),
  );

  const patchColumn = (column: string, patch: Partial<SeedColumnFormState>) => {
    setForm((prev) => ({
      ...prev,
      [column]: { ...(prev[column] ?? initialSeedColumnState()), ...patch },
    }));
  };

  const rowCount = Number(rowCountText);
  const rowCountValid =
    Number.isInteger(rowCount) && rowCount >= 1 && rowCount <= 1_000_000;
  const seedValid = seedText.trim() === "" || /^\d+$/.test(seedText.trim());

  const handleSubmit = async () => {
    await onSubmit({
      rowCount,
      seed: seedText.trim() === "" ? undefined : Number(seedText.trim()),
      columns: buildSeedColumnsPayload(form, columns),
    });
  };

  return (
    <div
      data-testid="seed-table-form"
      className="flex max-h-72 flex-col gap-2 overflow-y-auto border-b border-border-subtle bg-surface-panel px-3 py-2"
    >
      <div className="flex items-center justify-between">
        <span className="text-xs font-semibold">Seed table with fake data</span>
        <Button variant="ghost" size="sm" onClick={onClose}>
          Cancel
        </Button>
      </div>
      <div className="flex flex-wrap items-end gap-3">
        <label
          htmlFor="seed-row-count"
          className="flex flex-col gap-1 text-[0.625rem] text-text-muted"
        >
          Rows
          <Input
            id="seed-row-count"
            data-testid="seed-row-count"
            className="h-6 w-24 text-xs"
            value={rowCountText}
            onChange={(e) => setRowCountText(e.target.value)}
          />
        </label>
        <label
          htmlFor="seed-random-seed"
          className="flex flex-col gap-1 text-[0.625rem] text-text-muted"
        >
          Seed (optional, for reproducible data)
          <Input
            id="seed-random-seed"
            data-testid="seed-random-seed"
            className="h-6 w-40 text-xs"
            placeholder="random"
            value={seedText}
            onChange={(e) => setSeedText(e.target.value)}
          />
        </label>
      </div>
      <div className="grid grid-cols-1 gap-2 md:grid-cols-2">
        {columns.map((column) => (
          <SeedColumnField
            key={column.name}
            column={column}
            field={form[column.name] ?? initialSeedColumnState()}
            onPatch={(patch) => patchColumn(column.name, patch)}
          />
        ))}
      </div>
      <div className="flex items-center justify-end gap-2">
        <Button
          size="sm"
          data-testid="seed-submit"
          disabled={isSeeding || !rowCountValid || !seedValid}
          onClick={() => {
            void handleSubmit();
          }}
        >
          {isSeeding
            ? "Seeding…"
            : `Seed ${rowCountValid ? rowCount.toLocaleString() : ""} rows`}
        </Button>
      </div>
    </div>
  );
}

interface SeedColumnFieldProps {
  column: ColumnInfo;
  field: SeedColumnFormState;
  onPatch: (patch: Partial<SeedColumnFormState>) => void;
}

function SeedColumnField({ column, field, onPatch }: SeedColumnFieldProps) {
  const sourceValue =
    field.mode === "auto" && field.generator !== ""
      ? `gen:${field.generator}`
      : field.mode;

  const handleSourceChange = (value: string | null) => {
    if (value === null) return;
    if (value.startsWith("gen:")) {
      onPatch({ mode: "auto", generator: value.slice(4) });
    } else {
      onPatch({
        mode: value as SeedColumnFormState["mode"],
        generator: "",
      });
    }
  };

  return (
    <div className="flex flex-col gap-1 rounded-sm border border-border-subtle bg-surface-app px-2 py-1.5">
      <div className="flex items-center justify-between">
        <span className="text-xs font-medium">{column.name}</span>
        <span className="text-[0.625rem] text-text-muted">
          {column.dataType}
          {column.nullable ? "" : " · not null"}
        </span>
      </div>
      <div className="flex items-center gap-2">
        <Select value={sourceValue} onValueChange={handleSourceChange}>
          <SelectTrigger
            data-testid={`seed-source-${column.name}`}
            className="h-6 flex-1 text-xs"
          >
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            <SelectItem value={MODE_AUTO}>Auto</SelectItem>
            <SelectItem value={MODE_SKIP}>Skip (use DEFAULT)</SelectItem>
            <SelectItem value={MODE_CONSTANT}>Constant…</SelectItem>
            <SelectItem value={MODE_VALUES}>Value list…</SelectItem>
            {SEED_GENERATOR_OPTIONS.map((option) => (
              <SelectItem key={option.id} value={`gen:${option.id}`}>
                {option.label}
              </SelectItem>
            ))}
          </SelectContent>
        </Select>
        {column.nullable && field.mode !== "skip" ? (
          <label
            htmlFor={`seed-null-rate-${column.name}`}
            className="flex items-center gap-1 text-[0.625rem] text-text-muted"
          >
            NULL %
            <Input
              id={`seed-null-rate-${column.name}`}
              data-testid={`seed-null-rate-${column.name}`}
              className="h-6 w-14 text-xs"
              placeholder="10"
              value={field.nullPercent}
              onChange={(e) => onPatch({ nullPercent: e.target.value })}
            />
          </label>
        ) : null}
      </div>
      {field.mode === "constant" ? (
        <Input
          data-testid={`seed-constant-${column.name}`}
          className="h-6 text-xs"
          placeholder="Constant value"
          value={field.constant}
          onChange={(e) => onPatch({ constant: e.target.value })}
        />
      ) : null}
      {field.mode === "values" ? (
        <Input
          data-testid={`seed-values-${column.name}`}
          className="h-6 text-xs"
          placeholder="Comma-separated values"
          value={field.valuesText}
          onChange={(e) => onPatch({ valuesText: e.target.value })}
        />
      ) : null}
    </div>
  );
}
