/**
 * Pure helpers that turn a column/row table into CSV or JSON text.
 *
 * The grid stores cells as strings (or null for SQL NULLs). Exporters preserve
 * that shape: nulls round-trip as JSON `null` or as a configurable CSV token,
 * empty strings stay distinct from nulls, and no numeric coercion is attempted.
 */

import { isTauri, tauriInvoke } from "@/lib/tauri";

export type ExportCell = string | null;
export type ExportRow = ExportCell[];
export type ExportTable = {
  columns: string[];
  rows: ExportRow[];
};

export type ExportFormat =
  | "csv"
  | "json"
  | "sql"
  | "html"
  | "markdown"
  | "txt"
  | "xlsx";

export type ExportEncoding = "utf-8" | "utf-16le";
export type ExportCompression = "none" | "gzip";

export type ExportOptions = {
  format: ExportFormat;
  filenameBase: string;
  encoding: ExportEncoding;
  compression: ExportCompression;
  nullAs?: string;
  sqlTableName?: string;
};

export type PreparedExport = {
  filename: string;
  mime: string;
  content: BlobPart;
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

export interface ToSqlOptions {
  tableName: string;
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

const escapeHtml = (value: string): string =>
  value
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;");

const quoteSqlIdentifier = (identifier: string): string =>
  `"${identifier.replace(/"/g, '""')}"`;

const quoteSqlLiteral = (value: string): string =>
  `'${value.replace(/'/g, "''")}'`;

export function toSqlInserts(
  table: ExportTable,
  options: ToSqlOptions,
): string {
  const target = options.tableName.split(".").map(quoteSqlIdentifier).join(".");
  const columns = table.columns.map(quoteSqlIdentifier).join(", ");
  return table.rows
    .map((row) => {
      const values = table.columns.map((_column, index) => {
        const cell = row[index];
        return cell === null || cell === undefined
          ? "NULL"
          : quoteSqlLiteral(cell);
      });
      return `INSERT INTO ${target} (${columns}) VALUES (${values.join(", ")});`;
    })
    .join("\n");
}

export function toHtml(table: ExportTable, nullAs = ""): string {
  const head = table.columns
    .map((column) => `<th>${escapeHtml(column)}</th>`)
    .join("");
  const body = table.rows
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
  return `<!doctype html>
<html>
<head><meta charset="utf-8"><title>dbunk export</title></head>
<body><table>
<thead><tr>${head}</tr></thead>
<tbody>
${body}
</tbody>
</table></body>
</html>`;
}

const escapeMarkdown = (value: string): string =>
  value.replace(/\\/g, "\\\\").replace(/\|/g, "\\|").replace(/\r?\n/g, "<br>");

export function toMarkdown(table: ExportTable, nullAs = ""): string {
  const header = `| ${table.columns.map(escapeMarkdown).join(" | ")} |`;
  const separator = `| ${table.columns.map(() => "---").join(" | ")} |`;
  const rows = table.rows.map((row) => {
    const values = table.columns.map((_column, index) => {
      const cell = row[index] ?? null;
      return escapeMarkdown(cell === null ? nullAs : cell);
    });
    return `| ${values.join(" | ")} |`;
  });
  return [header, separator, ...rows].join("\n");
}

export function toTxt(table: ExportTable, nullAs = ""): string {
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

const encodeText = (text: string, encoding: ExportEncoding): BlobPart => {
  if (encoding === "utf-8") {
    return text;
  }
  const bytes = new Uint8Array(text.length * 2 + 2);
  bytes[0] = 0xff;
  bytes[1] = 0xfe;
  for (let index = 0; index < text.length; index += 1) {
    const code = text.charCodeAt(index);
    bytes[index * 2 + 2] = code & 0xff;
    bytes[index * 2 + 3] = code >> 8;
  }
  return bytes;
};

export function prepareExport(
  table: ExportTable,
  options: ExportOptions,
): PreparedExport {
  const nullAs = options.nullAs ?? "";
  const ext = options.format === "markdown" ? "md" : options.format;
  const filename =
    options.compression === "gzip"
      ? `${options.filenameBase}.${ext}.gz`
      : `${options.filenameBase}.${ext}`;
  if (options.format === "xlsx") {
    throw new Error(
      "XLSX export must use prepareExportBlob (requires async Tauri backend).",
    );
  }

  const text = exportableText(options, table, nullAs);
  return {
    filename,
    mime: mimeForFormat(options.format, options.encoding),
    content: encodeText(text, options.encoding),
  };
}

function exportableText(
  options: ExportOptions,
  table: ExportTable,
  nullAs: string,
) {
  switch (options.format) {
    case "csv":
      return toCsv(table, { nullAs });
    case "json":
      return toJson(table, { pretty: true });
    case "sql":
      return toSqlInserts(table, {
        tableName: options.sqlTableName ?? options.filenameBase,
      });
    case "html":
      return toHtml(table, nullAs);
    case "markdown":
      return toMarkdown(table, nullAs);
    default:
      return toTxt(table, nullAs);
  }
}

async function compressGzip(blob: Blob): Promise<Blob> {
  if (typeof CompressionStream === "undefined") {
    throw new Error("Gzip compression is not supported in this webview.");
  }
  const compressed = await new Response(
    blob.stream().pipeThrough(new CompressionStream("gzip")),
  ).blob();
  return new Blob([compressed], { type: "application/gzip" });
}

export async function prepareExportBlob(
  table: ExportTable,
  options: ExportOptions,
): Promise<{ filename: string; blob: Blob }> {
  // For XLSX, use the Rust backend which produces a proper Excel file
  // with numeric cells, correct shared-strings, etc.
  if (options.format === "xlsx" && isTauri()) {
    const nullAs = options.nullAs ?? "";
    const columns = table.columns;
    const rows = table.rows.map((row) =>
      row.map((cell) => (cell === null ? nullAs : cell)),
    );
    // Tauri v2 returns raw bytes as ArrayBuffer when the backend
    // uses `tauri::ipc::Response` with a Vec<u8> body.
    const buffer = await tauriInvoke<ArrayBuffer>("export_xlsx", {
      payload: { columns, rows, sheetName: options.filenameBase || "Export" },
    });
    const ext = "xlsx";
    const filename =
      options.compression === "gzip"
        ? `${options.filenameBase}.${ext}.gz`
        : `${options.filenameBase}.${ext}`;
    const blob = new Blob([buffer], {
      type: "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
    });
    if (options.compression === "gzip") {
      return { filename, blob: await compressGzip(blob) };
    }
    return { filename, blob };
  }

  const prepared = prepareExport(table, options);
  const blob = new Blob([prepared.content], { type: prepared.mime });
  if (options.compression !== "gzip") {
    return { filename: prepared.filename, blob };
  }
  return {
    filename: prepared.filename,
    blob: await compressGzip(blob),
  };
}

export function mimeForFormat(
  format: ExportFormat,
  encoding: ExportEncoding,
): string {
  const charset = `charset=${encoding}`;
  switch (format) {
    case "csv":
      return `text/csv;${charset}`;
    case "json":
      return `application/json;${charset}`;
    case "sql":
      return `application/sql;${charset}`;
    case "html":
      return `text/html;${charset}`;
    case "markdown":
      return `text/markdown;${charset}`;
    case "txt":
      return `text/plain;${charset}`;
    case "xlsx":
      return "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet";
  }
}
