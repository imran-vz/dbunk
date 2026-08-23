import { requestConfirm } from "@/lib/confirm";
import type { QueryTransactionStatus } from "@/lib/store/types";

export const CLOSE_QUERY_SESSION_CONFIRM_MESSAGE =
  "Its active or unresolved transaction will be rolled back.";

export async function confirmCloseQuerySession(
  transactionStatus: QueryTransactionStatus | undefined,
): Promise<boolean> {
  if (transactionStatus === undefined || transactionStatus === "idle") {
    return true;
  }
  return requestConfirm({
    title: "Close this query session?",
    message: CLOSE_QUERY_SESSION_CONFIRM_MESSAGE,
    confirmLabel: "Close session",
    danger: true,
  });
}
