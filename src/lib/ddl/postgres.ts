/**
 * PostgreSQL DDL generation for table structure changes.
 *
 * Legacy frontend fallback for ClickHouse and the not-yet-supported
 * MySQL/SQLite structure paths. PostgreSQL now previews and applies typed
 * operations through the backend object-DDL workflow.
 *
 * This module is pure: it takes a structured description of column-level
 * changes and produces an executable PostgreSQL DDL string. It is responsible
 * only for SQL formatting — it does NOT validate that the changes are
 * semantically safe against a live schema.
 */

import type { PgObjectOp } from "@/lib/store/types";

import {
  createIdentQuoter,
  formatDefault,
  renderAddColumn,
  renderDropColumn,
  renderRenameColumn,
} from "./shared";

export type NewColumn = {
  name: string;
  dataType: string;
  nullable: boolean;
  defaultValue: string | null;
};

export type ColumnChangeKind =
  | { kind: "add"; column: NewColumn }
  | { kind: "drop"; columnName: string }
  | { kind: "rename"; columnName: string; newName: string }
  | { kind: "set_type"; columnName: string; newType: string }
  | { kind: "set_nullable"; columnName: string; nullable: boolean }
  | { kind: "set_default"; columnName: string; default: string | null };

export type PendingChange = {
  id: string;
  schema: string;
  table: string;
  change: StructureChange;
};

export type StructureChange =
  | { kind: "column"; change: ColumnChangeKind }
  | { kind: "pg-op"; op: PgObjectOp };

const { quoteIdent, qualifiedTable } = createIdentQuoter('"');

const renderColumnDefinition = (column: NewColumn): string => {
  const parts = [quoteIdent(column.name), column.dataType];
  if (!column.nullable) {
    parts.push("NOT NULL");
  }
  if (column.defaultValue !== null && column.defaultValue !== "") {
    parts.push(`DEFAULT ${formatDefault(column.defaultValue)}`);
  }
  return parts.join(" ");
};

const renderChange = (
  schema: string,
  table: string,
  change: ColumnChangeKind,
): string => {
  const prefix = `ALTER TABLE ${qualifiedTable(schema, table)}`;
  switch (change.kind) {
    case "add":
      return renderAddColumn(prefix, renderColumnDefinition(change.column));
    case "drop":
      return renderDropColumn(prefix, quoteIdent(change.columnName));
    case "rename":
      return renderRenameColumn(
        prefix,
        quoteIdent(change.columnName),
        quoteIdent(change.newName),
      );
    case "set_type":
      return `${prefix} ALTER COLUMN ${quoteIdent(change.columnName)} TYPE ${change.newType};`;
    case "set_nullable":
      return `${prefix} ALTER COLUMN ${quoteIdent(change.columnName)} ${
        change.nullable ? "DROP NOT NULL" : "SET NOT NULL"
      };`;
    case "set_default":
      if (change.default === null) {
        return `${prefix} ALTER COLUMN ${quoteIdent(change.columnName)} DROP DEFAULT;`;
      }
      return `${prefix} ALTER COLUMN ${quoteIdent(change.columnName)} SET DEFAULT ${formatDefault(
        change.default,
      )};`;
  }
};

export function generatePostgresDdl(
  schema: string,
  table: string,
  changes: ColumnChangeKind[],
): string {
  if (changes.length === 0) {
    return "";
  }
  return changes
    .map((change) => renderChange(schema, table, change))
    .join("\n");
}

/**
 * Partition changes into destructive (require explicit confirmation) and
 * non-destructive buckets. Destructive in v1:
 *   - drop column
 *   - set_type (cast may fail or lose precision)
 *   - set_nullable=false (existing nulls would block the migration)
 *
 * Relaxing nullability (set_nullable=true), adding columns, renaming, and
 * default-value changes are non-destructive.
 */
export function classifyDestructive(changes: ColumnChangeKind[]) {
  const destructive: ColumnChangeKind[] = [];
  const nonDestructive: ColumnChangeKind[] = [];
  for (const change of changes) {
    if (
      change.kind === "drop" ||
      change.kind === "set_type" ||
      (change.kind === "set_nullable" && change.nullable === false)
    ) {
      destructive.push(change);
    } else {
      nonDestructive.push(change);
    }
  }
  return { destructive, nonDestructive };
}
