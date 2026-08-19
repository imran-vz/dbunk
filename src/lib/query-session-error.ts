/* oxlint-disable anti-slop/no-runtime-typeof, anti-slop/no-unknown-parameters -- Session errors arrive as unstructured invoke rejections and must be decoded at this boundary. */
import type { QuerySessionError } from "@/lib/store/types";

const QUERY_SESSION_ERROR_KINDS: ReadonlySet<string> = new Set([
  "unsupportedEngine",
  "connectionClosing",
  "sessionLimitReached",
  "sessionNotFound",
  "ownerMismatch",
  "executionInProgress",
  "invalidSequence",
  "invalidTransactionTransition",
  "transactionStateUnknown",
  "transactionObserverUnavailable",
  "connectionLost",
  "timeout",
  "database",
]);

export function isQuerySessionError(
  error: unknown,
): error is QuerySessionError {
  if (error === null || typeof error !== "object") return false;
  if (!("kind" in error) || typeof error.kind !== "string") return false;
  return QUERY_SESSION_ERROR_KINDS.has(error.kind);
}

export function formatQuerySessionError(error: QuerySessionError): string {
  switch (error.kind) {
    case "unsupportedEngine":
      return "Query sessions are only available for PostgreSQL.";
    case "connectionClosing":
      return "The connection is closing.";
    case "sessionLimitReached":
      return `Query session limit reached (${error.limit}).`;
    case "sessionNotFound":
      return "Query session not found.";
    case "ownerMismatch":
      return "Query session owner mismatch.";
    case "executionInProgress":
      return "A query is already running on this session.";
    case "invalidSequence":
      return "Query session lost a protocol event.";
    case "invalidTransactionTransition":
      return `Cannot ${error.attemptedAction} while the transaction is ${error.status}.`;
    case "transactionStateUnknown":
      return error.canRecheck
        ? "Transaction state is unknown. Recheck and try again."
        : "Transaction state is unknown.";
    case "transactionObserverUnavailable":
      return "Transaction observer is unavailable.";
    case "connectionLost":
      return "The database connection was lost.";
    case "timeout":
      return `Timed out during ${error.operation}.`;
    case "database":
      return error.code ? `${error.code}: ${error.message}` : error.message;
  }
}
