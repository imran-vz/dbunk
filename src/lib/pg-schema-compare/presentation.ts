import type { SchemaCompareFailure } from "./failure";
import type {
  SchemaCompareFieldPath,
  SchemaCompareFieldSummary,
  SchemaCompareObjectSummary,
  SchemaCompareStatus,
} from "./protocol";
import type {
  SchemaCompareEligibilityDetail,
  SchemaCompareMetadataDetail,
} from "./reader";

export const PHASE_LABEL = {
  resolving: "Resolving endpoints",
  readingSource: "Reading source",
  readingTarget: "Reading target",
  readingBoth: "Reading source and target",
  comparing: "Comparing",
  completed: "Completed",
  cancelling: "Cancelling",
  cancelled: "Cancelled",
  failed: "Failed",
} satisfies Record<SchemaCompareStatus["phase"], string>;

export type PhaseTone = "success" | "danger" | "neutral";
export const PHASE_TONE = {
  resolving: "neutral",
  readingSource: "neutral",
  readingTarget: "neutral",
  readingBoth: "neutral",
  comparing: "neutral",
  completed: "success",
  cancelling: "neutral",
  cancelled: "neutral",
  failed: "danger",
} satisfies Record<SchemaCompareStatus["phase"], PhaseTone>;

export type DifferenceKind =
  | SchemaCompareObjectSummary["kind"]
  | SchemaCompareFieldSummary["kind"];
export type DifferenceTone = "success" | "warning" | "info";
/** `equal` is only ever equality within the compared scope. */
export const DIFFERENCE_LABEL = {
  equal: "Equal within scope",
  changed: "Changed",
  sourceOnly: "Source only",
  targetOnly: "Target only",
  notComparable: "Not comparable",
} satisfies Record<DifferenceKind, string>;
export const DIFFERENCE_TONE = {
  equal: "success",
  changed: "warning",
  sourceOnly: "info",
  targetOnly: "info",
  notComparable: "warning",
} satisfies Record<DifferenceKind, DifferenceTone>;

type IncomparableReason = Extract<
  SchemaCompareFieldSummary,
  { kind: "notComparable" }
>["reason"];
export const INCOMPARABLE_REASON = {
  expressionOutsideSubset:
    "Expression outside the supported scalar grammar. Identical raw text does not establish equality.",
  renderingVersionDifference:
    "Rendered on different server versions. Rendered expressions compare only between matching PostgreSQL 16 versions.",
  externalDependency:
    "Depends on an object outside the compared tables, so its definition is not established here.",
  unknownAccessMethod: "The index access method is not recognized.",
  excludedCounterpart:
    "The counterpart on the other side is excluded from this comparison scope. This is not directional absence.",
  excludedObject:
    "This object is excluded from the comparison scope. No equality or absence is inferred.",
} satisfies Record<IncomparableReason, string>;

type ExclusionReason = Extract<
  SchemaCompareEligibilityDetail,
  { kind: "excluded" }
>["reason"];
export const EXCLUSION_REASON = {
  partitioned: "partitioned table",
  inherited: "inherited table",
  foreign: "foreign table",
  extensionOwned: "extension-owned table",
  otherKind: "not an ordinary table",
} satisfies Record<ExclusionReason, string>;

type ExcludedCategory =
  SchemaCompareMetadataDetail["sourceExcludedCounts"][number]["category"];
export const EXCLUDED_CATEGORY = {
  otherRelations: "Views, materialized views and other relations",
  routines: "Functions and procedures",
  sequences: "Sequences",
  typesAndDomains: "Types and domains",
  policies: "Row security policies",
  grants: "Grants and privileges",
  triggers: "Triggers",
  rules: "Rules",
  extensions: "Extensions",
  databaseObjects: "Database-level objects",
  identitySequenceConfiguration: "Identity sequence configuration",
  storageSecurityOwnershipReplication:
    "Storage, security, ownership and replication settings",
  indexPlacementClusteringReplicaIdentity:
    "Index placement, clustering and replica identity",
} satisfies Record<ExcludedCategory, string>;

/** Never zero or an invented exact total for an incomplete category count. */
export function formatExcludedCount(count: number, complete: boolean) {
  if (complete) return String(count);
  return count > 0 ? `At least ${count}` : "Not compared";
}

type FieldName = SchemaCompareFieldPath["field"];
const FIELD_LABEL = {
  persistence: "persistence",
  comment: "comment",
  position: "position",
  type: "type",
  typeModifier: "type modifier",
  arrayDimensions: "array dimensions",
  nullable: "nullable",
  default: "default",
  generatedKind: "generated kind",
  generatedExpression: "generated expression",
  identity: "identity",
  collation: "collation",
  kind: "kind",
  keys: "keys",
  referencedTable: "referenced table",
  referencedKeys: "referenced keys",
  updateAction: "on update",
  deleteAction: "on delete",
  deleteColumns: "delete columns",
  matchMode: "match mode",
  deferrable: "deferrable",
  initiallyDeferred: "initially deferred",
  validated: "validated",
  noInherit: "no inherit",
  expression: "expression",
  equalityOperators: "equality operators",
  exclusionOperators: "exclusion operators",
  accessMethod: "access method",
  unique: "unique",
  nullsNotDistinct: "nulls not distinct",
  immediate: "immediate",
  keyCount: "key count",
  includedColumns: "included columns",
  predicate: "predicate",
  relationOptions: "relation options",
  valid: "valid",
  ready: "ready",
  live: "live",
  column: "column",
  sortOptions: "sort options",
  opclass: "operator class",
  opclassOptions: "operator class options",
} satisfies Record<FieldName, string>;
export const fieldLabel = (field: FieldName) => FIELD_LABEL[field];

export type FieldPathParts = {
  group: "table" | "column" | "constraint" | "index" | "index key";
  /** Exact identifier, rendered in mono. Null for table-level facts. */
  name: string | null;
  /** Owning constraint of a constraint-backed index, kept visible. */
  owner: string | null;
  /** 1-based key position for index keys. */
  position: number | null;
  field: string;
};
export function describeFieldPath(
  path: SchemaCompareFieldPath,
): FieldPathParts {
  switch (path.kind) {
    case "table":
      return {
        group: "table",
        name: null,
        owner: null,
        position: null,
        field: fieldLabel(path.field),
      };
    case "column":
    case "constraint":
      return {
        group: path.kind,
        name: path.name,
        owner: null,
        position: null,
        field: fieldLabel(path.field),
      };
    case "index":
      return {
        group: "index",
        name: path.name,
        owner: path.owner,
        position: null,
        field: fieldLabel(path.field),
      };
    case "indexKey":
      return {
        group: "index key",
        name: path.name,
        owner: path.owner,
        position: path.position + 1,
        field: fieldLabel(path.field),
      };
  }
}
/** Plain-text path for labels and tests: `index / orders_pkey (constraint orders_pkey) #1 / unique`. */
export function formatFieldPath(path: SchemaCompareFieldPath) {
  const parts = describeFieldPath(path);
  const identity = [
    parts.name,
    parts.owner === null ? null : `(constraint ${parts.owner})`,
    parts.position === null ? null : `#${parts.position}`,
  ]
    .filter((part) => part !== null)
    .join(" ");
  return [parts.group, identity || null, parts.field]
    .filter((part) => part !== null)
    .join(" / ");
}

export const SIDE_LABEL = { source: "Source", target: "Target" } as const;
const LIMIT_LABEL = {
  inventory: "inventory",
  tables: "table count",
  childFacts: "column, constraint and index",
  fieldBytes: "field size",
  endpointBytes: "endpoint size",
  resultBytes: "result size",
  pageBytes: "page size",
  pageItems: "page item",
  allocation: "memory",
} as const;
/**
 * Concise, actionable text. `unavailable` never claims to know whether the
 * result expired, was released or was invalidated by a connection change.
 */
export function formatSchemaCompareFailure(failure: SchemaCompareFailure) {
  switch (failure.kind) {
    case "busy":
      return "Another comparison is using one of these endpoints, or both comparison slots are active. Wait for the active jobs below to finish.";
    case "limitExceeded":
      return `The ${LIMIT_LABEL[failure.limit]} limit was exceeded. No complete result was produced.`;
    case "unsupportedVersion":
      return `${SIDE_LABEL[failure.side]} is running PostgreSQL ${failure.version}. Schema comparison supports PostgreSQL 16 endpoints only.`;
    case "unsupportedEngine":
      return `${SIDE_LABEL[failure.side]} is not a PostgreSQL connection.`;
    case "unavailable":
      return "This comparison is unavailable. It may have expired, been dismissed or been invalidated by a connection change.";
    case "invalidRequest":
      return "The request was rejected as invalid. Check both connections and schema names.";
    case "captureChanged":
      // Native folds a catalog race and a lock wait that timed out twice.
      return "Definitions changed, or a table stayed locked by another session, while they were being read. Run a new comparison to capture the current state.";
    case "cancelled":
      return "Comparison cancelled. No result was produced.";
    case "deadlineExceeded":
      return "The comparison did not finish within its time limit. No complete result was produced.";
    case "transport":
      return "Unable to reach the native comparison service. Retry observation before starting another comparison.";
    case "invalidResponse":
      return "The native response could not be validated. Retry observation; the job may still exist.";
  }
}

const NO_RESULT = "No result was produced.";
/**
 * Text for a job that ended in failure. A failed job cannot have expired or
 * been dismissed, so `unavailable` here names the read failures the native
 * backend folds together without claiming to know which one occurred.
 */
export function formatSchemaCompareJobFailure(failure: SchemaCompareFailure) {
  if (failure.kind === "unavailable") {
    return `An endpoint could not be read. The schema may not exist, the connection may lack catalog privileges, or it may have been lost or invalidated while reading. ${NO_RESULT}`;
  }
  const text = formatSchemaCompareFailure(failure);
  return text.endsWith("result was produced.") ? text : `${text} ${NO_RESULT}`;
}

export function formatServerVersion(version: string) {
  return version.startsWith("PostgreSQL") ? version : `PostgreSQL ${version}`;
}
export function formatByteRange(chunk: { offset: number; nextOffset: number }) {
  return `bytes ${chunk.offset}–${chunk.nextOffset}`;
}
/** `1–100 of N` positions from an explicit server page; never a derived total. */
export function formatPagePosition(
  offset: number,
  length: number,
  total: number,
) {
  if (length === 0) return `0 of ${total}`;
  return `${offset + 1}–${offset + length} of ${total}`;
}
