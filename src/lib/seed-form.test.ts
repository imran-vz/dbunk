import { describe, expect, it } from "vitest";

import {
  buildSeedColumnsPayload,
  initialSeedColumnState,
  initialSeedFormState,
  SEED_GENERATOR_OPTIONS,
} from "@/lib/seed-form";
import type { ColumnInfo } from "@/lib/store";

function column(name: string, overrides: Partial<ColumnInfo> = {}): ColumnInfo {
  return {
    name,
    dataType: "text",
    nullable: true,
    defaultValue: null,
    isPrimaryKey: false,
    ordinalPosition: 1,
    ...overrides,
  };
}

describe("buildSeedColumnsPayload", () => {
  it("emits nothing for untouched columns", () => {
    const columns = [column("a"), column("b")];
    const form = initialSeedFormState(columns);
    expect(buildSeedColumnsPayload(form, columns)).toEqual([]);
  });

  it("emits skip, constant, and generator overrides", () => {
    const columns = [column("a"), column("b"), column("c")];
    const form = initialSeedFormState(columns);
    form.a = { ...initialSeedColumnState(), mode: "skip" };
    form.b = { ...initialSeedColumnState(), mode: "constant", constant: "x" };
    form.c = { ...initialSeedColumnState(), generator: "email" };
    expect(buildSeedColumnsPayload(form, columns)).toEqual([
      { column: "a", skip: true },
      { column: "b", constant: "x" },
      { column: "c", generator: "email" },
    ]);
  });

  it("splits and trims comma-separated value lists", () => {
    const columns = [column("status")];
    const form = initialSeedFormState(columns);
    form.status = {
      ...initialSeedColumnState(),
      mode: "values",
      valuesText: " active, inactive ,, pending ",
    };
    expect(buildSeedColumnsPayload(form, columns)).toEqual([
      { column: "status", values: ["active", "inactive", "pending"] },
    ]);
  });

  it("converts NULL percent to a clamped 0-1 rate on nullable columns only", () => {
    const columns = [
      column("a", { nullable: true }),
      column("b", { nullable: false }),
    ];
    const form = initialSeedFormState(columns);
    form.a = { ...initialSeedColumnState(), nullPercent: "250" };
    form.b = { ...initialSeedColumnState(), nullPercent: "50" };
    expect(buildSeedColumnsPayload(form, columns)).toEqual([
      { column: "a", nullRate: 1 },
    ]);
  });

  it("ignores empty value lists and blank null percent", () => {
    const columns = [column("a")];
    const form = initialSeedFormState(columns);
    form.a = { ...initialSeedColumnState(), mode: "values", valuesText: " , " };
    expect(buildSeedColumnsPayload(form, columns)).toEqual([]);
  });
});

describe("SEED_GENERATOR_OPTIONS", () => {
  it("has unique ids", () => {
    const ids = SEED_GENERATOR_OPTIONS.map((o) => o.id);
    expect(new Set(ids).size).toBe(ids.length);
  });
});
