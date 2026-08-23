import { IconPlus } from "@tabler/icons-react";
import type React from "react";

import { ConnectionForm } from "@/components/connection-form";
import { buttonVariants } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
  DialogTrigger,
} from "@/components/ui/dialog";
import { cn } from "@/lib/utils";

interface NewConnectionDialogProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  trigger?: React.ReactElement;
}

export function NewConnectionDialog({
  open,
  onOpenChange,
  trigger,
}: NewConnectionDialogProps) {
  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      {trigger ? (
        <DialogTrigger render={trigger} />
      ) : (
        <DialogTrigger
          aria-label="Add connection"
          className={cn(
            buttonVariants({ variant: "outline", size: "icon-xs" }),
          )}
        >
          <IconPlus />
        </DialogTrigger>
      )}
      <DialogContent size="md">
        <DialogHeader>
          <DialogTitle>New Connection</DialogTitle>
        </DialogHeader>
        <div className="flex min-h-0 flex-1 flex-col">
          <ConnectionForm
            mode="new"
            onSaved={() => onOpenChange(false)}
            onCancel={() => onOpenChange(false)}
          />
        </div>
      </DialogContent>
    </Dialog>
  );
}
