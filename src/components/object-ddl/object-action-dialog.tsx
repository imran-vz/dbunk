import { useState } from "react";

import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogBody,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { Textarea } from "@/components/ui/textarea";
import type { PgObjectOp, PgObjectRef } from "@/lib/store";

export type ObjectActionKind =
  | "comment"
  | "rename"
  | "add-enum-value"
  | "rename-enum-value"
  | "alter-sequence";

export const buildObjectActionOps = (input: {
  action: ObjectActionKind;
  reference: PgObjectRef;
  primary: string;
  secondary: string;
  restartWith: string;
  incrementBy: string;
  minValue: string;
  maxValue: string;
  cache: string;
  cycle: boolean | null;
}): PgObjectOp[] => {
  const schema = input.reference.schema ?? "";
  const optional = (value: string) => value.trim() || null;
  switch (input.action) {
    case "comment":
      return [
        {
          op: "setComment",
          target: { kind: "object", reference: input.reference },
          comment: optional(input.primary),
        },
      ];
    case "rename":
      return [
        {
          op: "renameObject",
          reference: input.reference,
          newName: input.primary.trim(),
        },
      ];
    case "add-enum-value":
      return [
        {
          op: "addEnumValue",
          schema,
          name: input.reference.name,
          value: input.primary.trim(),
          position: null,
        },
      ];
    case "rename-enum-value":
      return [
        {
          op: "renameEnumValue",
          schema,
          name: input.reference.name,
          from: input.primary,
          to: input.secondary.trim(),
        },
      ];
    case "alter-sequence":
      return [
        {
          op: "alterSequence",
          schema,
          name: input.reference.name,
          restartWith: optional(input.restartWith),
          incrementBy: optional(input.incrementBy),
          minValue: optional(input.minValue),
          maxValue: optional(input.maxValue),
          cycle: input.cycle,
          cache: optional(input.cache),
        },
      ];
  }
  input.action satisfies never;
};

const TITLES = {
  comment: "Edit comment",
  rename: "Rename object",
  "add-enum-value": "Add enum value",
  "rename-enum-value": "Rename enum value",
  "alter-sequence": "Alter sequence",
} satisfies Record<ObjectActionKind, string>;

export function ObjectActionDialog({
  open,
  onOpenChange,
  action,
  reference,
  currentComment,
  enumLabels = [],
  onOps,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  action: ObjectActionKind;
  reference: PgObjectRef;
  currentComment: string | null;
  enumLabels?: string[];
  onOps: (ops: PgObjectOp[]) => void;
}) {
  const [primary, setPrimary] = useState(
    action === "comment"
      ? (currentComment ?? "")
      : action === "rename-enum-value"
        ? (enumLabels[0] ?? "")
        : "",
  );
  const [secondary, setSecondary] = useState("");
  const [restartWith, setRestartWith] = useState("");
  const [incrementBy, setIncrementBy] = useState("");
  const [minValue, setMinValue] = useState("");
  const [maxValue, setMaxValue] = useState("");
  const [cache, setCache] = useState("");
  const [cycle, setCycle] = useState<"unchanged" | "yes" | "no">("unchanged");
  const [error, setError] = useState<string | null>(null);

  if (!open) return null;
  const requiresPrimary = action !== "comment" && action !== "alter-sequence";
  const submit = () => {
    if (requiresPrimary && primary.trim() === "") {
      setError("A value is required.");
      return;
    }
    if (action === "rename-enum-value" && secondary.trim() === "") {
      setError("The new enum value is required.");
      return;
    }
    onOps(
      buildObjectActionOps({
        action,
        reference,
        primary,
        secondary,
        restartWith,
        incrementBy,
        minValue,
        maxValue,
        cache,
        cycle: cycle === "unchanged" ? null : cycle === "yes",
      }),
    );
    onOpenChange(false);
  };

  return (
    <Dialog open onOpenChange={onOpenChange}>
      <DialogContent size="md">
        <DialogHeader>
          <DialogTitle>{TITLES[action]}</DialogTitle>
          <DialogDescription>
            This action opens a DDL review before anything is applied.
          </DialogDescription>
        </DialogHeader>
        <DialogBody className="grid gap-3">
          {action === "comment" ? (
            <div className="grid gap-1 text-xs text-text-secondary">
              Comment
              <Textarea
                aria-label="Object comment"
                value={primary}
                onChange={(event) => setPrimary(event.target.value)}
              />
            </div>
          ) : null}
          {action === "rename" || action === "add-enum-value" ? (
            <div className="grid gap-1 text-xs text-text-secondary">
              {action === "rename" ? "New name" : "Value"}
              <Input
                aria-label={
                  action === "rename" ? "New object name" : "Enum value"
                }
                value={primary}
                onChange={(event) => setPrimary(event.target.value)}
              />
            </div>
          ) : null}
          {action === "rename-enum-value" ? (
            <>
              <div className="grid gap-1 text-xs text-text-secondary">
                Existing value
                <select
                  aria-label="Existing enum value"
                  value={primary}
                  onChange={(event) => setPrimary(event.target.value)}
                  className="h-8 rounded-sm border border-border-subtle bg-surface-input px-2 text-xs text-foreground"
                >
                  {enumLabels.map((label) => (
                    <option key={label} value={label}>
                      {label}
                    </option>
                  ))}
                </select>
              </div>
              <div className="grid gap-1 text-xs text-text-secondary">
                New value
                <Input
                  aria-label="New enum value"
                  value={secondary}
                  onChange={(event) => setSecondary(event.target.value)}
                />
              </div>
            </>
          ) : null}
          {action === "alter-sequence" ? (
            <div className="grid grid-cols-2 gap-3">
              <div className="grid gap-1 text-xs text-text-secondary">
                Restart with
                <Input
                  aria-label="Restart sequence with"
                  value={restartWith}
                  onChange={(event) => setRestartWith(event.target.value)}
                />
              </div>
              <div className="grid gap-1 text-xs text-text-secondary">
                Increment by
                <Input
                  aria-label="Sequence increment by"
                  value={incrementBy}
                  onChange={(event) => setIncrementBy(event.target.value)}
                />
              </div>
              <div className="grid gap-1 text-xs text-text-secondary">
                Minimum
                <Input
                  aria-label="Sequence minimum"
                  value={minValue}
                  onChange={(event) => setMinValue(event.target.value)}
                />
              </div>
              <div className="grid gap-1 text-xs text-text-secondary">
                Maximum
                <Input
                  aria-label="Sequence maximum"
                  value={maxValue}
                  onChange={(event) => setMaxValue(event.target.value)}
                />
              </div>
              <div className="grid gap-1 text-xs text-text-secondary">
                Cache
                <Input
                  aria-label="Sequence cache"
                  value={cache}
                  onChange={(event) => setCache(event.target.value)}
                />
              </div>
              <div className="grid gap-1 text-xs text-text-secondary">
                Cycle
                <select
                  aria-label="Alter sequence cycle"
                  value={cycle}
                  onChange={(event) => {
                    const value = event.target.value;
                    if (
                      value === "unchanged" ||
                      value === "yes" ||
                      value === "no"
                    ) {
                      setCycle(value);
                    }
                  }}
                  className="h-8 rounded-sm border border-border-subtle bg-surface-input px-2 text-xs text-foreground"
                >
                  <option value="unchanged">Unchanged</option>
                  <option value="yes">Cycle</option>
                  <option value="no">No cycle</option>
                </select>
              </div>
            </div>
          ) : null}
          {error ? (
            <div role="alert" className="text-xs text-danger">
              {error}
            </div>
          ) : null}
        </DialogBody>
        <DialogFooter>
          <Button
            type="button"
            variant="outline"
            onClick={() => onOpenChange(false)}
          >
            Cancel
          </Button>
          <Button type="button" onClick={submit}>
            Review DDL
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
