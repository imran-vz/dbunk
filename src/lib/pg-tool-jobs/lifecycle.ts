import { createStore } from "zustand/vanilla";

import { requestConfirm } from "@/lib/confirm";
import {
  isPgTransferJobActive,
  pgTransferClient,
} from "@/lib/pg-transfer/client";
import { pgTransferObserver } from "@/lib/pg-transfer/observer";
import { isTauri } from "@/lib/tauri";

import { isPgToolJobActive, pgToolClient } from "./client";
import { pgToolObserver } from "./observer";

// Conservative app-wide review revision fences native PostgreSQL file reviews,
// same-ID retargeting, and global credential/bastion changes.
export const pgToolReview = createStore(() => ({ revision: 0, closing: 0 }));

/** Claim before asynchronous inspection, and keep new submissions out until finish. */
export async function preparePgToolFence(
  action: string,
  connectionId: string | null = null,
): Promise<(() => void) | null> {
  pgToolReview.setState((s) => ({
    revision: s.revision + 1,
    closing: s.closing + 1,
  }));
  let finished = false;
  const finish = () => {
    if (finished) return;
    finished = true;
    pgToolReview.setState((s) => ({
      revision: s.revision + 1,
      closing: s.closing - 1,
    }));
    if (isTauri()) {
      void pgToolObserver.refresh();
      void pgTransferObserver.refresh();
    }
  };
  if (!isTauri()) return finish;
  let message: string | null = null;
  const [toolResult, transferResult] = await Promise.allSettled([
    pgToolClient.list(connectionId),
    pgTransferClient.list(connectionId),
  ]);
  if (
    toolResult.status === "rejected" ||
    transferResult.status === "rejected"
  ) {
    message =
      "Active PostgreSQL file jobs could not be checked. Continuing may cancel work, including jobs on other connections for global changes.";
  } else {
    const tools = toolResult.value.filter(isPgToolJobActive).length;
    const transfers = transferResult.value.filter(isPgTransferJobActive).length;
    if (tools || transfers) {
      const parts = [
        tools ? `${tools} backup/restore job${tools === 1 ? "" : "s"}` : null,
        transfers
          ? `${transfers} CSV transfer${transfers === 1 ? "" : "s"}`
          : null,
      ].filter((part): part is string => Boolean(part));
      message = `${action} cancels ${parts.join(" and ")}. Cancellation cannot undo a restore or import that already committed.`;
    }
  }
  if (
    message &&
    !(await requestConfirm({
      title: `${action}?`,
      message,
      confirmLabel: `${action} and cancel jobs`,
      cancelLabel: "Keep editing",
      danger: true,
    }))
  ) {
    finish();
    return null;
  }
  return finish;
}
