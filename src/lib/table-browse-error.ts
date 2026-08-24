import {
  decodeDatabaseError,
  decodeTimeout,
  decodeTlsFailed,
  isRecord,
} from "@/lib/decode-transport-error";
import { formatSharedTransportError } from "@/lib/safety-policy";
/* oxlint-disable anti-slop/no-runtime-typeof, anti-slop/no-unknown-parameters, anti-slop/no-unsafe-dictionary-type -- Browse errors arrive as unstructured invoke rejections and must be decoded at this boundary. */
import type { TableBrowseError } from "@/lib/table-browse";

/** Decode, and structurally validate, an untrusted Tauri command rejection. */
export function decodeTableBrowseError(error: unknown): TableBrowseError {
  if (!isRecord(error) || typeof error.kind !== "string") {
    return { kind: "connectionLost" };
  }
  switch (error.kind) {
    case "unsupportedEngine":
    case "invalidCursor":
    case "superseded":
    case "cancelled":
    case "connectionClosing":
    case "connectionLost":
      return { kind: error.kind };
    case "unknownColumn":
      return typeof error.column === "string"
        ? { kind: "unknownColumn", column: error.column }
        : { kind: "connectionLost" };
    case "invalidFilter":
      return typeof error.reason === "string"
        ? { kind: "invalidFilter", reason: error.reason }
        : { kind: "connectionLost" };
    case "invalidSort":
      return typeof error.column === "string"
        ? { kind: "invalidSort", column: error.column }
        : { kind: "connectionLost" };
    case "tlsFailed":
      return decodeTlsFailed(error) ?? { kind: "connectionLost" };
    case "timeout":
      return decodeTimeout(error) ?? { kind: "connectionLost" };
    case "database":
      return decodeDatabaseError(error) ?? { kind: "connectionLost" };
    default:
      return { kind: "connectionLost" };
  }
}

export function isTableBrowseError(error: unknown): error is TableBrowseError {
  if (!isRecord(error) || typeof error.kind !== "string") return false;
  return decodeTableBrowseError(error).kind === error.kind;
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
    case "connectionLost":
    case "tlsFailed":
    case "timeout":
    case "database":
      return formatSharedTransportError(error);
  }
}
