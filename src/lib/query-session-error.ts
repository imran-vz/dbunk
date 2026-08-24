/* oxlint-disable anti-slop/no-runtime-typeof, anti-slop/no-unknown-parameters, anti-slop/no-unsafe-dictionary-type -- Session errors arrive as unstructured invoke rejections and must be decoded at this boundary. */
import { decodeStatementSummaries } from "@/lib/safety-policy";
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
  "policyBlocked",
  "policyNeedsConfirmation",
  "timeout",
  "database",
]);

const isRecord = (value: unknown): value is Record<string, unknown> =>
  value !== null && typeof value === "object";

export function isQuerySessionError(
  error: unknown,
): error is QuerySessionError {
  if (!isRecord(error) || typeof error.kind !== "string") {
    return false;
  }
  if (error.kind === "policyBlocked") {
    return typeof error.reason === "string";
  }
  if (error.kind === "policyNeedsConfirmation") {
    return decodeStatementSummaries(error.statements) !== null;
  }
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
    case "policyBlocked":
      return `${error.reason} Edit the connection to unlock writes.`;
    case "policyNeedsConfirmation":
      return "The connection safety policy requires confirmation.";
    case "timeout":
      return `Timed out during ${error.operation}.`;
    case "database":
      return error.code ? `${error.code}: ${error.message}` : error.message;
  }
}
