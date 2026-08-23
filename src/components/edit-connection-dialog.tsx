/**
 * Edit dialog — chrome wrapper around the unified `ConnectionForm`
 * (ADR-0012). All field rendering, validation, and per-engine
 * variant construction lives in `ConnectionForm`; this file owns
 * only the Dialog presentation and the open/close hand-off.
 *
 * The engine picker is disabled inside `ConnectionForm` when
 * `mode === "edit"`, so this dialog cannot mutate a connection's
 * engine — changing engine on a tagged-union record is a
 * delete-and-recreate.
 */

import { ConnectionForm } from "@/components/connection-form";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
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
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent size="md">
        <DialogHeader>
          <DialogTitle>Edit connection</DialogTitle>
          <DialogDescription className="text-xs text-text-muted">
            Update the connection details for{" "}
            {connection?.name ?? "this connection"}.
          </DialogDescription>
        </DialogHeader>
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
      </DialogContent>
    </Dialog>
  );
}
