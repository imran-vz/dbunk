import {
  confirmWriteStatements,
  type SafetyConfirmationConnection,
} from "@/lib/safety-confirmation";

import { decodePgToolError, type PgRestoreRequest } from "./client";
import { pgToolObserver } from "./observer";

/** The confirmed retry carries exactly the reviewed request, never fresh form data. */
export async function startReviewedPgRestore(
  payload: Omit<PgRestoreRequest, "confirmed">,
  connection: SafetyConfirmationConnection,
  isCurrent: () => boolean,
) {
  const frozen = { ...payload, confirmed: false };
  if (!isCurrent()) throw { kind: "connectionClosing" };
  try {
    return await pgToolObserver.startRestore(frozen);
  } catch (error) {
    const refusal = decodePgToolError(error);
    if (refusal.kind !== "policyNeedsConfirmation") throw error;
    if (!(await confirmWriteStatements(connection, refusal.statements)))
      throw { kind: "cancelled" };
    if (!isCurrent()) throw { kind: "connectionClosing" };
    return pgToolObserver.startRestore({ ...frozen, confirmed: true });
  }
}
