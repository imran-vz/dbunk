/**
 * Pure helpers for generating mock INSERT statements from a real
 * Postgres column list. Used by the Compare tab's "Mock data
 * generator" to emit SQL that matches the target table's actual
 * schema instead of a hard-coded `id, name, created_at` template.
 *
 * Identity / primary-key columns are emitted as `DEFAULT` so serial
 * sequences and auto-generated keys don't collide on insert.
 */

import type { ColumnInfo } from "@/lib/store";

export type MockRowOptions = {
  rowCount?: number;
  seed?: number;
};

const DEFAULT_ROWS = 5;

export function generateMockInsertSql(
  qualifiedTable: string,
  columns: ColumnInfo[],
  options: MockRowOptions = {},
): string {
  if (!qualifiedTable.trim()) {
    return "-- target table is required";
  }
  if (columns.length === 0) {
    return `-- no column metadata available for ${qualifiedTable}`;
  }
  const rowCount = Math.max(1, options.rowCount ?? DEFAULT_ROWS);
  const random = mulberry32(options.seed ?? 1);
  const columnList = columns
    .map((column) => quoteIdent(column.name))
    .join(", ");
  const lines: string[] = [];
  for (let row = 0; row < rowCount; row += 1) {
    const values = columns
      .map((column) => mockValue(column, row + 1, random))
      .join(", ");
    lines.push(
      `INSERT INTO ${qualifiedTable} (${columnList}) VALUES (${values});`,
    );
  }
  return lines.join("\n");
}

function mockValue(
  column: ColumnInfo,
  rowIndex: number,
  random: () => number,
): string {
  if (column.isPrimaryKey) return "DEFAULT";
  if (column.defaultValue !== null && column.defaultValue !== "") {
    return "DEFAULT";
  }
  const category = categoryFor(column.dataType);
  switch (category) {
    case "boolean":
      return random() < 0.5 ? "FALSE" : "TRUE";
    case "integer":
      return String(Math.floor(random() * 1000));
    case "numeric":
      return (random() * 1000).toFixed(2);
    case "uuid":
      return "gen_random_uuid()";
    case "date":
      return "CURRENT_DATE";
    case "time":
      return "CURRENT_TIME";
    case "timestamp":
      return "now()";
    case "json":
      return `'${escapeLiteral(`{"sample":${rowIndex}}`)}'::jsonb`;
    case "array":
      return `ARRAY['sample-${rowIndex}']`;
    case "bytea":
      return "'\\x00'::bytea";
    case "interval":
      return "INTERVAL '1 day'";
    case "inet":
      return "'192.0.2.1'::inet";
    case "text":
      return `'${escapeLiteral(`Sample ${rowIndex}`)}'`;
    default:
      return column.nullable ? "NULL" : "DEFAULT";
  }
}

type ValueCategory =
  | "boolean"
  | "integer"
  | "numeric"
  | "uuid"
  | "date"
  | "time"
  | "timestamp"
  | "json"
  | "array"
  | "bytea"
  | "interval"
  | "inet"
  | "text"
  | "unknown";

function categoryFor(dataType: string): ValueCategory {
  const normalized = dataType.toLowerCase().trim();
  if (normalized.endsWith("[]")) return "array";
  if (/^bool(ean)?\b/.test(normalized)) return "boolean";
  if (normalized.startsWith("interval")) return "interval";
  if (normalized.startsWith("timestamp")) return "timestamp";
  if (
    /^(smallint|integer|int[248]?|bigint|serial|smallserial|bigserial)\b/.test(
      normalized,
    )
  ) {
    return "integer";
  }
  if (/^(numeric|decimal|real|double|float)\b/.test(normalized)) {
    return "numeric";
  }
  if (normalized.startsWith("uuid")) return "uuid";
  if (normalized.startsWith("date")) return "date";
  if (normalized.startsWith("time")) return "time";
  if (normalized === "json" || normalized === "jsonb") return "json";
  if (normalized.startsWith("bytea")) return "bytea";
  if (normalized === "inet" || normalized === "cidr") return "inet";
  if (
    /^(text|varchar|character varying|char|character|citext|name)\b/.test(
      normalized,
    )
  ) {
    return "text";
  }
  return "unknown";
}

function quoteIdent(name: string): string {
  if (/^[a-z_][a-z0-9_]*$/.test(name)) return name;
  return `"${name.replaceAll('"', '""')}"`;
}

function escapeLiteral(value: string): string {
  return value.replaceAll("'", "''");
}

// Deterministic PRNG so tests get stable output and successive
// generations in one click are visibly different across rows.
function mulberry32(seed: number): () => number {
  let state = seed >>> 0;
  return () => {
    state = (state + 0x6d2b79f5) >>> 0;
    let t = state;
    t = Math.imul(t ^ (t >>> 15), t | 1);
    t ^= t + Math.imul(t ^ (t >>> 7), t | 61);
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}

export function splitQualifiedTable(
  input: string,
  defaultSchema: string,
): { schema: string; table: string } | null {
  const trimmed = input.trim();
  if (!trimmed) return null;
  const parts = trimmed.split(".").map((part) => part.replaceAll('"', ""));
  if (parts.length === 1) {
    return { schema: defaultSchema, table: parts[0] };
  }
  if (parts.length === 2) {
    return { schema: parts[0] || defaultSchema, table: parts[1] };
  }
  return null;
}
