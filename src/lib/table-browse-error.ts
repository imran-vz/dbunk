/* oxlint-disable anti-slop/no-runtime-typeof, anti-slop/no-unknown-parameters -- Browse errors arrive as unstructured invoke rejections and must be decoded at this boundary. */
import type { TableBrowseError } from "@/lib/table-browse";

const TABLE_BROWSE_ERROR_KINDS: ReadonlySet<string> = new Set([
  "unsupportedEngine",
  "unknownColumn",
  "invalidFilter",
  "invalidSort",
  "invalidCursor",
  "superseded",
  "cancelled",
  "connectionClosing",
  "connectionLost",
  "timeout",
  "database",
]);

export function isTableBrowseError(error: unknown): error is TableBrowseError {
  if (error === null || typeof error !== "object") return false;
  if (!("kind" in error) || typeof error.kind !== "string") return false;
  return TABLE_BROWSE_ERROR_KINDS.has(error.kind);
}

export function decodeTableBrowseError(error: unknown): TableBrowseError {
  if (isTableBrowseError(error)) return error;
  return { kind: "connectionLost" };
}

export function formatTableBrowseError(error: TableBrowseError): string {
  switch (error.kind) {
    case "unsupportedEngine":
      return "Server-backed browsing is only available for PostgreSQL.";
    case "unknownColumn":
      return `Unknown column ${error.column}.`;
    case "invalidFilter":
      return `Invalid filter (${error.reason}).`;
    case "invalidSort":
      return `Invalid sort on ${error.column}.`;
    case "invalidCursor":
      return "The page cursor is no longer valid.";
    case "superseded":
      return "This browse request was superseded.";
    case "cancelled":
      return "Browse cancelled.";
    case "connectionClosing":
      return "The connection is closing.";
    case "connectionLost":
      return "The database connection was lost.";
    case "timeout":
      return `Timed out during ${error.operation}.`;
    case "database":
      return error.code ? `${error.code}: ${error.message}` : error.message;
  }
}
