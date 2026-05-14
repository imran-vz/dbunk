import {
  IconAlertCircle,
  IconDatabaseOff,
  IconDotsVertical,
  IconPencil,
  IconTrash,
} from "@tabler/icons-react";

import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import type { Connection } from "@/lib/store";
import { cn } from "@/lib/utils";

export type ConnectionActionsProps = {
  connection: Connection;
  onConnect: () => void;
  onDisconnect: () => void;
  onEdit: () => void;
  onDelete: () => void;
};

export function ConnectionActionsDropdown(props: ConnectionActionsProps) {
  const isDisconnected = props.connection.status === "Disconnected";
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
