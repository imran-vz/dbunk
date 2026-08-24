import {
  IconAlertCircle,
  IconCopy,
  IconDatabaseOff,
  IconDotsVertical,
  IconLink,
  IconPencil,
  IconTrash,
} from "@tabler/icons-react";
import { toast } from "sonner";

import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { buildConnectionUri } from "@/lib/connection-uri";
import { type Connection, useAppStore } from "@/lib/store";
import { cn } from "@/lib/utils";

export type ConnectionActionsProps = {
  connection: Connection;
  onConnect: () => void;
  onDisconnect: () => void;
  onEdit: () => void;
  onDelete: () => void;
};

export function ConnectionActionsDropdown(props: ConnectionActionsProps) {
  const duplicateConnection = useAppStore((state) => state.duplicateConnection);
  const isDisconnected = props.connection.status === "Disconnected";
  // Secret-free by contract; SQLite/ClickHouse have no canonical URI
  // and hide the action instead of failing it (Plan 010).
  const uri = buildConnectionUri(props.connection);

  const handleDuplicate = async () => {
    const copy = await duplicateConnection(props.connection.id);
    if (copy) {
      toast.success(`Duplicated as “${copy.name}”`);
    } else {
      toast.error("Failed to duplicate the connection.");
    }
  };

  const handleCopyUri = async () => {
    if (!uri.ok) return;
    try {
      await navigator.clipboard.writeText(uri.uri);
      toast.success("Connection URI copied — password not included.");
    } catch {
      toast.error("Couldn't write to the clipboard.");
    }
  };

  return (
    <DropdownMenu>
      <DropdownMenuTrigger
        aria-label="Connection actions"
        className="rounded-md p-1 text-text-muted hover:bg-surface-panel-elevated hover:text-foreground"
      >
        <IconDotsVertical className="size-3.5" />
      </DropdownMenuTrigger>
      <DropdownMenuContent align="end">
        <DropdownMenuItem onClick={props.onConnect}>Connect</DropdownMenuItem>
        <DropdownMenuItem
          disabled={isDisconnected}
          onClick={props.onDisconnect}
        >
          <IconDatabaseOff className="size-3.5" />
          Disconnect
        </DropdownMenuItem>
        <DropdownMenuItem onClick={props.onEdit}>
          <IconPencil className="size-3.5" />
          Edit…
        </DropdownMenuItem>
        <DropdownMenuSeparator />
        <DropdownMenuItem onClick={() => void handleDuplicate()}>
          <IconCopy className="size-3.5" />
          Duplicate
        </DropdownMenuItem>
        {uri.ok ? (
          <DropdownMenuItem onClick={() => void handleCopyUri()}>
            <IconLink className="size-3.5" />
            Copy URI
          </DropdownMenuItem>
        ) : null}
        <DropdownMenuSeparator />
        <DropdownMenuItem onClick={props.onDelete} className="text-danger">
          <IconTrash className="size-3.5" />
          Delete…
        </DropdownMenuItem>
      </DropdownMenuContent>
    </DropdownMenu>
  );
}

export function ConnectionErrorAlert({
  message,
  className,
}: {
  message: string;
  className?: string;
}) {
  return (
    <div
      role="alert"
      className={cn(
        "flex items-start gap-2 rounded-md border border-danger/30 bg-danger/10 px-2 py-1.5 text-xs text-danger",
        className,
      )}
    >
      <IconAlertCircle className="mt-0.5 size-3.5 shrink-0" />
      <div className="flex-1 wrap-break-word">{message}</div>
    </div>
  );
}
