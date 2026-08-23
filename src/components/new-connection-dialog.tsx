import { IconPlus, IconX } from "@tabler/icons-react";
import type React from "react";

import { ConnectionForm } from "@/components/connection-form";
import {
  AlertDialog,
  AlertDialogContent,
  AlertDialogHeader,
  AlertDialogTitle,
  AlertDialogTrigger,
} from "@/components/ui/alert-dialog";
import { Button, buttonVariants } from "@/components/ui/button";
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
    <AlertDialog open={open} onOpenChange={onOpenChange}>
      {trigger ? (
        <AlertDialogTrigger render={trigger} />
      ) : (
        <AlertDialogTrigger
          aria-label="Add connection"
          className={cn(
            buttonVariants({ variant: "outline", size: "icon-xs" }),
          )}
        >
          <IconPlus />
        </AlertDialogTrigger>
      )}
      <AlertDialogContent className="flex max-h-[88vh] w-[26rem] max-w-[26rem] flex-col gap-0 overflow-hidden rounded-lg border border-border-subtle bg-surface-window p-0 sm:max-w-[26rem]">
        <AlertDialogHeader className="flex-row items-center justify-between border-b border-border-subtle px-4 py-3">
          <AlertDialogTitle className="text-sm font-semibold">
            New Connection
          </AlertDialogTitle>
          <Button
            type="button"
            size="icon-sm"
            variant="ghost"
            aria-label="Close"
            onClick={() => onOpenChange(false)}
            className="size-7"
          >
            <IconX className="size-3.5" />
          </Button>
        </AlertDialogHeader>
        <div className="flex min-h-0 flex-1 flex-col">
          <ConnectionForm
            mode="new"
            onSaved={() => onOpenChange(false)}
            onCancel={() => onOpenChange(false)}
          />
        </div>
      </AlertDialogContent>
    </AlertDialog>
  );
}
