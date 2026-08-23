/**
 * ConfirmDialogHost — renders the app confirmation service's queue
 * (`@/lib/confirm`) on the dialog primitives, replacing every
 * `window.confirm` / `window.prompt` (DESIGN-SYSTEM §6.4).
 *
 * Destructive confirms render the danger button as the non-default
 * control: Cancel holds initial focus, so Enter never destroys.
 */

import { useEffect, useState, useSyncExternalStore } from "react";

import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from "@/components/ui/alert-dialog";
import { Input } from "@/components/ui/input";
import {
  getAppDialog,
  resolveAppDialog,
  subscribeAppDialog,
} from "@/lib/confirm";

export function ConfirmDialogHost() {
  const request = useSyncExternalStore(
    subscribeAppDialog,
    getAppDialog,
    () => null,
  );
  const [promptValue, setPromptValue] = useState("");
  const requestId = request?.id;
  useEffect(() => {
    setPromptValue(
      request?.kind === "prompt" ? (request.defaultValue ?? "") : "",
    );
    // Reset per queued request, not per render.
    // oxlint-disable-next-line react-hooks/exhaustive-deps
  }, [requestId]);

  if (!request) return null;

  if (request.kind === "prompt") {
    return (
      <AlertDialog
        open
        onOpenChange={(open) => {
          if (!open) resolveAppDialog(request.id, null);
        }}
      >
        <AlertDialogContent size="sm">
          <AlertDialogHeader>
            <AlertDialogTitle>{request.title}</AlertDialogTitle>
            {request.message ? (
              <AlertDialogDescription>{request.message}</AlertDialogDescription>
            ) : null}
          </AlertDialogHeader>
          <form
            onSubmit={(event) => {
              event.preventDefault();
              resolveAppDialog(request.id, promptValue);
            }}
          >
            <Input
              // oxlint-disable-next-line jsx-a11y/no-autofocus -- single-field prompt dialog; focus belongs in the input.
              autoFocus
              value={promptValue}
              placeholder={request.placeholder}
              onChange={(event) => setPromptValue(event.target.value)}
            />
            <AlertDialogFooter className="mt-4">
              <AlertDialogCancel
                onClick={() => resolveAppDialog(request.id, null)}
              >
                Cancel
              </AlertDialogCancel>
              <AlertDialogAction type="submit">
                {request.confirmLabel ?? "Save"}
              </AlertDialogAction>
            </AlertDialogFooter>
          </form>
        </AlertDialogContent>
      </AlertDialog>
    );
  }

  return (
    <AlertDialog
      open
      onOpenChange={(open) => {
        if (!open) resolveAppDialog(request.id, false);
      }}
    >
      <AlertDialogContent size="sm">
        <AlertDialogHeader>
          <AlertDialogTitle>{request.title}</AlertDialogTitle>
          <AlertDialogDescription>{request.message}</AlertDialogDescription>
        </AlertDialogHeader>
        {request.detail ? (
          <div className="border-y border-border-subtle py-2 font-mono text-xs text-foreground">
            {request.detail}
          </div>
        ) : null}
        <AlertDialogFooter>
          <AlertDialogCancel
            // oxlint-disable-next-line jsx-a11y/no-autofocus -- §6.4: Cancel is the default control so Enter never destroys.
            autoFocus
            onClick={() => resolveAppDialog(request.id, false)}
          >
            {request.cancelLabel ?? "Cancel"}
          </AlertDialogCancel>
          <AlertDialogAction
            variant={request.danger ? "destructive" : "default"}
            onClick={() => resolveAppDialog(request.id, true)}
          >
            {request.confirmLabel ?? "Confirm"}
          </AlertDialogAction>
        </AlertDialogFooter>
      </AlertDialogContent>
    </AlertDialog>
  );
}
