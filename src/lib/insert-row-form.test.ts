import { describe, expect, it } from "vitest";

import {
  buildInsertValuesPayload,
  type InsertRowColumn,
  type InsertRowFormState,
  initialFormState,
} from "@/lib/insert-row-form";

const columns: InsertRowColumn[] = [
  {
    name: "id",
    dataType: "integer",
    nullable: false,
    defaultValue: "nextval('users_id_seq'::regclass)",
  },
  {
    name: "email",
    dataType: "text",
    nullable: false,
    defaultValue: null,
  },
  {
    name: "name",
    dataType: "text",
    nullable: true,
    defaultValue: null,
  },
];

describe("initialFormState", () => {
  it("starts non-null defaultable columns in `default` mode", () => {
    const state = initialFormState(columns);
    // The id column has a SERIAL-style default — start in default mode so
    // users don't have to think about it.
    expect(state.id?.mode).toBe("default");
  });

  it("starts nullable no-default columns in `null` mode", () => {
    const state = initialFormState(columns);
    expect(state.name?.mode).toBe("null");
  });

  it("starts non-null no-default columns in `value` mode with empty input", () => {
    const state = initialFormState(columns);
    expect(state.email?.mode).toBe("value");
    expect(state.email?.value).toBe("");
  });
});

describe("buildInsertValuesPayload", () => {
  it("includes value-mode columns with their entered text", () => {
    const state: InsertRowFormState = {
      id: { mode: "default", value: "" },
      email: { mode: "value", value: "ada@example.com" },
      name: { mode: "value", value: "Ada" },
    };
    const payload = buildInsertValuesPayload(state, columns);
    expect(payload).toEqual([
      { column: "email", value: "ada@example.com" },
      { column: "name", value: "Ada" },
    ]);
  });

  it("emits null for null-mode columns", () => {
    const state: InsertRowFormState = {
      id: { mode: "default", value: "" },
      email: { mode: "value", value: "ada@example.com" },
      name: { mode: "null", value: "anything" },
    };
    const payload = buildInsertValuesPayload(state, columns);
    expect(payload).toEqual([
      { column: "email", value: "ada@example.com" },
      { column: "name", value: null },
    ]);
  });

  it("omits default-mode columns from the payload entirely", () => {
    const state: InsertRowFormState = {
      id: { mode: "default", value: "" },
      email: { mode: "value", value: "ada@example.com" },
      name: { mode: "default", value: "" },
    };
    const payload = buildInsertValuesPayload(state, columns);
    expect(payload.find((entry) => entry.column === "id")).toBeUndefined();
    expect(payload.find((entry) => entry.column === "name")).toBeUndefined();
    expect(payload).toEqual([{ column: "email", value: "ada@example.com" }]);
  });

  it("preserves column ordinal position in the payload", () => {
    const state: InsertRowFormState = {
      id: { mode: "value", value: "42" },
      email: { mode: "value", value: "x@y.z" },
      name: { mode: "value", value: "X" },
    };
    const payload = buildInsertValuesPayload(state, columns);
    expect(payload.map((entry) => entry.column)).toEqual([
      "id",
      "email",
      "name",
    ]);
  });
});
