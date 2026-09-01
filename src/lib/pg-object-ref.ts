/* oxlint-disable anti-slop/no-runtime-typeof, anti-slop/no-unknown-parameters -- Persisted and transported object refs are unstructured until decoded here. */
import { isNullableString, isRecord } from "@/lib/decode-transport-error";
import type { PgObjectKind, PgObjectRef } from "@/lib/store/types";

export const PG_OBJECT_KINDS = [
  "schema",
  "table",
  "view",
  "materialized-view",
  "foreign-table",
  "sequence",
  "function",
  "procedure",
  "aggregate",
  "type",
  "domain",
  "extension",
] as const satisfies readonly PgObjectKind[];

export const isPgObjectKind = (value: unknown): value is PgObjectKind =>
  typeof value === "string" &&
  PG_OBJECT_KINDS.some((candidate) => candidate === value);

/** Validate persisted or transported object identity field-by-field. */
export function validatePgObjectRef(value: unknown): PgObjectRef | null {
  if (!isRecord(value)) return null;
  if (
    !isPgObjectKind(value.kind) ||
    !isNullableString(value.schema) ||
    typeof value.name !== "string" ||
    value.name.trim().length === 0 ||
    !isNullableString(value.identityArgs)
  ) {
    return null;
  }

  if (value.kind === "schema") {
    return value.schema === null && value.identityArgs === null
      ? {
          kind: "schema",
          schema: null,
          name: value.name,
          identityArgs: null,
        }
      : null;
  }

  if (value.schema === null || value.schema.trim().length === 0) return null;
  if (
    value.kind === "function" ||
    value.kind === "procedure" ||
    value.kind === "aggregate"
  ) {
    return value.identityArgs === null
      ? null
      : {
          kind: value.kind,
          schema: value.schema,
          name: value.name,
          identityArgs: value.identityArgs,
        };
  }

  return value.identityArgs === null
    ? {
        kind: value.kind,
        schema: value.schema,
        name: value.name,
        identityArgs: null,
      }
    : null;
}

/** Null and empty schema/identity spellings name the same PostgreSQL object. */
export const canonicalPgObjectRefKey = (reference: PgObjectRef): string =>
  JSON.stringify([
    reference.kind,
    reference.schema ?? "",
    reference.name,
    reference.identityArgs ?? "",
  ]);

/** Per-connection description-cache key for an object reference. */
export const pgObjectDescriptionKey = (
  connectionId: string,
  reference: PgObjectRef,
): string => `${connectionId}:${canonicalPgObjectRefKey(reference)}`;

/** Per-connection mutual-exclusion key for object-DDL applies. */
export const pgObjectDdlApplyKey = (connectionId: string): string =>
  connectionId;

/** Recover typed identity from a key emitted by canonicalPgObjectRefKey. */
export function parseCanonicalPgObjectRefKey(key: string): PgObjectRef | null {
  try {
    const identity: unknown = JSON.parse(key);
    if (
      !Array.isArray(identity) ||
      identity.length !== 4 ||
      !identity.every((part): part is string => typeof part === "string")
    ) {
      return null;
    }
    const [kind, schema, name, identityArgs] = identity;
    if (!isPgObjectKind(kind)) return null;

    const reference = validatePgObjectRef({
      kind,
      schema: kind === "schema" ? null : schema,
      name,
      identityArgs:
        kind === "function" || kind === "procedure" || kind === "aggregate"
          ? identityArgs
          : null,
    });
    return reference && canonicalPgObjectRefKey(reference) === key
      ? reference
      : null;
  } catch {
    return null;
  }
}

export function displayPgObjectName(reference: PgObjectRef): string {
  const routineSuffix =
    reference.identityArgs === null ? "" : `(${reference.identityArgs})`;
  const name = `${reference.name}${routineSuffix}`;
  return reference.schema ? `${reference.schema}.${name}` : name;
}
