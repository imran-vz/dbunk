/**
 * Pure helpers that turn a column/row table into CSV or JSON text.
 *
 * The grid stores cells as strings (or null for SQL NULLs). Exporters preserve
 * that shape: nulls round-trip as JSON `null` or as a configurable CSV token,
 * empty strings stay distinct from nulls, and no numeric coercion is attempted.
 */

export type ExportCell = string | null;
export type ExportRow = ExportCell[];
export type ExportTable = {
  columns: string[];
  rows: ExportRow[];
};

export interface ToCsvOptions {
  /**
   * Token used to render a null cell. Defaults to the empty string, which
   * makes nulls and empty strings indistinguishable in the output. Pass
   * something like "NULL" to disambiguate.
   */
  nullAs?: string;
}

export interface ToJsonOptions {
  /** When true, indents the output with 2 spaces. */
  pretty?: boolean;
}

const CSV_LINE_TERMINATOR = "\n";
const CSV_NEEDS_QUOTING = /[",\r\n]/;

const quoteCsvField = (value: string): string => {
  if (!CSV_NEEDS_QUOTING.test(value)) {
    return value;
  }
  return `"${value.replace(/"/g, '""')}"`;
};

const csvCell = (cell: ExportCell, nullAs: string): string => {
  if (cell === null) {
    // The configured null token is rendered verbatim and then quoted only if
    // it itself contains delimiters. This keeps `nullAs: ""` from emitting
    // empty quotes.
    return quoteCsvField(nullAs);
  }
  return quoteCsvField(cell);
};

export function toCsv(table: ExportTable, options?: ToCsvOptions): string {
  const nullAs = options?.nullAs ?? "";
  const lines: string[] = [];
  lines.push(table.columns.map((col) => quoteCsvField(col)).join(","));
  for (const row of table.rows) {
    lines.push(row.map((cell) => csvCell(cell, nullAs)).join(","));
  }
  return lines.join(CSV_LINE_TERMINATOR);
}

export function toJson(table: ExportTable, options?: ToJsonOptions): string {
  const objects = table.rows.map((row) => {
    const obj: Record<string, ExportCell> = {};
    table.columns.forEach((col, index) => {
      // `row[index]` may be undefined if a row is shorter than the column
      // list; coerce to null so JSON.stringify keeps the key.
      const cell = row[index];
      obj[col] = cell === undefined ? null : cell;
    });
    return obj;
  });
  return JSON.stringify(objects, null, options?.pretty ? 2 : undefined);
}
