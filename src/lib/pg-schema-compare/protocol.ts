import { z } from "zod";

export const SCHEMA_COMPARE_SCOPE = "postgres16OrdinaryTableProjectionV1";
export const SCHEMA_COMPARE_NORMALIZATION_VERSION = 1;
export const SCHEMA_COMPARE_CHUNK_BYTES = 64 * 1024;
export const SCHEMA_COMPARE_PAGE_BYTES = 1024 * 1024;

const encoder = new TextEncoder();
const bytes = (limit: number) =>
  z
    .string()
    .refine(
      (text) => text.length <= limit && encoder.encode(text).length <= limit,
      "UTF-8 byte limit exceeded",
    );
const identifier = bytes(63).refine(
  (text) => text.length > 0 && !text.includes("\0"),
);
const id = bytes(128).refine((text) => text.length > 0);
const count = z.number().int().nonnegative().max(100_000);

export const schemaCompareEndpoint = z.object({
  connectionId: id,
  schema: identifier,
});
export type SchemaCompareEndpoint = z.infer<typeof schemaCompareEndpoint>;

export const schemaCompareResultIdentity = z.object({
  jobId: id,
  resultId: id,
});
export type SchemaCompareResultIdentity = z.infer<
  typeof schemaCompareResultIdentity
>;

export const schemaCompareNamespace = z.discriminatedUnion("kind", [
  z.object({ kind: z.literal("selected") }),
  z.object({ kind: z.literal("external"), schema: identifier }),
]);
export const schemaCompareQualifiedName = z.object({
  namespace: schemaCompareNamespace,
  name: identifier,
});
export type SchemaCompareQualifiedName = z.infer<
  typeof schemaCompareQualifiedName
>;

export const schemaCompareRelationIdentity = z.object({
  kind: z.enum([
    "table",
    "partitionedTable",
    "foreignTable",
    "view",
    "materializedView",
    "sequence",
    "composite",
  ]),
  name: identifier,
});

export const schemaCompareFieldPath = z.discriminatedUnion("kind", [
  z.object({
    kind: z.literal("table"),
    field: z.enum(["persistence", "comment"]),
  }),
  z.object({
    kind: z.literal("column"),
    name: identifier,
    field: z.enum([
      "position",
      "type",
      "typeModifier",
      "arrayDimensions",
      "nullable",
      "default",
      "generatedKind",
      "generatedExpression",
      "identity",
      "collation",
      "comment",
    ]),
  }),
  z.object({
    kind: z.literal("constraint"),
    name: identifier,
    field: z.enum([
      "kind",
      "keys",
      "referencedTable",
      "referencedKeys",
      "updateAction",
      "deleteAction",
      "deleteColumns",
      "matchMode",
      "deferrable",
      "initiallyDeferred",
      "validated",
      "noInherit",
      "expression",
      "equalityOperators",
      "exclusionOperators",
    ]),
  }),
  z.object({
    kind: z.literal("index"),
    name: identifier,
    owner: identifier.nullable(),
    field: z.enum([
      "accessMethod",
      "unique",
      "nullsNotDistinct",
      "immediate",
      "keyCount",
      "includedColumns",
      "predicate",
      "relationOptions",
      "valid",
      "ready",
      "live",
    ]),
  }),
  z.object({
    kind: z.literal("indexKey"),
    name: identifier,
    owner: identifier.nullable(),
    position: z.number().int().nonnegative().max(65_535),
    field: z.enum([
      "kind",
      "column",
      "expression",
      "sortOptions",
      "opclass",
      "opclassOptions",
      "collation",
    ]),
  }),
]);
export type SchemaCompareFieldPath = z.infer<typeof schemaCompareFieldPath>;

export const schemaCompareValueRef = z.object({
  side: z.enum(["source", "target"]),
  valueId: z.number().int().nonnegative().max(99_999),
  rawBytes: z
    .number()
    .int()
    .nonnegative()
    .max(256 * 1024),
  valueKind: z.enum([
    "null",
    "text",
    "boolean",
    "integer",
    "qualifiedName",
    "orderedNames",
    "orderedReferences",
    "operatorSignatures",
  ]),
});
export type SchemaCompareValueRef = z.infer<typeof schemaCompareValueRef>;
export const schemaCompareValueRequest = z.object({
  identity: schemaCompareResultIdentity,
  value: schemaCompareValueRef,
  offset: z
    .number()
    .int()
    .nonnegative()
    .max(256 * 1024),
});
export type SchemaCompareValueRequest = z.infer<
  typeof schemaCompareValueRequest
>;

export const schemaCompareChunk = z
  .object({
    responseId: id,
    identity: schemaCompareResultIdentity,
    value: schemaCompareValueRef,
    offset: z
      .number()
      .int()
      .nonnegative()
      .max(256 * 1024),
    text: bytes(SCHEMA_COMPARE_CHUNK_BYTES),
    nextOffset: z
      .number()
      .int()
      .nonnegative()
      .max(256 * 1024),
    complete: z.boolean(),
  })
  .refine(
    (chunk) =>
      chunk.text.length <= SCHEMA_COMPARE_CHUNK_BYTES &&
      chunk.nextOffset === chunk.offset + encoder.encode(chunk.text).length &&
      chunk.nextOffset <= chunk.value.rawBytes &&
      chunk.complete === (chunk.nextOffset === chunk.value.rawBytes),
    "Invalid chunk offsets",
  );
export type SchemaCompareChunk = z.infer<typeof schemaCompareChunk>;

const incomparableReason = z.enum([
  "expressionOutsideSubset",
  "renderingVersionDifference",
  "externalDependency",
  "unknownAccessMethod",
  "excludedCounterpart",
  "excludedObject",
]);
const sourceValueRef = schemaCompareValueRef.extend({
  side: z.literal("source"),
});
const targetValueRef = schemaCompareValueRef.extend({
  side: z.literal("target"),
});
const fieldSummary = { path: schemaCompareFieldPath };
export const schemaCompareFieldSummary = z.union([
  z.strictObject({
    ...fieldSummary,
    kind: z.literal("equal"),
    source: sourceValueRef,
    target: targetValueRef,
  }),
  z.strictObject({
    ...fieldSummary,
    kind: z.literal("changed"),
    source: sourceValueRef,
    target: targetValueRef,
  }),
  z.strictObject({
    ...fieldSummary,
    kind: z.literal("sourceOnly"),
    source: sourceValueRef,
  }),
  z.strictObject({
    ...fieldSummary,
    kind: z.literal("targetOnly"),
    target: targetValueRef,
  }),
  z.strictObject({
    ...fieldSummary,
    kind: z.literal("notComparable"),
    reason: incomparableReason,
    source: sourceValueRef,
    target: targetValueRef,
  }),
  z.strictObject({
    ...fieldSummary,
    kind: z.literal("notComparable"),
    reason: incomparableReason,
    source: sourceValueRef,
  }),
  z.strictObject({
    ...fieldSummary,
    kind: z.literal("notComparable"),
    reason: incomparableReason,
    target: targetValueRef,
  }),
]);
export type SchemaCompareFieldSummary = z.infer<
  typeof schemaCompareFieldSummary
>;
const objectCounts = {
  fieldCount: count,
  changedFields: count,
  incomparableFields: count,
};
export const schemaCompareObjectSummary = z.union([
  z.strictObject({
    ...objectCounts,
    kind: z.literal("equal"),
    source: schemaCompareRelationIdentity,
    target: schemaCompareRelationIdentity,
  }),
  z.strictObject({
    ...objectCounts,
    kind: z.literal("changed"),
    source: schemaCompareRelationIdentity,
    target: schemaCompareRelationIdentity,
  }),
  z.strictObject({
    ...objectCounts,
    kind: z.literal("sourceOnly"),
    source: schemaCompareRelationIdentity,
  }),
  z.strictObject({
    ...objectCounts,
    kind: z.literal("targetOnly"),
    target: schemaCompareRelationIdentity,
  }),
  z.strictObject({
    ...objectCounts,
    kind: z.literal("notComparable"),
    reason: incomparableReason,
    source: schemaCompareRelationIdentity,
    target: schemaCompareRelationIdentity,
  }),
  z.strictObject({
    ...objectCounts,
    kind: z.literal("notComparable"),
    reason: incomparableReason,
    source: schemaCompareRelationIdentity,
  }),
  z.strictObject({
    ...objectCounts,
    kind: z.literal("notComparable"),
    reason: incomparableReason,
    target: schemaCompareRelationIdentity,
  }),
]);
const pageMetadata = {
  responseId: id,
  identity: schemaCompareResultIdentity,
  offset: count,
  nextOffset: count.nullable(),
};
export const schemaCompareObjectPage = z.object({
  ...pageMetadata,
  items: z.array(schemaCompareObjectSummary).max(100),
});
export const schemaCompareFieldPage = z.object({
  ...pageMetadata,
  items: z.array(schemaCompareFieldSummary).max(100),
});

export const schemaCompareError = z.discriminatedUnion("kind", [
  z.object({ kind: z.literal("busy") }),
  z.object({
    kind: z.literal("limitExceeded"),
    limit: z.enum([
      "inventory",
      "tables",
      "childFacts",
      "fieldBytes",
      "endpointBytes",
      "resultBytes",
      "pageBytes",
      "pageItems",
      "allocation",
    ]),
  }),
  z.object({
    kind: z.literal("unsupportedVersion"),
    side: z.enum(["source", "target"]),
    version: bytes(128),
  }),
  z.object({
    kind: z.literal("unsupportedEngine"),
    side: z.enum(["source", "target"]),
  }),
  z.object({ kind: z.literal("unavailable") }),
  z.object({ kind: z.literal("invalidRequest") }),
  z.object({ kind: z.literal("captureChanged") }),
  z.object({ kind: z.literal("cancelled") }),
  z.object({ kind: z.literal("deadlineExceeded") }),
]);
export type SchemaCompareError = z.infer<typeof schemaCompareError>;

const statusMetadata = {
  jobId: id,
  requestId: id,
  source: schemaCompareEndpoint,
  target: schemaCompareEndpoint,
  sourceObjects: count,
  targetObjects: count,
};
export const schemaCompareStatus = z.discriminatedUnion("phase", [
  z.strictObject({ ...statusMetadata, phase: z.literal("resolving") }),
  z.strictObject({ ...statusMetadata, phase: z.literal("readingSource") }),
  z.strictObject({ ...statusMetadata, phase: z.literal("readingTarget") }),
  z.strictObject({ ...statusMetadata, phase: z.literal("readingBoth") }),
  z.strictObject({ ...statusMetadata, phase: z.literal("comparing") }),
  z.strictObject({
    ...statusMetadata,
    phase: z.literal("completed"),
    resultId: id,
  }),
  z.strictObject({ ...statusMetadata, phase: z.literal("cancelling") }),
  z.strictObject({ ...statusMetadata, phase: z.literal("cancelled") }),
  z.strictObject({
    ...statusMetadata,
    phase: z.literal("failed"),
    failure: schemaCompareError,
  }),
]);
export type SchemaCompareStatus = z.infer<typeof schemaCompareStatus>;

export const schemaCompareCaptureMetadata = z.object({
  endpoint: schemaCompareEndpoint,
  serverVersion: bytes(256),
  serverVersionNum: z.number().int().nonnegative().max(4_294_967_295),
  capturedAt: bytes(64),
});
const schemaCompareExcludedCategory = z.enum([
  "otherRelations",
  "routines",
  "sequences",
  "typesAndDomains",
  "policies",
  "grants",
  "triggers",
  "rules",
  "extensions",
  "databaseObjects",
  "identitySequenceConfiguration",
  "storageSecurityOwnershipReplication",
  "indexPlacementClusteringReplicaIdentity",
]);
export const schemaCompareEligibility = z.discriminatedUnion("kind", [
  z.object({ kind: z.literal("eligible") }),
  z.object({
    kind: z.literal("excluded"),
    reason: z.enum([
      "partitioned",
      "inherited",
      "foreign",
      "extensionOwned",
      "otherKind",
    ]),
  }),
]);
export const schemaCompareCoverage = z.object({
  scope: z.literal(SCHEMA_COMPARE_SCOPE),
  normalizationVersion: z.literal(SCHEMA_COMPARE_NORMALIZATION_VERSION),
  excludedRelations: count,
  incomparableFields: count,
  excludedCategories: z.array(schemaCompareExcludedCategory).max(13),
});
export const schemaCompareInventoryEntry = z.object({
  identity: schemaCompareRelationIdentity,
  eligibility: schemaCompareEligibility,
});
export const schemaCompareOperatorSignature = z.object({
  operator: schemaCompareQualifiedName,
  leftType: schemaCompareQualifiedName.nullable(),
  rightType: schemaCompareQualifiedName.nullable(),
});
export type SchemaCompareCoverage = z.infer<typeof schemaCompareCoverage>;
export type SchemaCompareCaptureMetadata = z.infer<
  typeof schemaCompareCaptureMetadata
>;
export type SchemaCompareInventoryEntry = z.infer<
  typeof schemaCompareInventoryEntry
>;
export type SchemaCompareOperatorSignature = z.infer<
  typeof schemaCompareOperatorSignature
>;

export const schemaCompareMetadata = z.object({
  identity: schemaCompareResultIdentity,
  source: schemaCompareCaptureMetadata,
  target: schemaCompareCaptureMetadata,
  consistency: z.enum(["sharedTransaction", "independentTransactions"]),
  coverage: schemaCompareCoverage,
});
export type SchemaCompareMetadata = z.infer<typeof schemaCompareMetadata>;

export const schemaCompareStartRequest = z.object({
  requestId: id,
  source: schemaCompareEndpoint,
  target: schemaCompareEndpoint,
});
export type SchemaCompareStartRequest = z.infer<
  typeof schemaCompareStartRequest
>;
export const schemaCompareResultRequest = z.object({
  identity: schemaCompareResultIdentity,
  source: schemaCompareEndpoint,
  target: schemaCompareEndpoint,
});
export type SchemaCompareResultRequest = z.infer<
  typeof schemaCompareResultRequest
>;
export const schemaCompareReadRequest = z.discriminatedUnion("kind", [
  z.object({ kind: z.literal("metadata") }),
  z.object({ kind: z.literal("objects"), offset: count }),
  z.object({
    kind: z.literal("fields"),
    object: schemaCompareRelationIdentity,
    offset: count,
  }),
  z.object({
    kind: z.literal("eligibility"),
    object: schemaCompareRelationIdentity,
    side: z.enum(["source", "target"]),
  }),
  z.object({
    kind: z.literal("value"),
    value: schemaCompareValueRef,
    offset: z
      .number()
      .int()
      .nonnegative()
      .max(256 * 1024),
  }),
]);
export type SchemaCompareReadRequest = z.infer<typeof schemaCompareReadRequest>;
const excludedCount = z.object({
  category: schemaCompareExcludedCategory,
  count,
  complete: z.boolean(),
});
export const schemaCompareMetadataPage = z.object({
  responseId: id,
  identity: schemaCompareResultIdentity,
  detail: z.object({
    metadata: schemaCompareMetadata,
    kind: z.enum([
      "equal",
      "changed",
      "sourceOnly",
      "targetOnly",
      "notComparable",
    ]),
    objectCount: count,
    sourceExcludedCounts: z.array(excludedCount).max(13),
    targetExcludedCounts: z.array(excludedCount).max(13),
  }),
});
export const schemaCompareEligibilityPage = z.object({
  responseId: id,
  identity: schemaCompareResultIdentity,
  detail: z.object({
    object: schemaCompareRelationIdentity,
    side: z.enum(["source", "target"]),
    eligibility: schemaCompareEligibility,
  }),
});
