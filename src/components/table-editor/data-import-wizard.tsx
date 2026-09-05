import { IconUpload, IconX } from "@tabler/icons-react";
import { useMemo, useState } from "react";

import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import {
  buildImportRows,
  type ColumnMapping,
  defaultColumnMapping,
  type ParsedImportSheet,
  parseCsvSheet,
  parseXlsxSheets,
  shouldUseCopyFrom,
} from "@/lib/import";
import type { ColumnInfo } from "@/lib/store";

type DataImportWizardProps = {
  columns: ColumnInfo[];
  engine: string;
  isWriting: boolean;
  acceptedKinds?: "all" | "xlsx";
  onClose: () => void;
  onImportRows: (payload: {
    columns: string[];
    rows: Array<Array<string | null>>;
    useCopy: boolean;
  }) => Promise<void>;
};

const readFileText = async (file: File) => {
  // oxlint-disable-next-line anti-slop/no-runtime-typeof -- The value is handled at a typed library or domain boundary here.
  if (typeof file.text === "function") {
    return file.text();
  }
  return new TextDecoder().decode(await file.arrayBuffer());
};

export function DataImportWizard({
  columns,
  engine,
  isWriting,
  acceptedKinds = "all",
  onClose,
  onImportRows,
}: DataImportWizardProps) {
  const [sheets, setSheets] = useState<ParsedImportSheet[]>([]);
  const [sheetIndex, setSheetIndex] = useState(0);
  const [fileKind, setFileKind] = useState<"csv" | "xlsx">(
    acceptedKinds === "xlsx" ? "xlsx" : "csv",
  );
  const [mapping, setMapping] = useState<ColumnMapping[]>([]);
  const [nullToken, setNullToken] = useState("\\N");
  const [error, setError] = useState<string | null>(null);
  const targetColumns = useMemo(
    () => columns.map((column) => column.name),
    [columns],
  );
  const sheet = sheets[sheetIndex];
  const mappingChanged = mapping.some(
    (entry) => entry.include && entry.source !== entry.target,
  );
  const copyEligible =
    sheet !== undefined &&
    shouldUseCopyFrom({
      engine,
      fileKind,
      rowCount: sheet.rows.length,
      mappingChanged,
    });

  const loadFile = async (file: File) => {
    setError(null);
    try {
      const isXlsx = /\.xlsx$/i.test(file.name);
      if (acceptedKinds === "xlsx" && !isXlsx) {
        throw new Error(
          "PostgreSQL CSV files use the native Transfer tab. Choose an XLSX file here.",
        );
      }
      setFileKind(isXlsx ? "xlsx" : "csv");
      const parsed = isXlsx
        ? await parseXlsxSheets(await file.arrayBuffer())
        : [parseCsvSheet(await readFileText(file))];
      setSheets(parsed);
      setSheetIndex(0);
      setMapping(defaultColumnMapping(parsed[0]?.columns ?? [], targetColumns));
    } catch (error) {
      setError(error instanceof Error ? error.message : String(error));
    }
  };

  const updateMapping = (source: string, patch: Partial<ColumnMapping>) => {
    setMapping((current) =>
      current.map((entry) =>
        entry.source === source ? { ...entry, ...patch } : entry,
      ),
    );
  };

  const submit = async () => {
    if (!sheet) {
      return;
    }
    const rowsWithColumns = buildImportRows({ sheet, mapping, nullToken });
    if (rowsWithColumns.every((row) => row.length === 0)) {
      setError("Map at least one source column before importing.");
      return;
    }
    const columns = rowsWithColumns[0].map((entry) => entry.column);
    const rows = rowsWithColumns.map((row) => row.map((entry) => entry.value));
    await onImportRows({ columns, rows, useCopy: copyEligible });
  };

  return (
    <div className="border-b border-border-subtle bg-surface-window px-3 py-3">
      <div className="mb-3 flex items-center justify-between gap-3">
        <div>
          <div className="text-sm font-semibold">
            {acceptedKinds === "xlsx" ? "Import XLSX" : "Import data"}
          </div>
          <div className="text-xs text-text-muted">
            {acceptedKinds === "xlsx"
              ? "XLSX into the current table"
              : "CSV or XLSX into the current table"}
          </div>
        </div>
        <Button type="button" variant="ghost" size="icon-sm" onClick={onClose}>
          <IconX />
        </Button>
      </div>

      <div className="grid gap-3 md:grid-cols-[minmax(0,1fr)_12rem]">
        <Input
          type="file"
          aria-label="Import file"
          accept={
            acceptedKinds === "xlsx"
              ? ".xlsx,application/vnd.openxmlformats-officedocument.spreadsheetml.sheet"
              : ".csv,.xlsx,text/csv,application/vnd.openxmlformats-officedocument.spreadsheetml.sheet"
          }
          onChange={(event) => {
            const file = event.target.files?.[0];
            if (file) void loadFile(file);
          }}
        />
        <Input
          value={nullToken}
          onChange={(event) => setNullToken(event.target.value)}
          placeholder="NULL token"
          aria-label="NULL token"
        />
      </div>

      {sheets.length > 1 ? (
        <div className="mt-3 flex flex-wrap gap-1">
          {sheets.map((candidate, index) => (
            <Button
              key={candidate.name}
              type="button"
              size="sm"
              variant={index === sheetIndex ? "secondary" : "outline"}
              onClick={() => {
                setSheetIndex(index);
                setMapping(
                  defaultColumnMapping(candidate.columns, targetColumns),
                );
              }}
            >
              {candidate.name}
            </Button>
          ))}
        </div>
      ) : null}

      {sheet ? (
        <>
          <div className="mt-3 max-h-56 overflow-auto rounded-md border border-border-subtle">
            <table className="w-full text-xs">
              <thead className="bg-surface-panel text-text-muted">
                <tr>
                  <th className="px-2 py-1 text-left">Import</th>
                  <th className="px-2 py-1 text-left">Source</th>
                  <th className="px-2 py-1 text-left">Target</th>
                </tr>
              </thead>
              <tbody>
                {mapping.map((entry) => (
                  <tr
                    key={entry.source}
                    className="border-t border-border-subtle"
                  >
                    <td className="px-2 py-1">
                      <input
                        aria-label={`Import source column ${entry.source}`}
                        type="checkbox"
                        checked={entry.include}
                        onChange={(event) =>
                          updateMapping(entry.source, {
                            include: event.target.checked,
                          })
                        }
                      />
                    </td>
                    <td className="px-2 py-1 font-mono">{entry.source}</td>
                    <td className="px-2 py-1">
                      <select
                        className="h-7 rounded border border-border-subtle bg-surface-app px-2"
                        value={entry.target}
                        onChange={(event) =>
                          updateMapping(entry.source, {
                            target: event.target.value,
                            include: event.target.value.length > 0,
                          })
                        }
                      >
                        <option value="">Skip</option>
                        {targetColumns.map((column) => (
                          <option key={column} value={column}>
                            {column}
                          </option>
                        ))}
                      </select>
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>

          <div className="mt-3 flex flex-wrap items-center justify-between gap-2 text-xs text-text-muted">
            <span>
              {sheet.rows.length.toLocaleString()} rows ready
              {copyEligible ? " · COPY FROM fast path eligible" : ""}
            </span>
            <Button type="button" disabled={isWriting} onClick={submit}>
              <IconUpload />
              {isWriting ? "Importing..." : "Import rows"}
            </Button>
          </div>
        </>
      ) : null}

      {error ? <div className="mt-2 text-xs text-danger">{error}</div> : null}
    </div>
  );
}
