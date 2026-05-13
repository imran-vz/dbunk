export type ImportCell = string | null;

export type ImportColumnMapping = {
  sourceColumn: string;
  targetColumn: string;
  include: boolean;
};

export type ImportSettings = {
  hasHeader: boolean;
  nullTokens: string[];
  dateFormat: string;
};

export type ParsedImportSheet = {
  name: string;
  columns: string[];
  rows: ImportCell[][];
};

export type WorkbookSheetInput = {
  name: string;
  rows: string[][];
};

const DEFAULT_NULL_TOKENS = ["NULL", "\\N"];

const normalizeHeader = (value: string, index: number): string => {
  const trimmed = value.trim();
  return trimmed.length > 0 ? trimmed : `column_${index + 1}`;
};

export function detectCsvHeader(rows: string[][]): boolean {
  const [first, second] = rows;
  if (!first || !second) {
    return false;
  }
  const firstLooksTextual = first.some((cell) => /[A-Za-z_]/.test(cell));
  const secondLooksData = second.some(
    (cell) => cell.trim() === "" || !Number.isNaN(Number(cell)),
  );
  return firstLooksTextual && secondLooksData;
}

export function parseDelimited(text: string, delimiter = ","): string[][] {
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
    } else if (char === delimiter) {
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

export function parseImportSheet(
  name: string,
  rows: string[][],
  settings: Partial<ImportSettings> = {},
): ParsedImportSheet {
  const hasHeader = settings.hasHeader ?? detectCsvHeader(rows);
  const nullTokens = new Set(settings.nullTokens ?? DEFAULT_NULL_TOKENS);
  const width = Math.max(0, ...rows.map((row) => row.length));
  const columns = hasHeader
    ? Array.from({ length: width }, (_value, index) =>
        normalizeHeader(rows[0]?.[index] ?? "", index),
      )
    : Array.from({ length: width }, (_value, index) => `column_${index + 1}`);
  const dataRows = hasHeader ? rows.slice(1) : rows;
  return {
    name,
    columns,
    rows: dataRows.map((row) =>
      columns.map((_column, index) => {
        const value = row[index] ?? "";
        return nullTokens.has(value) ? null : value;
      }),
    ),
  };
}

export function parseCsvImport(
  text: string,
  settings: Partial<ImportSettings> = {},
): ParsedImportSheet {
  return parseImportSheet("CSV import", parseDelimited(text), settings);
}

export function parseWorkbookImport(
  sheets: WorkbookSheetInput[],
  settings: Partial<ImportSettings> = {},
): ParsedImportSheet[] {
  return sheets.map((sheet) =>
    parseImportSheet(sheet.name, sheet.rows, settings),
  );
}

export function buildDefaultColumnMapping(
  sourceColumns: string[],
  targetColumns: string[],
): ImportColumnMapping[] {
  const targetByLower = new Map(
    targetColumns.map((column) => [column.toLowerCase(), column]),
  );
  return sourceColumns.map((sourceColumn) => {
    const targetColumn = targetByLower.get(sourceColumn.toLowerCase()) ?? "";
    return { sourceColumn, targetColumn, include: targetColumn.length > 0 };
  });
}

export function shouldUseCopyFastPath(params: {
  engine: "PostgreSQL" | string;
  format: "csv" | "xlsx";
  rowCount: number;
  hasTransform: boolean;
}): boolean {
  return (
    params.engine === "PostgreSQL" &&
    params.format === "csv" &&
    params.rowCount >= 10_000 &&
    !params.hasTransform
  );
}
