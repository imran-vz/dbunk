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

export type ExportFormat =
  | "csv"
  | "json"
  | "sql"
  | "html"
  | "markdown"
  | "txt"
  | "xlsx";

export type ExportScope =
  | { kind: "result-set" }
  | { kind: "whole-table"; schema: string; table: string };

export type ExportEncoding = "utf-8" | "utf-16le";
export type ExportCompression = "none" | "gzip";

export interface ToJsonOptions {
  /** When true, indents the output with 2 spaces. */
  pretty?: boolean;
}

export interface ToSqlInsertOptions {
  schema?: string;
  table: string;
  nullAsDefault?: boolean;
}

export interface ExportTaskConfig {
  id: string;
  name: string;
  connectionId: string;
  scope: ExportScope;
  format: ExportFormat;
  encoding: ExportEncoding;
  compression: ExportCompression;
  nullAs?: string;
  includeHeader: boolean;
  createdAt: string;
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

const quoteSqlIdentifier = (identifier: string): string =>
  `"${identifier.replace(/"/g, '""')}"`;

const quoteSqlLiteral = (value: string): string =>
  `'${value.replace(/'/g, "''")}'`;

const qualifiedSqlTable = (
  schema: string | undefined,
  table: string,
): string =>
  schema && schema.length > 0
    ? `${quoteSqlIdentifier(schema)}.${quoteSqlIdentifier(table)}`
    : quoteSqlIdentifier(table);

export function toSqlInserts(
  table: ExportTable,
  options: ToSqlInsertOptions,
): string {
  const columns = table.columns.map(quoteSqlIdentifier).join(", ");
  const target = qualifiedSqlTable(options.schema, options.table);
  return table.rows
    .map((row) => {
      const values = table.columns.map((_column, index) => {
        const cell = row[index];
        if (cell === null || cell === undefined) {
          return options.nullAsDefault ? "DEFAULT" : "NULL";
        }
        return quoteSqlLiteral(cell);
      });
      return `INSERT INTO ${target} (${columns}) VALUES (${values.join(", ")});`;
    })
    .join("\n");
}

const escapeHtml = (value: string): string =>
  value
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;");

export function toHtmlTable(table: ExportTable, nullAs = ""): string {
  const header = table.columns
    .map((column) => `<th>${escapeHtml(column)}</th>`)
    .join("");
  const rows = table.rows
    .map((row) => {
      const cells = table.columns
        .map((_column, index) => {
          const cell = row[index] ?? null;
          return `<td>${escapeHtml(cell === null ? nullAs : cell)}</td>`;
        })
        .join("");
      return `<tr>${cells}</tr>`;
    })
    .join("\n");
  return `<table>\n<thead><tr>${header}</tr></thead>\n<tbody>${rows}</tbody>\n</table>`;
}

const escapeMarkdownCell = (value: string): string =>
  value.replace(/\\/g, "\\\\").replace(/\|/g, "\\|").replace(/\r?\n/g, "<br>");

export function toMarkdownTable(table: ExportTable, nullAs = ""): string {
  const header = `| ${table.columns.map(escapeMarkdownCell).join(" | ")} |`;
  const separator = `| ${table.columns.map(() => "---").join(" | ")} |`;
  const rows = table.rows.map((row) => {
    const values = table.columns.map((_column, index) => {
      const cell = row[index] ?? null;
      return escapeMarkdownCell(cell === null ? nullAs : cell);
    });
    return `| ${values.join(" | ")} |`;
  });
  return [header, separator, ...rows].join("\n");
}

export function toTxtTable(table: ExportTable, nullAs = ""): string {
  return [
    table.columns.join("\t"),
    ...table.rows.map((row) =>
      table.columns
        .map((_column, index) => {
          const cell = row[index] ?? null;
          return cell === null ? nullAs : cell.replace(/\r?\n/g, " ");
        })
        .join("\t"),
    ),
  ].join("\n");
}

export function toExcelWorkbookXml(table: ExportTable, nullAs = ""): string {
  const rows = [
    table.columns,
    ...table.rows.map((row) =>
      table.columns.map((_column, index) => row[index] ?? nullAs),
    ),
  ];
  const xmlRows = rows
    .map((row) => {
      const cells = row
        .map(
          (cell) =>
            `<Cell><Data ss:Type="String">${escapeHtml(cell ?? nullAs)}</Data></Cell>`,
        )
        .join("");
      return `<Row>${cells}</Row>`;
    })
    .join("");
  return `<?xml version="1.0" encoding="UTF-8"?><?mso-application progid="Excel.Sheet"?><Workbook xmlns="urn:schemas-microsoft-com:office:spreadsheet" xmlns:ss="urn:schemas-microsoft-com:office:spreadsheet"><Worksheet ss:Name="Export"><Table>${xmlRows}</Table></Worksheet></Workbook>`;
}

export function serializeExportTask(config: ExportTaskConfig): string {
  return JSON.stringify(config, null, 2);
}
