import type { ColumnInfo } from "@/lib/store";

/**
 * Per-column form mode for the "Add row" dialog.
 *
 * - `value`: emit the current `value` string as a positional parameter.
 * - `null`: emit explicit SQL NULL (regardless of `value`).
 * - `default`: omit the column from the INSERT entirely so Postgres assigns
 *   its declared DEFAULT (or auto-generates a SERIAL/identity value).
 */
export type InsertRowFieldMode = "value" | "null" | "default";

export type InsertRowFieldState = {
  mode: InsertRowFieldMode;
  value: string;
};

export type InsertRowFormState = Record<string, InsertRowFieldState>;

/**
 * Subset of `ColumnInfo` the insert form actually needs. Keeping it narrow
 * makes the helper unit-testable without constructing full structures.
 */
export type InsertRowColumn = Pick<
  ColumnInfo,
  "name" | "dataType" | "nullable" | "defaultValue"
>;

/**
 * Pick the most user-friendly default mode for a column when the form
 * first opens:
 *
 *   - Has a non-null default (SERIAL, identity, NOW(), …) → `default`
 *     so the user can ignore it unless they want to override.
 *   - Otherwise nullable → `null` since that's the only valid empty value.
 *   - Otherwise → `value` with an empty input that the user must fill.
 */
export function initialFieldState(
  column: InsertRowColumn,
): InsertRowFieldState {
  if (column.defaultValue !== null && column.defaultValue !== undefined) {
    return { mode: "default", value: "" };
  }
  if (column.nullable) {
    return { mode: "null", value: "" };
  }
  return { mode: "value", value: "" };
}

export function initialFormState(
  columns: InsertRowColumn[],
): InsertRowFormState {
  const state: InsertRowFormState = {};
  for (const column of columns) {
    state[column.name] = initialFieldState(column);
  }
  return state;
}

export type InsertRowPayloadEntry = {
  column: string;
  value: string | null;
};

/**
 * Translate the form state into the wire payload for `insert_row`.
 *
 * Iterates `columns` (not the form-state keys) so the resulting payload
 * preserves the table's column order — handy when reading the SQL the
 * backend builds against the database.
 */
export function buildInsertValuesPayload(
  state: InsertRowFormState,
  columns: InsertRowColumn[],
): InsertRowPayloadEntry[] {
  const payload: InsertRowPayloadEntry[] = [];
  for (const column of columns) {
    const field = state[column.name];
    if (!field) {
      continue;
    }
    if (field.mode === "default") {
      continue;
    }
    if (field.mode === "null") {
      payload.push({ column: column.name, value: null });
      continue;
    }
    payload.push({ column: column.name, value: field.value });
  }
  return payload;
}
