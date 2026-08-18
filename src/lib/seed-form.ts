// Seed Spec form state and payload building for the Table Seeding
// panel (ADR-0020). Pure helpers — no Tauri, no store.

import type { ColumnInfo } from "@/lib/store";

/** Mirrors the backend `SeedTableResult`. */
export interface SeedTableResult {
  rowsInserted: number;
  seedUsed: number;
  runtimeMs: number;
}

export interface SeedTableProgress {
  operationId: string;
  rowsCompleted: number;
  totalRows: number;
}

/** One per-column entry of the `seed_table` payload. */
export interface SeedColumnSpecPayload {
  column: string;
  skip?: boolean;
  constant?: string;
  values?: string[];
  generator?: string;
  nullRate?: number;
}

/**
 * Generator ids the backend's `parse_generator_id` accepts, with
 * display labels. Keep in sync with `src-tauri/src/seed.rs`.
 */
export const SEED_GENERATOR_OPTIONS: ReadonlyArray<{
  id: string;
  label: string;
}> = [
  { id: "email", label: "Email" },
  { id: "firstName", label: "First name" },
  { id: "lastName", label: "Last name" },
  { id: "fullName", label: "Full name" },
  { id: "userName", label: "Username" },
  { id: "company", label: "Company" },
  { id: "url", label: "URL" },
  { id: "phone", label: "Phone" },
  { id: "city", label: "City" },
  { id: "country", label: "Country" },
  { id: "streetAddress", label: "Street address" },
  { id: "word", label: "Word" },
  { id: "sentence", label: "Sentence" },
  { id: "boolean", label: "Boolean" },
  { id: "tinyInt", label: "Tiny integer" },
  { id: "smallInt", label: "Small integer" },
  { id: "integer", label: "Integer" },
  { id: "bigInt", label: "Big integer" },
  { id: "float", label: "Float" },
  { id: "decimal", label: "Decimal" },
  { id: "price", label: "Price" },
  { id: "uuid", label: "UUID" },
  { id: "date", label: "Date" },
  { id: "time", label: "Time" },
  { id: "timestamp", label: "Timestamp" },
  { id: "json", label: "JSON" },
];

/** `auto` lets the backend pick by type + name inference. */
export type SeedColumnMode = "auto" | "skip" | "constant" | "values";

export interface SeedColumnFormState {
  mode: SeedColumnMode;
  /** Generator id override; empty string means auto. */
  generator: string;
  constant: string;
  /** Comma-separated value list for `values` mode. */
  valuesText: string;
  /** NULL percentage 0–100 as entered; empty means backend default. */
  nullPercent: string;
}

export type SeedFormState = Record<string, SeedColumnFormState>;

export const DEFAULT_SEED_ROW_COUNT = 100;

export function initialSeedColumnState(): SeedColumnFormState {
  return {
    mode: "auto",
    generator: "",
    constant: "",
    valuesText: "",
    nullPercent: "",
  };
}

export function initialSeedFormState(columns: ColumnInfo[]) {
  const state: SeedFormState = {};
  for (const column of columns) {
    state[column.name] = initialSeedColumnState();
  }
  return state;
}

function parseNullRate(nullPercent: string): number | undefined {
  if (nullPercent.trim() === "") return undefined;
  const parsed = Number(nullPercent);
  if (!Number.isFinite(parsed)) return undefined;
  return Math.min(Math.max(parsed, 0), 100) / 100;
}

/**
 * Build the `columns` payload, emitting entries only for columns the
 * user actually changed — untouched columns stay fully backend-driven.
 */
export function buildSeedColumnsPayload(
  form: SeedFormState,
  columns: ColumnInfo[],
): SeedColumnSpecPayload[] {
  const payload: SeedColumnSpecPayload[] = [];
  for (const column of columns) {
    const field = form[column.name];
    if (!field) continue;
    const entry: SeedColumnSpecPayload = { column: column.name };
    let changed = false;

    if (field.mode === "skip") {
      entry.skip = true;
      changed = true;
    } else if (field.mode === "constant") {
      entry.constant = field.constant;
      changed = true;
    } else if (field.mode === "values") {
      const values = field.valuesText
        .split(",")
        .map((value) => value.trim())
        .filter((value) => value.length > 0);
      if (values.length > 0) {
        entry.values = values;
        changed = true;
      }
    } else if (field.generator !== "") {
      entry.generator = field.generator;
      changed = true;
    }

    if (field.mode !== "skip" && column.nullable) {
      const nullRate = parseNullRate(field.nullPercent);
      if (nullRate !== undefined) {
        entry.nullRate = nullRate;
        changed = true;
      }
    }

    if (changed) payload.push(entry);
  }
  return payload;
}
