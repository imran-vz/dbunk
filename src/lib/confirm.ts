/**
 * App confirmation service (DESIGN-SYSTEM §6.4) — the themed
 * replacement for `window.confirm` / `window.prompt`. Callers (UI or
 * store actions) await `requestConfirm` / `requestPrompt`; the
 * `ConfirmDialogHost` mounted in the app shell renders the queued
 * request on the AlertDialog/Dialog primitives and resolves it.
 *
 * Destructive-dialog spec: pass `danger: true` and name the affected
 * objects in `detail` — the confirm button renders in danger style and
 * is never the default-focused control.
 */

export type ConfirmRequest = {
  title: string;
  message: string;
  /** Named objects / consequences, rendered emphasized under the message. */
  detail?: string;
  confirmLabel?: string;
  cancelLabel?: string;
  /** Danger styling for destructive actions (§6.4). */
  danger?: boolean;
};

export type PromptRequest = {
  title: string;
  message?: string;
  placeholder?: string;
  defaultValue?: string;
  confirmLabel?: string;
};

export type PendingAppDialog =
  | (ConfirmRequest & {
      id: string;
      kind: "confirm";
      resolve: (confirmed: boolean) => void;
    })
  | (PromptRequest & {
      id: string;
      kind: "prompt";
      resolve: (value: string | null) => void;
    });

const listeners = new Set<() => void>();
const queue: PendingAppDialog[] = [];
let current: PendingAppDialog | null = null;

const emit = () => {
  current = queue[0] ?? null;
  for (const listener of listeners) listener();
};

export const subscribeAppDialog = (listener: () => void) => {
  listeners.add(listener);
  return () => listeners.delete(listener);
};

export const getAppDialog = () => current;

export const requestConfirm = (request: ConfirmRequest): Promise<boolean> =>
  new Promise((resolve) => {
    queue.push({
      ...request,
      id: crypto.randomUUID(),
      kind: "confirm",
      resolve,
    });
    emit();
  });

export const requestPrompt = (request: PromptRequest): Promise<string | null> =>
  new Promise((resolve) => {
    queue.push({
      ...request,
      id: crypto.randomUUID(),
      kind: "prompt",
      resolve,
    });
    emit();
  });

export const resolveAppDialog = (value: boolean | string | null) => {
  const pending = queue.shift();
  if (!pending) return;
  if (pending.kind === "confirm") {
    pending.resolve(value === true);
  } else {
    // oxlint-disable-next-line anti-slop/no-runtime-typeof -- resolver accepts both dialog kinds' values.
    pending.resolve(typeof value === "string" ? value : null);
  }
  emit();
};
