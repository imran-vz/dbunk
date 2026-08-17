import { IconTrash } from "@tabler/icons-react";

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
import { type Connection, useAppStore } from "@/lib/store";

interface DeleteConnectionDialogProps {
  connection: Connection | null;
  open: boolean;
  onOpenChange: (open: boolean) => void;
}

export function DeleteConnectionDialog({
  connection,
  open,
  onOpenChange,
}: DeleteConnectionDialogProps) {
  const deleteConnection = useAppStore((state) => state.deleteConnection);

  const handleDelete = async () => {
    if (!connection) return;
    await deleteConnection(connection.id);
    onOpenChange(false);
  };

  return (
    <AlertDialog open={open} onOpenChange={onOpenChange}>
      <AlertDialogContent>
        <AlertDialogHeader>
          <AlertDialogTitle>Delete connection?</AlertDialogTitle>
          <AlertDialogDescription>
            This will permanently delete the connection{" "}
            <span className="font-medium text-foreground">
              {connection?.name ?? "this connection"}
            </span>{" "}
            and remove all saved credentials. This action cannot be undone.
          </AlertDialogDescription>
        </AlertDialogHeader>
        <AlertDialogFooter>
          <AlertDialogCancel>Cancel</AlertDialogCancel>
          <AlertDialogAction variant="destructive" onClick={handleDelete}>
            <IconTrash className="size-4" />
            Delete connection
          </AlertDialogAction>
        </AlertDialogFooter>
      </AlertDialogContent>
    </AlertDialog>
  );
}
