/* oxlint-disable anti-slop/no-unknown-parameters -- Tauri rejections are validated by the comparison error schema at this boundary. */
import { schemaCompareError, type SchemaCompareError } from "./protocol";
import type { SchemaCompareStatus } from "./protocol";

/**
 * A native comparison error, or one of the two frontend-only observation
 * failures: a lost/rejected transport call and a fulfilled response that
 * failed client validation. Neither proves anything about native state.
 */
export type SchemaCompareFailure =
  | SchemaCompareError
  | { kind: "transport" }
  | { kind: "invalidResponse" };

/** Unknown rejections never echo their payload: it may contain identifiers. */
export function decodeSchemaCompareFailure(
  value: unknown,
): SchemaCompareFailure {
  const native = schemaCompareError.safeParse(value).data;
  if (native) return native;
  return value instanceof Error
    ? { kind: "invalidResponse" }
    : { kind: "transport" };
}

export const isSchemaCompareActive = (status: SchemaCompareStatus) =>
  status.phase !== "completed" &&
  status.phase !== "cancelled" &&
  status.phase !== "failed";
