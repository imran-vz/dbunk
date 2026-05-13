import {
  AlertDialog,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from "@/components/ui/alert-dialog";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";

interface DialogShellProps {
  open: boolean;
  keyName: string;
  onOpenChange: (open: boolean) => void;
  onConfirm: () => Promise<void> | void;
}

export function DeleteDialog({
  open,
  keyName,
  confirmText,
  onConfirmTextChange,
  onOpenChange,
  onConfirm,
}: DialogShellProps & {
  confirmText: string;
  onConfirmTextChange: (text: string) => void;
}) {
  return (
    <AlertDialog open={open} onOpenChange={onOpenChange}>
      <AlertDialogContent>
        <AlertDialogHeader>
          <AlertDialogTitle>Delete key</AlertDialogTitle>
          <AlertDialogDescription>
            Type{" "}
            <code className="rounded bg-surface-panel px-1 py-0.5 font-mono">
              {keyName}
            </code>{" "}
            to confirm. This is irreversible.
          </AlertDialogDescription>
        </AlertDialogHeader>
        <Input
          value={confirmText}
          onChange={(event) => onConfirmTextChange(event.target.value)}
          className="font-mono text-xs"
        />
        <AlertDialogFooter>
          <AlertDialogCancel type="button">Cancel</AlertDialogCancel>
          <Button
            variant="destructive"
            disabled={confirmText !== keyName}
            onClick={() => {
              void onConfirm();
            }}
          >
            Delete
          </Button>
        </AlertDialogFooter>
      </AlertDialogContent>
    </AlertDialog>
  );
}

export function RenameDialog({
  open,
  keyName,
  value,
  onValueChange,
  onOpenChange,
  onConfirm,
}: DialogShellProps & {
  value: string;
  onValueChange: (value: string) => void;
}) {
  return (
    <AlertDialog open={open} onOpenChange={onOpenChange}>
      <AlertDialogContent>
        <AlertDialogHeader>
          <AlertDialogTitle>Rename key</AlertDialogTitle>
          <AlertDialogDescription>
            Rename{" "}
            <code className="rounded bg-surface-panel px-1 py-0.5 font-mono">
              {keyName}
            </code>{" "}
            — issues `RENAME old new`, overwriting any existing key at the new
            name.
          </AlertDialogDescription>
        </AlertDialogHeader>
        <div className="space-y-1">
          <Label htmlFor="rename-key-input" className="text-xs">
            New name
          </Label>
          <Input
            id="rename-key-input"
            value={value}
            onChange={(event) => onValueChange(event.target.value)}
            className="font-mono text-xs"
          />
        </div>
        <AlertDialogFooter>
          <AlertDialogCancel type="button">Cancel</AlertDialogCancel>
          <Button
            disabled={!value.trim() || value === keyName}
            onClick={() => {
              void onConfirm();
            }}
          >
            Rename
          </Button>
        </AlertDialogFooter>
      </AlertDialogContent>
    </AlertDialog>
  );
}

export function ExpireDialog({
  open,
  keyName,
  value,
  onValueChange,
  onOpenChange,
  onConfirm,
}: DialogShellProps & {
  value: string;
  onValueChange: (value: string) => void;
}) {
  return (
    <AlertDialog open={open} onOpenChange={onOpenChange}>
      <AlertDialogContent>
        <AlertDialogHeader>
          <AlertDialogTitle>Set TTL</AlertDialogTitle>
          <AlertDialogDescription>
            Set TTL for{" "}
            <code className="rounded bg-surface-panel px-1 py-0.5 font-mono">
              {keyName}
            </code>
            . Leave blank to remove TTL (PERSIST).
          </AlertDialogDescription>
        </AlertDialogHeader>
        <div className="space-y-1">
          <Label htmlFor="expire-key-input" className="text-xs">
            Seconds
          </Label>
          <Input
            id="expire-key-input"
            type="number"
            value={value}
            onChange={(event) => onValueChange(event.target.value)}
            placeholder="e.g. 3600 — leave empty to PERSIST"
            className="font-mono text-xs"
          />
        </div>
        <AlertDialogFooter>
          <AlertDialogCancel type="button">Cancel</AlertDialogCancel>
          <Button
            onClick={() => {
              void onConfirm();
            }}
          >
            Apply
          </Button>
        </AlertDialogFooter>
      </AlertDialogContent>
    </AlertDialog>
  );
}
