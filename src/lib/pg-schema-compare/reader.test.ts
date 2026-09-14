import { describe, expect, it, vi } from "vitest";

import { createSchemaCompareClient, type SchemaCompareInvoke } from "./client";
import {
  SCHEMA_COMPARE_NORMALIZATION_VERSION,
  SCHEMA_COMPARE_SCOPE,
  type SchemaCompareFieldSummary,
  type SchemaCompareObjectSummary,
  type SchemaCompareResultRequest,
  schemaCompareReadRequest,
  schemaCompareResultRequest,
} from "./protocol";
import { createSchemaCompareReader } from "./reader";

const transport = "4f2b9150-58d7-4a77-8819-90bccf0329b9";
const source = { connectionId: "a", schema: "public" };
const target = { connectionId: "b", schema: "public" };
const resultA: SchemaCompareResultRequest = {
  identity: { jobId: "job-a", resultId: "result-a" },
  source,
  target,
};
const resultB: SchemaCompareResultRequest = {
  identity: { jobId: "job-b", resultId: "result-b" },
  source,
  target,
};
const capture = (endpoint: typeof source) => ({
  endpoint,
  serverVersion: "16.15",
  serverVersionNum: 160015,
  capturedAt: "2026-09-14T10:42:03Z",
});
const orders: SchemaCompareObjectSummary = {
  kind: "changed",
  fieldCount: 2,
  changedFields: 1,
  incomparableFields: 1,
  source: { kind: "table", name: "orders" },
  target: { kind: "table", name: "orders" },
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
const crab = "🦀";
const totalType: SchemaCompareFieldSummary = {
  kind: "changed",
  path: { kind: "column", name: "total", field: "type" },
  source: { side: "source", valueId: 0, rawBytes: 8, valueKind: "text" },
  target: { side: "target", valueId: 1, rawBytes: 0, valueKind: "text" },
};

function flush() {
  return new Promise((resolve) => setTimeout(resolve, 0));
}
function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((res) => {
    resolve = res;
  });
  return { promise, resolve };
}

/** A native double behind the real client, so acknowledgement stays observable. */
function harness() {
  const acknowledged: string[] = [];
  const reads: Array<ReturnType<typeof schemaCompareReadRequest.parse>> = [];
  let held: Promise<void> | null = null;
  let corruptFields = false;
  const invoke = vi.fn<SchemaCompareInvoke>(async (command, payload) => {
    if (command === "get_pg_schema_compare_transport") return transport;
    if (command === "acknowledge_pg_schema_compare") {
      acknowledged.push(String(payload?.responseId));
      return null;
    }
    const request = schemaCompareResultRequest.parse(payload?.request);
    const read = schemaCompareReadRequest.parse(payload?.read);
    reads.push(read);
    const base = {
      responseId: payload?.responseId,
      identity: request.identity,
    };
    if (held) await held;
    switch (read.kind) {
      case "metadata":
        return {
          ...base,
          detail: {
            metadata: {
              identity: request.identity,
              source: capture(request.source),
              target: capture(request.target),
              consistency: "independentTransactions",
              coverage: {
                scope: SCHEMA_COMPARE_SCOPE,
                normalizationVersion: SCHEMA_COMPARE_NORMALIZATION_VERSION,
                excludedRelations: 1,
                incomparableFields: 1,
                excludedCategories: ["routines"],
              },
            },
            kind: "changed",
            objectCount: 2,
            sourceExcludedCounts: [
              { category: "routines", count: 0, complete: false },
            ],
            targetExcludedCounts: [],
          },
        };
      case "objects":
        return {
          ...base,
          offset: 0,
          nextOffset: null,
          items: [orders, events],
        };
      case "fields":
        if (corruptFields) return { ...base, offset: 0, items: "broken" };
        return {
          ...base,
          offset: read.offset,
          nextOffset: null,
          items: [
            {
              ...totalType,
              path: { kind: "column", name: read.object.name, field: "type" },
            },
          ],
        };
      case "eligibility":
        return {
          ...base,
          detail: {
            object: read.object,
            side: read.side,
            eligibility:
              read.side === "target"
                ? { kind: "excluded", reason: "partitioned" }
                : { kind: "eligible" },
          },
        };
      case "value":
        return {
          ...base,
          value: read.value,
          offset: read.offset,
          text: crab,
          nextOffset: read.offset + 4,
          complete: read.offset + 4 === read.value.rawBytes,
        };
    }
  });
  const reader = createSchemaCompareReader(createSchemaCompareClient(invoke));
  return {
    reader,
    invoke,
    acknowledged,
    reads,
    /** Every read issued while held waits; release lets them all through. */
    hold() {
      const barrier = deferred<void>();
      held = barrier.promise;
      return {
        release() {
          held = null;
          barrier.resolve();
        },
      };
    },
    corrupt(next: boolean) {
      corruptFields = next;
    },
  };
}

describe("schema comparison result reader", () => {
  it("reads metadata then the first object page, acknowledging each response", async () => {
    const h = harness();
    h.reader.open(resultA);
    await flush();
    const state = h.reader.store.getState();
    expect(state.metadata?.objectCount).toBe(2);
    expect(state.objects?.items).toHaveLength(2);
    expect(state.loading).toBeNull();
    expect(h.reads.map((r) => r.kind)).toEqual(["metadata", "objects"]);
    expect(h.acknowledged).toHaveLength(2);
  });

  it("acknowledges a late response for a superseded result without applying it", async () => {
    const h = harness();
    const held = h.hold();
    h.reader.open(resultA);
    await flush();
    expect(h.invoke).toHaveBeenCalledWith(
      "read_pg_schema_compare",
      expect.objectContaining({ request: resultA }),
    );
    h.reader.open(resultB);
    held.release();
    await flush();
    await flush();
    const state = h.reader.store.getState();
    expect(state.request).toEqual(resultB);
    expect(state.metadata?.metadata.identity).toEqual(resultB.identity);
    // Result A's single metadata read was acknowledged; B read metadata and objects.
    expect(h.acknowledged).toHaveLength(3);
    expect(h.reads.map((r) => r.kind)).toEqual([
      "metadata",
      "metadata",
      "objects",
    ]);
  });

  it("queues only the latest selection and never fills the newer view from an older read", async () => {
    const h = harness();
    h.reader.open(resultA);
    await flush();
    const held = h.hold();
    h.reader.selectObject(orders);
    await flush();
    h.reader.selectObject(events);
    h.reader.selectObject(orders);
    held.release();
    await flush();
    await flush();
    const fieldReads = h.reads.filter((r) => r.kind === "fields");
    // The intermediate selection of `events` was superseded before it ran.
    expect(fieldReads.map((r) => r.object.name)).toEqual(["orders", "orders"]);
    const state = h.reader.store.getState();
    expect(state.selectedObject).toEqual(orders);
    expect(state.fields?.items[0]?.path).toMatchObject({ name: "orders" });
    expect(state.loading).toBeNull();
  });

  it("explains an excluded counterpart through eligibility reads for each observed side", async () => {
    const h = harness();
    h.reader.open(resultA);
    await flush();
    h.reader.selectObject(events);
    await flush();
    const state = h.reader.store.getState();
    expect(state.fields).toBeNull();
    expect(state.eligibility).toEqual({
      source: { kind: "eligible" },
      target: { kind: "excluded", reason: "partitioned" },
    });
    expect(h.reads.filter((r) => r.kind === "eligibility")).toHaveLength(2);
  });

  it("drops every payload when the result becomes unavailable", async () => {
    const h = harness();
    h.reader.open(resultA);
    await flush();
    h.invoke.mockImplementationOnce(async () => transport);
    h.invoke.mockRejectedValueOnce({ kind: "unavailable" });
    h.reader.selectObject(orders);
    await flush();
    const state = h.reader.store.getState();
    expect(state.unavailable).toBe(true);
    expect(state.request).toEqual(resultA);
    expect(state.metadata).toBeNull();
    expect(state.objects).toBeNull();
    expect(state.selectedObject).toBeNull();
  });

  it("discards every page on transport loss and reopens the result on retry", async () => {
    const h = harness();
    h.reader.open(resultA);
    await flush();
    h.reader.selectObject(orders);
    await flush();
    expect(h.reader.store.getState().fields).not.toBeNull();
    h.invoke.mockImplementationOnce(async () => transport);
    h.invoke.mockRejectedValueOnce("lost");
    h.reader.selectField(totalType);
    await flush();
    let state = h.reader.store.getState();
    expect(state.error).toEqual({ kind: "transport" });
    expect(state.unavailable).toBe(false);
    expect(state.request).toEqual(resultA);
    expect(state.metadata).toBeNull();
    expect(state.objects).toBeNull();
    expect(state.fields).toBeNull();
    expect(state.selectedObject).toBeNull();
    h.reader.retry();
    await flush();
    state = h.reader.store.getState();
    expect(state.error).toBeNull();
    expect(state.metadata).not.toBeNull();
    expect(state.objects?.items).toHaveLength(2);
    expect(state.selectedObject).toBeNull();
  });

  it("records a validation failure for a corrupt page, still acknowledges it, and retries the same read", async () => {
    const h = harness();
    h.reader.open(resultA);
    await flush();
    h.corrupt(true);
    h.reader.selectObject(orders);
    await flush();
    let state = h.reader.store.getState();
    expect(state.error).toEqual({ kind: "invalidResponse" });
    expect(state.fields).toBeNull();
    expect(state.loading).toBeNull();
    expect(h.acknowledged).toHaveLength(3);
    h.corrupt(false);
    h.reader.retry();
    await flush();
    state = h.reader.store.getState();
    expect(state.error).toBeNull();
    expect(state.fields?.items).toHaveLength(1);
  });

  it("pages values by returned UTF-8 byte offsets and fetches nothing for an empty value", async () => {
    const h = harness();
    h.reader.open(resultA);
    await flush();
    h.reader.selectObject(orders);
    await flush();
    h.reader.selectField(totalType);
    await flush();
    let state = h.reader.store.getState();
    expect(state.values.source).toEqual({
      text: crab,
      offset: 0,
      nextOffset: 4,
      complete: false,
    });
    expect(state.values.target).toEqual({
      text: "",
      offset: 0,
      nextOffset: 0,
      complete: true,
    });
    expect(h.reads.filter((r) => r.kind === "value")).toHaveLength(1);
    h.reader.valueChunk("source", "next");
    await flush();
    state = h.reader.store.getState();
    expect(state.values.source).toEqual({
      text: crab,
      offset: 4,
      nextOffset: 8,
      complete: true,
    });
    // Complete: there is no next chunk to request.
    h.reader.valueChunk("source", "next");
    await flush();
    // Previous returns to the offset the earlier chunk started at, never to
    // an arithmetic offset that could split a code point.
    h.reader.valueChunk("source", "previous");
    await flush();
    state = h.reader.store.getState();
    expect(state.values.source?.offset).toBe(0);
    expect(state.previousValueOffsets.source).toEqual([]);
    h.reader.valueChunk("source", "previous");
    await flush();
    const valueReads = h.reads.filter((r) => r.kind === "value");
    expect(valueReads.map((r) => r.offset)).toEqual([0, 4, 0]);
  });

  it("closing drops payloads and ignores the outstanding read while still acknowledging it", async () => {
    const h = harness();
    const held = h.hold();
    h.reader.open(resultA);
    await flush();
    h.reader.close();
    held.release();
    await flush();
    await flush();
    const state = h.reader.store.getState();
    expect(state.request).toBeNull();
    expect(state.metadata).toBeNull();
    expect(state.loading).toBeNull();
    expect(h.acknowledged).toHaveLength(1);
    expect(h.reads.map((r) => r.kind)).toEqual(["metadata"]);
  });
});
