import { createStore } from "zustand/vanilla";

import { requestConfirm } from "@/lib/confirm";
import { isTauri } from "@/lib/tauri";

import { isPgToolJobActive, pgToolClient } from "./client";
import { pgToolObserver } from "./observer";

// Conservative app-wide review revision also fences same-ID retargeting and
// global credentials/bastion changes. It is never persisted or reset on deletion.
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
    if (isTauri()) void pgToolObserver.refresh();
  };
  if (!isTauri()) return finish;
  let message: string | null = null;
  try {
    const active = (await pgToolClient.list(connectionId)).filter(
      isPgToolJobActive,
    );
    if (active.length)
      message = `${action} cancels ${active.length} active backup/restore job${active.length === 1 ? "" : "s"}. Cancellation cannot undo a restore that already committed.`;
  } catch {
    message =
      "Active backup/restore jobs could not be checked. Continuing may cancel work, including jobs on other connections for global changes.";
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
