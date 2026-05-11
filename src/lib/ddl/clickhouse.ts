/**
 * ClickHouse DDL generation for table structure changes.
 *
 * Mirrors `postgres.ts` but emits CH-flavoured `ALTER TABLE` syntax:
 *
 * - Identifiers are backtick-quoted (CH supports both backticks and
 *   double quotes, but backticks match the rest of the CH path in dbunk
 *   — see `qualified_table_name` in `lib.rs`).
 * - Nullability is part of the *type* in CH (`Nullable(T)`), not a
 *   column-level constraint, so `set_nullable` is emitted as `MODIFY
 *   COLUMN name <Nullable-wrapped-type>`. We need the column's current
 *   type for that — passed via the optional `columnTypes` map on
 *   `generateClickHouseDdl`. When no type is known, the change is
 *   skipped with a comment so the user can fix it manually.
 * - `set_default` translates to `MODIFY COLUMN … DEFAULT …` (set) or
 *   `MODIFY COLUMN … REMOVE DEFAULT` (drop).
 *
 * Like the PG builder, this module is pure SQL formatting — it does
 * NOT validate that the changes are semantically safe against a live
 * schema.
 */

import type {
  ColumnChangeKind,
  NewColumn,
  PendingChange,
} from "@/lib/ddl/postgres";

export type { ColumnChangeKind, NewColumn, PendingChange };

const quoteIdent = (identifier: string): string =>
  `\`${identifier.replace(/`/g, "``")}\``;

const qualifiedTable = (schema: string, table: string): string =>
  `${quoteIdent(schema)}.${quoteIdent(table)}`;

const formatDefault = (raw: string): string => {
  if (/^-?[0-9]+(\.[0-9]+)?$/.test(raw)) {
    return raw;
  }
  if (raw.endsWith("()")) {
    return raw;
  }
  return `'${raw.replace(/'/g, "''")}'`;
};

const wrapNullable = (type: string): string => {
  if (type.startsWith("Nullable(") && type.endsWith(")")) {
    return type;
  }
  return `Nullable(${type})`;
};

const unwrapNullable = (type: string): string => {
  if (type.startsWith("Nullable(") && type.endsWith(")")) {
    return type.slice("Nullable(".length, -1);
  }
  return type;
};

const renderColumnDefinition = (column: NewColumn): string => {
  // CH treats nullability as part of the type. The PG builder lets the
  // user write `text` + `nullable: false`; for CH we map that to the
  // bare type. `nullable: true` wraps the user-supplied type in
  // `Nullable(...)` unless they already did.
  const renderedType = column.nullable
    ? wrapNullable(column.dataType)
    : unwrapNullable(column.dataType);
  const parts = [quoteIdent(column.name), renderedType];
  if (column.defaultValue !== null && column.defaultValue !== "") {
    parts.push(`DEFAULT ${formatDefault(column.defaultValue)}`);
  }
  return parts.join(" ");
};

const renderChange = (
  schema: string,
  table: string,
  change: ColumnChangeKind,
  columnTypes: Map<string, string>,
): string => {
  const prefix = `ALTER TABLE ${qualifiedTable(schema, table)}`;
  switch (change.kind) {
    case "add":
      return `${prefix} ADD COLUMN ${renderColumnDefinition(change.column)};`;
    case "drop":
      return `${prefix} DROP COLUMN ${quoteIdent(change.columnName)};`;
    case "rename":
      return `${prefix} RENAME COLUMN ${quoteIdent(change.columnName)} TO ${quoteIdent(
        change.newName,
      )};`;
    case "set_type":
      // CH MODIFY COLUMN preserves nullability if the new type is also
      // wrapped; we forward the user's literal input so they can write
      // `Nullable(Int64)` or `Int64` explicitly.
      return `${prefix} MODIFY COLUMN ${quoteIdent(change.columnName)} ${change.newType};`;
    case "set_nullable": {
      // Need the current type to wrap/unwrap. If we don't have it, emit
      // a comment so the operator can correct it manually rather than
      // running a broken MODIFY COLUMN.
      const currentType = columnTypes.get(change.columnName);
      if (!currentType) {
        return `-- ${prefix} MODIFY COLUMN ${quoteIdent(change.columnName)} <type> -- type not loaded; cannot ${change.nullable ? "wrap with" : "remove"} Nullable()`;
      }
      const newType = change.nullable
        ? wrapNullable(currentType)
        : unwrapNullable(currentType);
      return `${prefix} MODIFY COLUMN ${quoteIdent(change.columnName)} ${newType};`;
    }
    case "set_default":
      if (change.default === null) {
        return `${prefix} MODIFY COLUMN ${quoteIdent(change.columnName)} REMOVE DEFAULT;`;
      }
      return `${prefix} MODIFY COLUMN ${quoteIdent(change.columnName)} DEFAULT ${formatDefault(
        change.default,
      )};`;
  }
};

export function generateClickHouseDdl(
  schema: string,
  table: string,
  changes: ColumnChangeKind[],
  columnTypes: Map<string, string> = new Map(),
): string {
  if (changes.length === 0) {
    return "";
  }
  return changes
    .map((change) => renderChange(schema, table, change, columnTypes))
    .join("\n");
}
