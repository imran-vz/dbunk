/**
 * Engine-aware DDL dispatch.
 *
 * The structure-edit UI builds a list of `ColumnChangeKind` entries
 * regardless of engine; this module renders them as engine-specific
 * SQL. Adding a new engine = a new builder file + a new branch here.
 */

import type { ColumnInfo, DatabaseEngine } from "@/lib/store";

import { generateClickHouseDdl } from "./clickhouse";
import { type ColumnChangeKind, generatePostgresDdl } from "./postgres";

export type {
  ColumnChangeKind,
  NewColumn,
  PendingChange,
  StructureChange,
} from "./postgres";
export { classifyDestructive } from "./postgres";

/** Build a `name -> dataType` map from the loaded column list, used by
 * the CH builder to resolve nullability changes. PG ignores it. */
const columnTypeMap = (
  columns: ColumnInfo[] | undefined,
): Map<string, string> => {
  const map = new Map<string, string>();
  for (const column of columns ?? []) {
    map.set(column.name, column.dataType);
  }
  return map;
};

export function generateDdlForEngine(
  engine: DatabaseEngine,
  schema: string,
  table: string,
  changes: ColumnChangeKind[],
  columns?: ColumnInfo[],
): string {
  switch (engine) {
    case "ClickHouse":
      return generateClickHouseDdl(
        schema,
        table,
        changes,
        columnTypeMap(columns),
      );
    case "PostgreSQL":
    case "MySQL":
    case "SQLite":
      // Non-PG engines fall back to the PG builder for now — the
      // backend rejects them at the dispatcher with an engine-name
      // error, so the rendered SQL never actually runs.
      return generatePostgresDdl(schema, table, changes);
    case "Redis":
      // Redis has no DDL; the structure view never renders for Redis
      // connections (ADR-0008 forks the workspace shell), so this
      // branch is an invariant assertion rather than a real fallback.
      throw new Error(
        "generateDdlForEngine() called with Redis — relational shell only",
      );
  }
}
