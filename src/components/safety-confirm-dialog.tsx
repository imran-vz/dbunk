import { useState, useSyncExternalStore } from "react";

import { EnvironmentBadge } from "@/components/environment-badge";
import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from "@/components/ui/alert-dialog";
import { Input } from "@/components/ui/input";
import {
  getSafetyConfirmation,
  resolveSafetyConfirmation,
  safetyConfirmationRequiresTyping,
  subscribeSafetyConfirmation,
} from "@/lib/safety-confirmation";

const CLASS_LABEL = {
  read: "Read",
  dml: "Data change",
  ddl: "Schema change",
  transaction: "Transaction",
  session: "Session",
  unknown: "Unknown statement",
} as const;

const commandLabel = (command: string) =>
  command
    .replaceAll("_", " ")
    .replace(/^./, (character) => character.toUpperCase());

export function SafetyConfirmDialog() {
  const request = useSyncExternalStore(
    subscribeSafetyConfirmation,
    getSafetyConfirmation,
    () => null,
  );
  const [confirmation, setConfirmation] = useState({
    requestId: "",
    value: "",
  });

  if (!request) return null;

  const requiresTyping = safetyConfirmationRequiresTyping(request);
  const confirmationValue =
    confirmation.requestId === request.id ? confirmation.value : "";
  const canConfirm =
    !requiresTyping || confirmationValue === request.connection.name;

  return (
    <AlertDialog
      open
      onOpenChange={(open) => {
        if (!open) {
          setConfirmation({ requestId: "", value: "" });
          resolveSafetyConfirmation(false);
        }
      }}
    >
      <AlertDialogContent size="md" className="border border-danger/40">
        <AlertDialogHeader>
          <AlertDialogTitle className="flex items-center gap-2">
            Confirm safety override
            <EnvironmentBadge environment={request.connection.environment} />
          </AlertDialogTitle>
          <AlertDialogDescription>
            The safety policy refused this operation on{" "}
            <strong className="text-foreground">
              {request.connection.name}
            </strong>
            .
          </AlertDialogDescription>
        </AlertDialogHeader>

        {request.subject.kind === "statements" ? (
          <ul className="space-y-1 border-y border-border-subtle py-2 text-xs text-text-secondary">
            {request.subject.statements.map((statement) => (
              <li key={statement.index} className="flex items-center gap-2">
                <span className="font-medium text-foreground">
                  {CLASS_LABEL[statement.class]}
                </span>
                {statement.unbounded ? (
                  <span className="text-warning">Unbounded</span>
                ) : null}
                {statement.destructive ? (
                  <span className="text-danger">Destructive</span>
                ) : null}
              </li>
            ))}
          </ul>
        ) : (
          <div className="border-y border-border-subtle py-2 text-xs text-text-secondary">
            Command:{" "}
            <span className="font-medium text-foreground">
              {commandLabel(request.subject.command)}
            </span>
          </div>
        )}

        {requiresTyping ? (
          <label className="grid gap-1.5 text-xs text-text-secondary">
            <span>
              Type{" "}
              <strong className="text-foreground">
                {request.connection.name}
              </strong>{" "}
              to continue
            </span>
            <Input
              value={confirmationValue}
              onChange={(event) =>
                setConfirmation({
                  requestId: request.id,
                  value: event.target.value,
                })
              }
              autoComplete="off"
            />
          </label>
        ) : null}

        <AlertDialogFooter>
          <AlertDialogAction
            variant="outline"
            onClick={() => {
              setConfirmation({ requestId: "", value: "" });
              resolveSafetyConfirmation(false);
            }}
          >
            Cancel
          </AlertDialogAction>
          <AlertDialogAction
            variant="destructive"
            disabled={!canConfirm}
            onClick={() => {
              setConfirmation({ requestId: "", value: "" });
              resolveSafetyConfirmation(true);
            }}
          >
            Confirm and run
          </AlertDialogAction>
        </AlertDialogFooter>
      </AlertDialogContent>
    </AlertDialog>
  );
}
