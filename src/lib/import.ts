import * as XLSX from "xlsx";

export type ImportCell = string | null;

export type ParsedImportSheet = {
  name: string;
  columns: string[];
  rows: string[][];
};

export type ImportSettings = {
  hasHeader: boolean;
  nullToken: string;
};

export type ColumnMapping = {
  source: string;
  target: string;
  include: boolean;
};

export function parseCsv(text: string): string[][] {
  const rows: string[][] = [];
  let row: string[] = [];
  let cell = "";
  let quoted = false;
  for (let index = 0; index < text.length; index += 1) {
    const char = text[index];
    const next = text[index + 1];
    if (quoted) {
      if (char === '"' && next === '"') {
        cell += '"';
        index += 1;
      } else if (char === '"') {
        quoted = false;
      } else {
        cell += char;
      }
      continue;
    }
    if (char === '"') {
      quoted = true;
    } else if (char === ",") {
      row.push(cell);
      cell = "";
    } else if (char === "\n") {
      row.push(cell);
      rows.push(row);
      row = [];
      cell = "";
    } else if (char !== "\r") {
      cell += char;
    }
  }
  row.push(cell);
  if (row.length > 1 || row[0] !== "") {
    rows.push(row);
  }
  return rows;
}

export function detectHeader(rows: string[][]): boolean {
  const first = rows[0] ?? [];
  const second = rows[1] ?? [];
  if (first.length === 0 || second.length === 0) {
    return false;
  }
  const firstHasText = first.some((cell) => /[A-Za-z_]/.test(cell));
  const secondHasData = second.some(
    (cell) => cell.trim() === "" || !Number.isNaN(Number(cell)),
  );
  return firstHasText && secondHasData;
}

export function normalizeRows(
  name: string,
  rows: string[][],
  hasHeader = detectHeader(rows),
): ParsedImportSheet {
  const width = Math.max(0, ...rows.map((row) => row.length));
  const columns = Array.from({ length: width }, (_value, index) => {
    if (!hasHeader) {
      return `column_${index + 1}`;
    }
    const header = rows[0]?.[index]?.trim();
    return header ? header : `column_${index + 1}`;
  });
  return {
    name,
    columns,
    rows: (hasHeader ? rows.slice(1) : rows).map((row) =>
      columns.map((_column, index) => row[index] ?? ""),
    ),
  };
}

export function parseCsvSheet(text: string): ParsedImportSheet {
  const rows = parseCsv(text);
  return normalizeRows("CSV import", rows);
}

export function parseXlsxSheets(data: ArrayBuffer): ParsedImportSheet[] {
  const workbook = XLSX.read(data, { type: "array" });
  return workbook.SheetNames.map((sheetName) => {
    const worksheet = workbook.Sheets[sheetName];
    const rows = XLSX.utils.sheet_to_json<string[]>(worksheet, {
      header: 1,
      raw: false,
      defval: "",
    });
    return normalizeRows(sheetName, rows);
  });
}

export function defaultColumnMapping(
  sourceColumns: string[],
  targetColumns: string[],
): ColumnMapping[] {
  const targetByLower = new Map(
    targetColumns.map((column) => [column.toLowerCase(), column]),
  );
  return sourceColumns.map((source) => {
    const target = targetByLower.get(source.toLowerCase()) ?? "";
    return { source, target, include: target.length > 0 };
  });
}

export function buildImportRows(params: {
  sheet: ParsedImportSheet;
  mapping: ColumnMapping[];
  nullToken: string;
}): Array<Array<{ column: string; value: string | null }>> {
  const sourceIndex = new Map(
    params.sheet.columns.map((column, index) => [column, index]),
  );
  const active = params.mapping.filter(
    (entry) => entry.include && entry.target,
  );
  return params.sheet.rows.map((row) =>
    active.map((entry) => {
      const value = row[sourceIndex.get(entry.source) ?? -1] ?? "";
      return {
        column: entry.target,
        value: value === params.nullToken ? null : value,
      };
    }),
  );
}

export function shouldUseCopyFrom(params: {
  engine: string;
  fileKind: "csv" | "xlsx";
  rowCount: number;
  mappingChanged: boolean;
}): boolean {
  return (
    params.engine === "PostgreSQL" &&
    params.fileKind === "csv" &&
    params.rowCount >= 10_000 &&
    !params.mappingChanged
  );
}
