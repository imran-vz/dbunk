// @vitest-environment jsdom
import { cleanup, render, screen, within } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

import {
  SCHEMA_COMPARE_NORMALIZATION_VERSION,
  SCHEMA_COMPARE_SCOPE,
  type SchemaCompareFieldSummary,
  type SchemaCompareObjectSummary,
} from "@/lib/pg-schema-compare/protocol";
import type { SchemaCompareMetadataDetail } from "@/lib/pg-schema-compare/reader";

import { CoverageSummary } from "./coverage";
import { FieldMatrix } from "./field-matrix";
import { ObjectList } from "./object-list";

afterEach(cleanup);

const identity = { jobId: "job", resultId: "result" };
const capture = (connectionId: string, version: string) => ({
  endpoint: { connectionId, schema: "public" },
  serverVersion: version,
  serverVersionNum: 160015,
  capturedAt: "2026-09-14T10:42:03Z",
});
const detail: SchemaCompareMetadataDetail = {
  metadata: {
    identity,
    source: capture("a", "16.15"),
    target: capture("b", "16.14"),
    consistency: "independentTransactions",
    coverage: {
      scope: SCHEMA_COMPARE_SCOPE,
      normalizationVersion: SCHEMA_COMPARE_NORMALIZATION_VERSION,
      excludedRelations: 2,
      incomparableFields: 1,
      excludedCategories: ["routines", "policies"],
    },
  },
  kind: "changed",
  objectCount: 3,
  sourceExcludedCounts: [
    { category: "routines", count: 0, complete: false },
    { category: "policies", count: 4, complete: false },
    { category: "sequences", count: 0, complete: true },
  ],
  targetExcludedCounts: [],
};

const orders: SchemaCompareObjectSummary = {
  kind: "changed",
  fieldCount: 4,
  changedFields: 1,
  incomparableFields: 1,
  source: { kind: "table", name: "orders" },
  target: { kind: "table", name: "orders" },
};
const audit: SchemaCompareObjectSummary = {
  kind: "sourceOnly",
  fieldCount: 1,
  changedFields: 0,
  incomparableFields: 0,
  source: { kind: "table", name: "audit_log" },
};
const events: SchemaCompareObjectSummary = {
  kind: "notComparable",
  reason: "excludedCounterpart",
  fieldCount: 0,
  changedFields: 0,
  incomparableFields: 0,
  source: { kind: "table", name: "events" },
  target: { kind: "partitionedTable", name: "events" },
};
const customers: SchemaCompareObjectSummary = {
  kind: "equal",
  fieldCount: 2,
  changedFields: 0,
  incomparableFields: 0,
  source: { kind: "table", name: "customers" },
  target: { kind: "table", name: "customers" },
};
const ref = <Side extends "source" | "target">(
  side: Side,
  valueId: number,
  rawBytes = 4,
) => ({ side, valueId, rawBytes, valueKind: "text" as const });
const fields: SchemaCompareFieldSummary[] = [
  {
    kind: "changed",
    path: { kind: "column", name: "total", field: "typeModifier" },
    source: ref("source", 0),
    target: ref("target", 1),
  },
  {
    kind: "notComparable",
    reason: "renderingVersionDifference",
    path: { kind: "column", name: "created_at", field: "default" },
    source: ref("source", 2),
    target: ref("target", 3),
  },
  {
    kind: "sourceOnly",
    path: {
      kind: "indexKey",
      name: "orders_pkey",
      owner: "orders_pkey",
      position: 0,
      field: "column",
    },
    source: ref("source", 4),
  },
  {
    kind: "equal",
    path: { kind: "column", name: "id", field: "nullable" },
    source: ref("source", 5),
    target: ref("target", 6),
  },
];
const noop = vi.fn();
/** Body rows of the field matrix, after the header row. */
function bodyRow(index: number): HTMLElement {
  const row = screen.getAllByRole("row")[index + 1];
  if (!row) throw new Error(`missing row ${index}`);
  return row;
}

describe("schema comparison result panels", () => {
  it("shows known changes and incomparable fields side by side with exact reasons", () => {
    render(
      <FieldMatrix
        schema={{ source: "public", target: "public" }}
        object={orders}
        page={{
          responseId: "r",
          identity,
          offset: 0,
          nextOffset: null,
          items: fields,
        }}
        eligibility={{ source: null, target: null }}
        selected={fields[1] ?? null}
        values={{
          source: { text: "now()", offset: 0, nextOffset: 4, complete: true },
          target: { text: "now()", offset: 0, nextOffset: 4, complete: true },
        }}
        loading={null}
        onSelect={noop}
        onPage={noop}
        onChunk={noop}
      />,
    );
    expect(screen.getByText("1 changed · 1 not comparable")).toBeTruthy();
    expect(within(bodyRow(0)).getByText("Changed")).toBeTruthy();
    expect(within(bodyRow(1)).getByText("Not comparable")).toBeTruthy();
    expect(within(bodyRow(3)).getByText("Equal within scope")).toBeTruthy();
    // Identical raw text stays not comparable, with the version reason visible.
    expect(
      screen.getAllByText(/Rendered on different server versions/).length,
    ).toBeGreaterThan(0);
    expect(screen.getByText("Fields 1–4 of 4")).toBeTruthy();
  });

  it("renders directional absence as Absent while keeping the index owner and key position", () => {
    render(
      <FieldMatrix
        schema={{ source: "public", target: "public" }}
        object={orders}
        page={{
          responseId: "r",
          identity,
          offset: 0,
          nextOffset: null,
          items: fields,
        }}
        eligibility={{ source: null, target: null }}
        selected={fields[2] ?? null}
        values={{
          source: { text: "id", offset: 0, nextOffset: 4, complete: true },
          target: null,
        }}
        loading={null}
        onSelect={noop}
        onPage={noop}
        onChunk={noop}
      />,
    );
    const row = bodyRow(2);
    expect(within(row).getByText("Absent")).toBeTruthy();
    expect(within(row).getByText("Source only")).toBeTruthy();
    expect(within(row).getAllByText("orders_pkey")).toHaveLength(2);
    expect(within(row).getByText("#1")).toBeTruthy();
    const target = screen.getByRole("region", { name: "Target value" });
    expect(within(target).getByText("Absent")).toBeTruthy();
  });

  it("explains an excluded counterpart through eligibility instead of absence", () => {
    render(
      <FieldMatrix
        schema={{ source: "public", target: "public" }}
        object={events}
        page={null}
        eligibility={{
          source: { kind: "eligible" },
          target: { kind: "excluded", reason: "partitioned" },
        }}
        selected={null}
        values={{ source: null, target: null }}
        loading={null}
        onSelect={noop}
        onPage={noop}
        onChunk={noop}
      />,
    );
    expect(screen.getByText(/eligible ordinary table/)).toBeTruthy();
    expect(screen.getByText(/excluded, partitioned table/)).toBeTruthy();
    expect(
      screen.getAllByText(/not directional absence/).length,
    ).toBeGreaterThan(0);
    expect(screen.queryByText("Absent")).toBeNull();
    expect(screen.queryByText("Target only")).toBeNull();
  });

  it("labels a missing side of an excluded definition as Excluded, never Absent", () => {
    const eventsWithFields = { ...events, fieldCount: 1 };
    const persistence: SchemaCompareFieldSummary = {
      kind: "notComparable",
      reason: "excludedCounterpart",
      path: { kind: "table", field: "persistence" },
      source: ref("source", 7),
    };
    render(
      <FieldMatrix
        schema={{ source: "public", target: "public" }}
        object={eventsWithFields}
        page={{
          responseId: "r",
          identity,
          offset: 0,
          nextOffset: null,
          items: [persistence],
        }}
        eligibility={{
          source: { kind: "eligible" },
          target: { kind: "excluded", reason: "partitioned" },
        }}
        selected={persistence}
        values={{
          source: { text: "p", offset: 0, nextOffset: 4, complete: true },
          target: null,
        }}
        loading={null}
        onSelect={noop}
        onPage={noop}
        onChunk={noop}
      />,
    );
    expect(screen.queryByText("Absent")).toBeNull();
    expect(screen.getAllByText("Excluded (partitioned table)")).toHaveLength(2);
    expect(screen.getByText(/excluded, partitioned table/)).toBeTruthy();
    // The row itself carries the reason; it is not hidden behind selection.
    expect(
      within(bodyRow(0)).getByText(/not directional absence/),
    ).toBeTruthy();
  });

  it("says when an object has no comparable fields in scope", () => {
    render(
      <FieldMatrix
        schema={{ source: "public", target: "public" }}
        object={{ ...customers, fieldCount: 0 }}
        page={null}
        eligibility={{ source: null, target: null }}
        selected={null}
        values={{ source: null, target: null }}
        loading={null}
        onSelect={noop}
        onPage={noop}
        onChunk={noop}
      />,
    );
    expect(
      screen.getByText("No comparable fields in scope for this object."),
    ).toBeTruthy();
  });

  it("lists explicit server page positions and equality only within scope", () => {
    render(
      <ObjectList
        page={{
          responseId: "r",
          identity,
          offset: 100,
          nextOffset: null,
          items: [orders, audit, events, customers],
        }}
        total={104}
        selected={customers}
        onSelect={noop}
        onPage={noop}
      />,
    );
    expect(screen.getByText("101–104 of 104")).toBeTruthy();
    const selected = screen.getByRole("button", { pressed: true });
    expect(within(selected).getByText("customers")).toBeTruthy();
    expect(within(selected).getByText("Equal within scope")).toBeTruthy();
    expect(screen.getByText("Source only")).toBeTruthy();
    expect(screen.getByText("Not comparable")).toBeTruthy();
    expect(
      screen
        .getByRole("button", { name: "Previous object page" })
        .matches(":disabled"),
    ).toBe(false);
    expect(
      screen
        .getByRole("button", { name: "Next object page" })
        .matches(":disabled"),
    ).toBe(true);
  });

  it("never invents exact totals for incomplete exclusion counts", () => {
    render(<CoverageSummary detail={detail} />);
    expect(screen.getByText("Not compared")).toBeTruthy();
    expect(screen.getByText("At least 4")).toBeTruthy();
    expect(screen.getByText("0")).toBeTruthy();
    expect(screen.getByText("1 field not comparable")).toBeTruthy();
    expect(
      screen.getByText(
        /Independent captures. This is not a single cross-database snapshot./,
      ),
    ).toBeTruthy();
    expect(screen.getByText(/PostgreSQL 16.14/)).toBeTruthy();
  });
});
