import {
  decodeDatabaseError,
  decodeTimeout,
  decodeTlsFailed,
  isRecord,
} from "@/lib/decode-transport-error";
import {
  decodeStatementSummaries,
  formatSharedTransportError,
} from "@/lib/safety-policy";
/* oxlint-disable anti-slop/no-runtime-typeof, anti-slop/no-unknown-parameters, anti-slop/no-unsafe-dictionary-type -- Session errors arrive as unstructured invoke rejections and must be decoded at this boundary. */
import type {
  QuerySessionError,
  QueryTransactionStatus,
} from "@/lib/store/types";

const isTransactionStatus = (value: unknown): value is QueryTransactionStatus =>
  value === "idle" ||
  value === "active" ||
  value === "failed" ||
  value === "unknown";

/** Decode, and structurally validate, an untrusted Tauri command rejection. */
export function decodeQuerySessionError(error: unknown): QuerySessionError {
  if (!isRecord(error) || typeof error.kind !== "string") {
    return { kind: "connectionLost" };
  }
  switch (error.kind) {
    case "unsupportedEngine":
    case "connectionClosing":
    case "sessionNotFound":
    case "ownerMismatch":
    case "executionInProgress":
    case "invalidSequence":
    case "transactionObserverUnavailable":
    case "connectionLost":
      return { kind: error.kind };
    case "sessionLimitReached":
      return typeof error.limit === "string"
        ? { kind: "sessionLimitReached", limit: error.limit }
        : { kind: "connectionLost" };
    case "invalidTransactionTransition":
      return isTransactionStatus(error.status) &&
        typeof error.attemptedAction === "string" &&
        Array.isArray(error.allowedActions) &&
        error.allowedActions.every((action) => typeof action === "string")
        ? {
            kind: "invalidTransactionTransition",
            status: error.status,
            attemptedAction: error.attemptedAction,
            allowedActions: error.allowedActions,
          }
        : { kind: "connectionLost" };
    case "transactionStateUnknown":
      return typeof error.canRecheck === "boolean"
        ? { kind: "transactionStateUnknown", canRecheck: error.canRecheck }
        : { kind: "connectionLost" };
    case "policyBlocked":
      return typeof error.reason === "string"
        ? { kind: "policyBlocked", reason: error.reason }
        : { kind: "connectionLost" };
    case "policyNeedsConfirmation": {
      const statements = decodeStatementSummaries(error.statements);
      return statements
        ? { kind: "policyNeedsConfirmation", statements }
        : { kind: "connectionLost" };
    }
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

export function isQuerySessionError(
  error: unknown,
): error is QuerySessionError {
  if (!isRecord(error) || typeof error.kind !== "string") return false;
  return decodeQuerySessionError(error).kind === error.kind;
}

export function formatQuerySessionError(error: QuerySessionError): string {
  switch (error.kind) {
    case "unsupportedEngine":
      return "Query sessions are only available for PostgreSQL.";
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
    case "connectionClosing":
    case "connectionLost":
    case "tlsFailed":
    case "policyBlocked":
    case "policyNeedsConfirmation":
    case "timeout":
    case "database":
      return formatSharedTransportError(error);
  }
}
