import { z } from "zod";

import { tauriInvoke } from "@/lib/tauri";

import {
  schemaCompareStatus,
  schemaCompareStartRequest,
  schemaCompareResultRequest,
  schemaCompareReadRequest,
  schemaCompareObjectPage,
  schemaCompareFieldPage,
  schemaCompareChunk,
  schemaCompareMetadataPage,
  schemaCompareEligibilityPage,
  type SchemaCompareEndpoint,
  type SchemaCompareResultIdentity,
  type SchemaCompareStartRequest,
  type SchemaCompareResultRequest,
  type SchemaCompareReadRequest,
  type SchemaCompareValueRef,
} from "./protocol";

/** Keep this request unchanged when reconciling an uncertain start. New starts
 * accept IDs created within one minute; retained requests reconcile for ten.
 */
export function createSchemaCompareRequest(
  source: SchemaCompareEndpoint,
  target: SchemaCompareEndpoint,
): SchemaCompareStartRequest {
  return schemaCompareStartRequest.parse({
    requestId: `${Date.now()}:${crypto.randomUUID()}`,
    source,
    target,
  });
}

export type SchemaCompareInvoke = typeof tauriInvoke<unknown>;

function sameIdentity(
  a: SchemaCompareResultIdentity,
  b: SchemaCompareResultIdentity,
) {
  return a.jobId === b.jobId && a.resultId === b.resultId;
}

async function readPage<
  T extends { responseId: string; identity: SchemaCompareResultIdentity },
>(
  invoke: SchemaCompareInvoke,
  request: SchemaCompareResultRequest,
  read: SchemaCompareReadRequest,
  schema: z.ZodType<T>,
): Promise<T> {
  // Fetch for each read: a reload retires the previous document's native token.
  // Never retry a rejected read with a newer token; it may belong to an old page.
  const transport = z
    .string()
    .uuid()
    .parse(await invoke("get_pg_schema_compare_transport"));
  const responseId = crypto.randomUUID();
  const received = await invoke("read_pg_schema_compare", {
    responseId,
    transport,
    request: schemaCompareResultRequest.parse(request),
    read: schemaCompareReadRequest.parse(read),
  });
  // A fulfilled invoke proves receipt, even if validation below fails. A rejected
  // or lost response is uncertain and must keep its native serializer lease.
  try {
    const page = schema.parse(received);
    if (
      page.responseId !== responseId ||
      !sameIdentity(page.identity, request.identity)
    ) {
      throw new Error("Schema comparison response identity mismatch");
    }
    return page;
  } finally {
    await invoke("acknowledge_pg_schema_compare", { transport, responseId });
  }
}

type Relation = z.infer<typeof schemaCompareReadRequest> & { kind: "fields" };

export function createSchemaCompareClient(
  invoke: SchemaCompareInvoke = tauriInvoke,
) {
  return {
    start: async (payload: SchemaCompareStartRequest) =>
      schemaCompareStatus.parse(
        await invoke("start_pg_schema_compare", {
          payload: schemaCompareStartRequest.parse(payload),
        }),
      ),
    list: async () =>
      z
        .array(schemaCompareStatus)
        .max(4)
        .parse(await invoke("list_pg_schema_compares")),
    get: async (jobId: string) =>
      schemaCompareStatus.parse(
        await invoke("get_pg_schema_compare", { jobId }),
      ),
    cancel: async (jobId: string) =>
      schemaCompareStatus.parse(
        await invoke("cancel_pg_schema_compare", { jobId }),
      ),
    release: async (jobId: string) => {
      await invoke("release_pg_schema_compare", { jobId });
    },
    metadata: async (request: SchemaCompareResultRequest) => {
      const page = await readPage(
        invoke,
        request,
        { kind: "metadata" },
        schemaCompareMetadataPage,
      );
      const { metadata } = page.detail;
      if (
        !sameIdentity(metadata.identity, request.identity) ||
        metadata.source.endpoint.connectionId !== request.source.connectionId ||
        metadata.source.endpoint.schema !== request.source.schema ||
        metadata.target.endpoint.connectionId !== request.target.connectionId ||
        metadata.target.endpoint.schema !== request.target.schema
      ) {
        throw new Error("Schema comparison endpoint mismatch");
      }
      return page;
    },
    objects: async (request: SchemaCompareResultRequest, offset = 0) => {
      const page = await readPage(
        invoke,
        request,
        { kind: "objects", offset },
        schemaCompareObjectPage,
      );
      if (page.offset !== offset)
        throw new Error("Schema comparison page offset mismatch");
      return page;
    },
    fields: async (
      request: SchemaCompareResultRequest,
      object: Relation["object"],
      offset = 0,
    ) => {
      const page = await readPage(
        invoke,
        request,
        { kind: "fields", object, offset },
        schemaCompareFieldPage,
      );
      if (page.offset !== offset)
        throw new Error("Schema comparison page offset mismatch");
      return page;
    },
    eligibility: async (
      request: SchemaCompareResultRequest,
      object: Relation["object"],
      side: "source" | "target",
    ) => {
      const page = await readPage(
        invoke,
        request,
        { kind: "eligibility", object, side },
        schemaCompareEligibilityPage,
      );
      if (
        page.detail.side !== side ||
        page.detail.object.kind !== object.kind ||
        page.detail.object.name !== object.name
      )
        throw new Error("Schema comparison object mismatch");
      return page;
    },
    value: async (
      request: SchemaCompareResultRequest,
      value: SchemaCompareValueRef,
      offset = 0,
    ) => {
      const page = await readPage(
        invoke,
        request,
        { kind: "value", value, offset },
        schemaCompareChunk,
      );
      if (
        page.offset !== offset ||
        page.value.side !== value.side ||
        page.value.valueId !== value.valueId ||
        page.value.rawBytes !== value.rawBytes ||
        page.value.valueKind !== value.valueKind
      )
        throw new Error("Schema comparison value mismatch");
      return page;
    },
  };
}

export const schemaCompareClient = createSchemaCompareClient();
