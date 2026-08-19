import type { QueryTransactionStatus } from "@/lib/store/types";

export const CLOSE_QUERY_SESSION_CONFIRM_MESSAGE =
  "Close this query session? Its active or unresolved transaction will be rolled back.";

export function confirmCloseQuerySession(
  transactionStatus: QueryTransactionStatus | undefined,
): boolean {
  if (transactionStatus === undefined || transactionStatus === "idle") {
    return true;
  }
  return window.confirm(CLOSE_QUERY_SESSION_CONFIRM_MESSAGE);
}
