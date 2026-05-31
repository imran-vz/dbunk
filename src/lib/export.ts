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

const crcTable = Array.from({ length: 256 }, (_value, index) => {
  let crc = index;
  for (let bit = 0; bit < 8; bit += 1) {
    crc = crc & 1 ? 0xedb88320 ^ (crc >>> 1) : crc >>> 1;
  }
  return crc >>> 0;
});

const crc32 = (data: Uint8Array): number => {
  let crc = 0xffffffff;
  for (const byte of data) {
    crc = crcTable[(crc ^ byte) & 0xff] ^ (crc >>> 8);
  }
  return (crc ^ 0xffffffff) >>> 0;
};

const textEncoder = new TextEncoder();

const dosDateTime = () => {
  const now = new Date();
  const time =
    (now.getHours() << 11) | (now.getMinutes() << 5) | (now.getSeconds() >> 1);
  const date =
    ((now.getFullYear() - 1980) << 9) |
    ((now.getMonth() + 1) << 5) |
    now.getDate();
  return { time, date };
};

const u16 = (value: number) => {
  const bytes = new Uint8Array(2);
  new DataView(bytes.buffer).setUint16(0, value, true);
  return bytes;
};

const u32 = (value: number) => {
  const bytes = new Uint8Array(4);
  new DataView(bytes.buffer).setUint32(0, value, true);
  return bytes;
};

const concatBytes = (chunks: Uint8Array[]): Uint8Array => {
  const length = chunks.reduce((total, chunk) => total + chunk.length, 0);
  const out = new Uint8Array(length);
  let offset = 0;
  for (const chunk of chunks) {
    out.set(chunk, offset);
    offset += chunk.length;
  }
  return out;
};

const sheetXml = (table: ExportTable): string => {
  const rows = [table.columns, ...table.rows];
  const xmlRows = rows
    .map((row, rowIndex) => {
      const cells = row
        .map((cell, columnIndex) => {
          const value = cell === null ? "" : String(cell);
          const ref = `${String.fromCharCode(65 + columnIndex)}${rowIndex + 1}`;
          return `<c r="${ref}" t="inlineStr"><is><t>${escapeHtml(value)}</t></is></c>`;
        })
        .join("");
      return `<row r="${rowIndex + 1}">${cells}</row>`;
    })
    .join("");
  return `<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main">
<sheetData>${xmlRows}</sheetData>
</worksheet>`;
};

export function toXlsx(table: ExportTable): Uint8Array {
  const files = new Map<string, string>([
    [
      "[Content_Types].xml",
      `<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">
<Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>
<Default Extension="xml" ContentType="application/xml"/>
<Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/>
<Override PartName="/xl/worksheets/sheet1.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/>
</Types>`,
    ],
    [
      "_rels/.rels",
      `<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
<Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="xl/workbook.xml"/>
</Relationships>`,
    ],
    [
      "xl/workbook.xml",
      `<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships">
<sheets><sheet name="Export" sheetId="1" r:id="rId1"/></sheets>
</workbook>`,
    ],
    [
      "xl/_rels/workbook.xml.rels",
      `<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
<Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet1.xml"/>
</Relationships>`,
    ],
    ["xl/worksheets/sheet1.xml", sheetXml(table)],
  ]);
  const localParts: Uint8Array[] = [];
  const centralParts: Uint8Array[] = [];
  const { time, date } = dosDateTime();
  let offset = 0;
  for (const [path, content] of files) {
    const name = textEncoder.encode(path);
    const data = textEncoder.encode(content);
    const crc = crc32(data);
    const local = concatBytes([
      u32(0x04034b50),
      u16(20),
      u16(0),
      u16(0),
      u16(time),
      u16(date),
      u32(crc),
      u32(data.length),
      u32(data.length),
      u16(name.length),
      u16(0),
      name,
      data,
    ]);
    localParts.push(local);
    centralParts.push(
      concatBytes([
        u32(0x02014b50),
        u16(20),
        u16(20),
        u16(0),
        u16(0),
        u16(time),
        u16(date),
        u32(crc),
        u32(data.length),
        u32(data.length),
        u16(name.length),
        u16(0),
        u16(0),
        u16(0),
        u16(0),
        u32(0),
        u32(offset),
        name,
      ]),
    );
    offset += local.length;
  }
  const central = concatBytes(centralParts);
  const end = concatBytes([
    u32(0x06054b50),
    u16(0),
    u16(0),
    u16(files.size),
    u16(files.size),
    u32(central.length),
    u32(offset),
    u16(0),
  ]);
  return concatBytes([...localParts, central, end]);
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
    // XLSX is handled async via prepareExportBlob; this path is a
    // fallback that produces a basic file using the hand-rolled writer.
    return {
      filename,
      mime: "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
      content: toXlsx(table),
    };
  }
  const text =
    options.format === "csv"
      ? toCsv(table, { nullAs })
      : options.format === "json"
        ? toJson(table, { pretty: true })
        : options.format === "sql"
          ? toSqlInserts(table, {
              tableName: options.sqlTableName ?? options.filenameBase,
            })
          : options.format === "html"
            ? toHtml(table, nullAs)
            : options.format === "markdown"
              ? toMarkdown(table, nullAs)
              : toTxt(table, nullAs);
  return {
    filename,
    mime: mimeForFormat(options.format, options.encoding),
    content: encodeText(text, options.encoding),
  };
}

export async function prepareExportBlob(
  table: ExportTable,
  options: ExportOptions,
): Promise<{ filename: string; blob: Blob }> {
  // For XLSX, prefer the Rust backend which produces a proper Excel file
  // with numeric cells, correct shared-strings, etc.
  if (options.format === "xlsx" && isTauri()) {
    const nullAs = options.nullAs ?? "";
    const columns = table.columns;
    const rows = table.rows.map((row) =>
      row.map((cell) => (cell === null ? nullAs : cell)),
    );
    const base64 = await tauriInvoke<string>("export_xlsx", {
      payload: { columns, rows, sheetName: options.filenameBase || "Export" },
    });
    const binary = atob(base64);
    const bytes = new Uint8Array(binary.length);
    for (let i = 0; i < binary.length; i++) {
      bytes[i] = binary.charCodeAt(i);
    }
    const ext = "xlsx";
    const filename =
      options.compression === "gzip"
        ? `${options.filenameBase}.${ext}.gz`
        : `${options.filenameBase}.${ext}`;
    const blob = new Blob([bytes], {
      type: "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
    });
    if (options.compression === "gzip") {
      if (typeof CompressionStream === "undefined") {
        throw new Error("Gzip compression is not supported in this webview.");
      }
      const compressed = await new Response(
        blob.stream().pipeThrough(new CompressionStream("gzip")),
      ).blob();
      return {
        filename,
        blob: new Blob([compressed], { type: "application/gzip" }),
      };
    }
    return { filename, blob };
  }

  const prepared = prepareExport(table, options);
  const blob = new Blob([prepared.content], { type: prepared.mime });
  if (options.compression !== "gzip") {
    return { filename: prepared.filename, blob };
  }
  if (typeof CompressionStream === "undefined") {
    throw new Error("Gzip compression is not supported in this webview.");
  }
  const compressed = await new Response(
    blob.stream().pipeThrough(new CompressionStream("gzip")),
  ).blob();
  return {
    filename: prepared.filename,
    blob: new Blob([compressed], { type: "application/gzip" }),
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
