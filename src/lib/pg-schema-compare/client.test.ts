import { beforeEach, describe, expect, it, vi } from "vitest";

import {
  createSchemaCompareRequest,
  createSchemaCompareClient,
  type SchemaCompareInvoke,
} from "./client";

const transport = "4f2b9150-58d7-4a77-8819-90bccf0329b9";
const invoke = vi.fn<SchemaCompareInvoke>();
const schemaCompareClient = createSchemaCompareClient(invoke);
const request = {
  identity: { jobId: "job", resultId: "result" },
  source: { connectionId: "a", schema: "public" },
  target: { connectionId: "b", schema: "public" },
};

beforeEach(() => {
  vi.resetAllMocks();
});

describe("schema comparison client ownership", () => {
  it("keeps the caller request stable and never retries an uncertain start", async () => {
    const payload = createSchemaCompareRequest(request.source, request.target);
    expect(payload.requestId).toMatch(/^\d+:/);
    invoke.mockRejectedValueOnce({ kind: "transport" });
    await expect(schemaCompareClient.start(payload)).rejects.toEqual({
      kind: "transport",
    });
    expect(invoke).toHaveBeenCalledTimes(1);
    invoke.mockResolvedValueOnce([
      {
        jobId: "job",
        ...payload,
        phase: "resolving",
        sourceObjects: 0,
        targetObjects: 0,
      },
    ]);
    expect((await schemaCompareClient.list())[0].requestId).toBe(
      payload.requestId,
    );
  });

  it("acknowledges received pages after decoding and binds the result identity", async () => {
    invoke.mockImplementation(async (command, payload) => {
      if (command === "get_pg_schema_compare_transport") return transport;
      if (command === "acknowledge_pg_schema_compare") return null;
      return {
        responseId: payload?.responseId,
        identity: request.identity,
        offset: 0,
        nextOffset: null,
        items: [],
      };
    });
    const page = await schemaCompareClient.objects(request);
    expect(page.items).toEqual([]);
    expect(invoke).toHaveBeenNthCalledWith(3, "acknowledge_pg_schema_compare", {
      transport,
      responseId: page.responseId,
    });
  });

  it("acknowledges malformed fulfilled responses, but preserves leases on uncertain transport errors", async () => {
    invoke
      .mockResolvedValueOnce(transport)
      .mockResolvedValueOnce({ broken: true })
      .mockResolvedValueOnce(null);
    await expect(schemaCompareClient.objects(request)).rejects.toThrow();
    expect(invoke).toHaveBeenLastCalledWith(
      "acknowledge_pg_schema_compare",
      expect.objectContaining({ responseId: expect.any(String) }),
    );
    invoke
      .mockReset()
      .mockResolvedValueOnce(transport)
      .mockRejectedValueOnce({ kind: "unavailable" });
    await expect(schemaCompareClient.objects(request)).rejects.toEqual({
      kind: "unavailable",
    });
    expect(invoke).toHaveBeenCalledTimes(2);
  });

  it("refuses a response for another result even when its shape is valid", async () => {
    invoke.mockImplementation(async (command, payload) => {
      if (command === "get_pg_schema_compare_transport") return transport;
      if (command === "acknowledge_pg_schema_compare") return null;
      return {
        responseId: payload?.responseId,
        identity: { ...request.identity, resultId: "stale" },
        offset: 0,
        nextOffset: null,
        items: [],
      };
    });
    await expect(schemaCompareClient.objects(request)).rejects.toThrow(
      "identity mismatch",
    );
    expect(invoke).toHaveBeenCalledTimes(3);
  });

  it("keeps independent source and target value references and validates their offsets", async () => {
    const value = {
      side: "target",
      valueId: 3,
      rawBytes: 4,
      valueKind: "text",
    } as const;
    invoke.mockImplementation(async (command, payload) => {
      if (command === "get_pg_schema_compare_transport") return transport;
      if (command === "acknowledge_pg_schema_compare") return null;
      return {
        responseId: payload?.responseId,
        identity: request.identity,
        value,
        offset: 0,
        text: "🦀",
        nextOffset: 4,
        complete: true,
      };
    });
    expect((await schemaCompareClient.value(request, value)).text).toBe("🦀");
    await expect(
      schemaCompareClient.value(request, { ...value, side: "source" }),
    ).rejects.toThrow("value mismatch");
  });

  it("uses the current document token for each read and never retries a stale read", async () => {
    const nextTransport = "32c3961c-f0f9-4a7e-a59b-cbf510bc062c";
    invoke
      .mockResolvedValueOnce(transport)
      .mockRejectedValueOnce({ kind: "unavailable" });
    await expect(schemaCompareClient.objects(request)).rejects.toEqual({
      kind: "unavailable",
    });
    expect(invoke).toHaveBeenCalledTimes(2);
    expect(invoke).toHaveBeenLastCalledWith(
      "read_pg_schema_compare",
      expect.objectContaining({ transport }),
    );

    invoke.mockImplementation(async (command, payload) => {
      if (command === "get_pg_schema_compare_transport") return nextTransport;
      if (command === "acknowledge_pg_schema_compare") return null;
      return {
        responseId: payload?.responseId,
        identity: request.identity,
        offset: 0,
        nextOffset: null,
        items: [],
      };
    });
    await schemaCompareClient.objects(request);
    expect(invoke).toHaveBeenLastCalledWith(
      "acknowledge_pg_schema_compare",
      expect.objectContaining({ transport: nextTransport }),
    );
  });
});
