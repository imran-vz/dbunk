import type { TableStructure } from "@/lib/store";

/**
 * Identity used to address a single row for safe edits.
 *
 * `null` means the table has no usable identity — callers should treat the
 * grid as read-only because we cannot generate a precise WHERE clause.
 */
export type RowIdentity = { columns: string[] } | null;

/**
 * Pick the safest row identity from table metadata, in order of preference:
 *
 *   1. The declared primary key (`structure.primaryKey`).
 *   2. The smallest unique non-primary index whose every column is NOT NULL
 *      and is known in the column list. NULL participation breaks identity
 *      under SQL semantics (NULL != NULL), so we exclude any index that
 *      includes a nullable column.
 *
 * Returns `null` when neither source yields a usable identity.
 */
export function pickRowIdentity(
  structure: TableStructure | undefined,
): RowIdentity {
  if (!structure) {
    return null;
  }

  if (structure.primaryKey && structure.primaryKey.length > 0) {
    return { columns: [...structure.primaryKey] };
  }

  const nonNullColumns = new Set(
    structure.columns.filter((col) => !col.nullable).map((col) => col.name),
  );

  const candidates = structure.indexes
    .filter(
      (index) =>
        index.isUnique &&
        !index.isPrimary &&
        index.columns.length > 0 &&
        index.columns.every((col) => nonNullColumns.has(col)),
    )
    // Prefer the narrowest index — fewer columns = simpler WHERE clause and
    // a single-column unique key is the closest substitute for a PK.
    .sort((a, b) => a.columns.length - b.columns.length);

  const winner = candidates[0];
  if (!winner) {
    return null;
  }

  return { columns: [...winner.columns] };
}
