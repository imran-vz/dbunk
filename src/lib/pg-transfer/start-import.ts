import {
  confirmWriteStatements,
  type SafetyConfirmationConnection,
} from "@/lib/safety-confirmation";

import {
  decodePgTransferError,
  type PgTransferStartImportRequest,
} from "./client";
import { pgTransferObserver } from "./observer";

/** Confirmation retries only the frozen inspection token and source-index mapping. */
export async function startReviewedPgCsvImport(
  payload: Omit<PgTransferStartImportRequest, "confirmed">,
  connection: SafetyConfirmationConnection,
  isCurrent: () => boolean,
) {
  const frozen = {
    inspectionToken: payload.inspectionToken,
    mapping: payload.mapping.map((entry) => ({ ...entry })),
    confirmed: false,
  };
  if (!isCurrent()) throw { kind: "connectionClosing" };
  try {
    return await pgTransferObserver.startImport(frozen);
  } catch (error) {
    const refusal = decodePgTransferError(error);
    if (refusal.kind !== "policyNeedsConfirmation") throw error;
    if (!(await confirmWriteStatements(connection, refusal.statements))) {
      throw { kind: "cancelled" };
    }
    if (!isCurrent()) throw { kind: "connectionClosing" };
    return pgTransferObserver.startImport({ ...frozen, confirmed: true });
  }
}
