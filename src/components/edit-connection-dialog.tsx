/**
 * Edit dialog — chrome wrapper around the unified `ConnectionForm`
 * (ADR-0012). All field rendering, validation, and per-engine
 * variant construction lives in `ConnectionForm`; this file owns
 * only the AlertDialog presentation and the open/close hand-off.
 *
 * The engine picker is disabled inside `ConnectionForm` when
 * `mode === "edit"`, so this dialog cannot mutate a connection's
 * engine — changing engine on a tagged-union record is a
 * delete-and-recreate.
 */

import { ConnectionForm } from "@/components/connection-form";
import {
  AlertDialog,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogHeader,
  AlertDialogTitle,
} from "@/components/ui/alert-dialog";
import type { Connection } from "@/lib/store";

interface EditConnectionDialogProps {
  connection: Connection | null;
  open: boolean;
  onOpenChange: (open: boolean) => void;
}

export function EditConnectionDialog({
  connection,
  open,
  onOpenChange,
}: EditConnectionDialogProps) {
  return (
    <AlertDialog open={open} onOpenChange={onOpenChange}>
      <AlertDialogContent className="flex max-h-[88vh] w-[26rem] max-w-[26rem] flex-col gap-0 overflow-hidden rounded-lg border border-border-subtle bg-surface-window p-0 sm:max-w-[26rem]">
        <AlertDialogHeader className="border-b border-border-subtle px-4 py-3">
          <AlertDialogTitle className="text-sm font-semibold">
            Edit connection
          </AlertDialogTitle>
          <AlertDialogDescription className="text-2xs text-text-muted">
            Update the connection details for{" "}
            {connection?.name ?? "this connection"}.
          </AlertDialogDescription>
        </AlertDialogHeader>
        <div className="flex min-h-0 flex-1 flex-col">
          {connection ? (
            <ConnectionForm
              mode="edit"
              connection={connection}
              onSaved={() => onOpenChange(false)}
              onCancel={() => onOpenChange(false)}
            />
          ) : null}
        </div>
      </AlertDialogContent>
    </AlertDialog>
  );
}
