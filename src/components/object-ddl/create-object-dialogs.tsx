import { useState } from "react";

import { DdlReviewDialog } from "@/components/object-ddl/ddl-review-dialog";
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
import type { PgObjectOp } from "@/lib/store";

const optional = (value: string): string | null => {
  const trimmed = value.trim();
  return trimmed === "" ? null : trimmed;
};

export const buildCreateSchemaOps = (name: string): PgObjectOp[] => [
  { op: "createSchema", name: name.trim() },
];

export const buildCreateSequenceOps = (input: {
  schema: string;
  name: string;
  dataType: string;
  start: string;
  increment: string;
  minValue: string;
  maxValue: string;
  cycle: boolean | null;
  cache: string;
}): PgObjectOp[] => [
  {
    op: "createSequence",
    schema: input.schema,
    name: input.name.trim(),
    dataType: optional(input.dataType),
    start: optional(input.start),
    increment: optional(input.increment),
    minValue: optional(input.minValue),
    maxValue: optional(input.maxValue),
    cycle: input.cycle,
    cache: optional(input.cache),
  },
];

export const buildCreateEnumOps = (input: {
  schema: string;
  name: string;
  labels: string[];
}): PgObjectOp[] => [
  {
    op: "createEnum",
    schema: input.schema,
    name: input.name.trim(),
    labels: input.labels.map((label) => label.trim()).filter(Boolean),
  },
];

export const buildCreateViewOps = (input: {
  schema: string;
  name: string;
  sqlBody: string;
  orReplace: boolean;
}): PgObjectOp[] => [
  {
    op: "createView",
    schema: input.schema,
    name: input.name.trim(),
    sqlBody: input.sqlBody.trim(),
    orReplace: input.orReplace,
  },
];

export const buildCreateMaterializedViewOps = (input: {
  schema: string;
  name: string;
  sqlBody: string;
  withData: boolean;
}): PgObjectOp[] => [
  {
    op: "createMaterializedView",
    schema: input.schema,
    name: input.name.trim(),
    sqlBody: input.sqlBody.trim(),
    withData: input.withData,
  },
];

type CreateKind = "schema" | "sequence" | "enum" | "view" | "materialized-view";

type CreateObjectDialogProps = {
  kind: CreateKind;
  open: boolean;
  onOpenChange: (open: boolean) => void;
  connectionId: string;
  schema?: string;
  initialName?: string;
  initialSqlBody?: string;
  initialWithData?: boolean;
  orReplace?: boolean;
  onOps?: (ops: PgObjectOp[]) => void;
};

const TITLES = {
  schema: "New schema",
  sequence: "New sequence",
  enum: "New enum",
  view: "New view",
  "materialized-view": "New materialized view",
} satisfies Record<CreateKind, string>;

function CreateObjectDialog({
  kind,
  open,
  onOpenChange,
  connectionId,
  schema = "",
  initialName = "",
  initialSqlBody = "",
  initialWithData = true,
  orReplace = false,
  onOps,
}: CreateObjectDialogProps) {
  const [name, setName] = useState(initialName);
  const [sqlBody, setSqlBody] = useState(initialSqlBody);
  const [labels, setLabels] = useState("");
  const [dataType, setDataType] = useState("");
  const [start, setStart] = useState("");
  const [increment, setIncrement] = useState("");
  const [minValue, setMinValue] = useState("");
  const [maxValue, setMaxValue] = useState("");
  const [cache, setCache] = useState("");
  const [cycle, setCycle] = useState<"default" | "yes" | "no">("default");
  const [withData, setWithData] = useState(initialWithData);
  const [error, setError] = useState<string | null>(null);
  const [reviewOps, setReviewOps] = useState<PgObjectOp[] | null>(null);
  const sequenceFields: Array<{
    label: string;
    value: string;
    setValue: (value: string) => void;
  }> = [
    { label: "Data type", value: dataType, setValue: setDataType },
    { label: "Start", value: start, setValue: setStart },
    { label: "Increment", value: increment, setValue: setIncrement },
    { label: "Minimum", value: minValue, setValue: setMinValue },
    { label: "Maximum", value: maxValue, setValue: setMaxValue },
    { label: "Cache", value: cache, setValue: setCache },
  ];

  const buildOps = (): PgObjectOp[] | null => {
    if (name.trim() === "") {
      setError("Name is required.");
      return null;
    }
    if (kind === "schema") return buildCreateSchemaOps(name);
    if (schema === "") {
      setError("Schema is required.");
      return null;
    }
    if (kind === "sequence") {
      return buildCreateSequenceOps({
        schema,
        name,
        dataType,
        start,
        increment,
        minValue,
        maxValue,
        cycle: cycle === "default" ? null : cycle === "yes",
        cache,
      });
    }
    if (kind === "enum") {
      const orderedLabels = labels
        .split("\n")
        .map((label) => label.trim())
        .filter(Boolean);
      if (orderedLabels.length === 0) {
        setError("Add at least one enum label.");
        return null;
      }
      return buildCreateEnumOps({ schema, name, labels: orderedLabels });
    }
    if (sqlBody.trim() === "") {
      setError("SQL body is required.");
      return null;
    }
    return kind === "view"
      ? buildCreateViewOps({ schema, name, sqlBody, orReplace })
      : buildCreateMaterializedViewOps({ schema, name, sqlBody, withData });
  };

  if (!open) return null;
  if (reviewOps) {
    return (
      <DdlReviewDialog
        open
        onOpenChange={(next) => {
          if (!next) {
            setReviewOps(null);
            onOpenChange(false);
          }
        }}
        connectionId={connectionId}
        ops={reviewOps}
      />
    );
  }

  return (
    <Dialog open onOpenChange={onOpenChange}>
      <DialogContent
        size={kind === "view" || kind === "materialized-view" ? "lg" : "md"}
      >
        <DialogHeader>
          <DialogTitle>{TITLES[kind]}</DialogTitle>
          <DialogDescription>
            Build typed operations, then review the generated DDL.
          </DialogDescription>
        </DialogHeader>
        <DialogBody>
          <form
            id="create-object-form"
            className="grid gap-3"
            onSubmit={(event) => {
              event.preventDefault();
              setError(null);
              const ops = buildOps();
              if (!ops) return;
              if (onOps) {
                onOps(ops);
                onOpenChange(false);
              } else {
                setReviewOps(ops);
              }
            }}
          >
            {kind === "schema" ? null : (
              <div className="text-xs text-text-muted">
                Schema{" "}
                <span className="font-mono text-foreground">{schema}</span>
              </div>
            )}
            <div className="grid gap-1 text-xs text-text-secondary">
              Name
              <Input
                aria-label={`${TITLES[kind]} name`}
                value={name}
                onChange={(event) => setName(event.target.value)}
              />
            </div>
            {kind === "enum" ? (
              <div className="grid gap-1 text-xs text-text-secondary">
                Ordered labels, one per line
                <Textarea
                  aria-label="Enum labels"
                  value={labels}
                  onChange={(event) => setLabels(event.target.value)}
                />
              </div>
            ) : null}
            {kind === "view" || kind === "materialized-view" ? (
              <div className="grid gap-1 text-xs text-text-secondary">
                SQL body
                <Textarea
                  aria-label="SQL body"
                  value={sqlBody}
                  onChange={(event) => setSqlBody(event.target.value)}
                  className="min-h-40 font-mono"
                />
              </div>
            ) : null}
            {kind === "materialized-view" ? (
              <label className="flex items-center gap-2 text-xs text-text-secondary">
                <input
                  type="checkbox"
                  checked={withData}
                  onChange={(event) => setWithData(event.target.checked)}
                />
                Populate with data
              </label>
            ) : null}
            {kind === "sequence" ? (
              <div className="grid grid-cols-2 gap-3">
                {sequenceFields.map(({ label, value, setValue }) => (
                  <div
                    key={label}
                    className="grid gap-1 text-xs text-text-secondary"
                  >
                    {label}
                    <Input
                      aria-label={`Sequence ${label.toLowerCase()}`}
                      value={value}
                      onChange={(event) => setValue(event.target.value)}
                    />
                  </div>
                ))}
                <div className="grid gap-1 text-xs text-text-secondary">
                  Cycle
                  <select
                    aria-label="Sequence cycle"
                    value={cycle}
                    onChange={(event) => {
                      const value = event.target.value;
                      if (
                        value === "default" ||
                        value === "yes" ||
                        value === "no"
                      ) {
                        setCycle(value);
                      }
                    }}
                    className="h-8 rounded-sm border border-border-subtle bg-surface-input px-2 text-xs text-foreground"
                  >
                    <option value="default">Database default</option>
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
          </form>
        </DialogBody>
        <DialogFooter>
          <Button
            type="button"
            variant="outline"
            onClick={() => onOpenChange(false)}
          >
            Cancel
          </Button>
          <Button type="submit" form="create-object-form">
            Review DDL
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}

type CommonProps = Omit<CreateObjectDialogProps, "kind">;

export function NewSchemaDialog(props: CommonProps) {
  return <CreateObjectDialog {...props} kind="schema" />;
}

export function NewSequenceDialog(props: CommonProps) {
  return <CreateObjectDialog {...props} kind="sequence" />;
}

export function NewEnumDialog(props: CommonProps) {
  return <CreateObjectDialog {...props} kind="enum" />;
}

export function NewViewDialog(props: CommonProps) {
  return <CreateObjectDialog {...props} kind="view" />;
}

export function NewMaterializedViewDialog(props: CommonProps) {
  return <CreateObjectDialog {...props} kind="materialized-view" />;
}
