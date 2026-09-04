import MonacoEditor from "@monaco-editor/react";
import { useRef, useState } from "react";

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
import type {
  PgObjectDescription,
  PgObjectOp,
  PgParallelSafety,
  PgVolatility,
} from "@/lib/store";
import { useAppStore } from "@/lib/store";

type RoutineKind = "function" | "procedure";
type RoutineFacts = Extract<PgObjectDescription["facts"], { kind: "routine" }>;

export type RoutineEditorState = {
  schema: string;
  name: string;
  arguments: string;
  returns: string;
  language: string;
  body: string;
  volatility: PgVolatility;
  strict: boolean;
  securityDefiner: boolean;
  parallel: PgParallelSafety | null;
};

const volatility = (value: string | null): PgVolatility =>
  value === "immutable" || value === "stable" ? value : "volatile";

const parallel = (value: string | null): PgParallelSafety | null =>
  value === "safe" || value === "restricted" || value === "unsafe"
    ? value
    : null;

export function routineEditorState(
  schema: string,
  name = "",
  facts?: RoutineFacts,
): RoutineEditorState {
  return {
    schema,
    name,
    arguments: facts?.arguments ?? "",
    returns: facts?.returns ?? "",
    language: facts?.language ?? "plpgsql",
    body: facts?.body ?? "BEGIN\n  \nEND;",
    volatility: volatility(facts?.volatility ?? null),
    strict: facts?.strict ?? false,
    securityDefiner: facts?.securityDefiner ?? false,
    parallel: parallel(facts?.parallel ?? null),
  };
}

export function buildRoutineOp(
  kind: RoutineKind,
  state: RoutineEditorState,
  editing: boolean,
): PgObjectOp {
  const shared = {
    schema: state.schema.trim(),
    name: state.name.trim(),
    orReplace: editing,
    arguments: state.arguments.trim(),
    language: state.language.trim(),
    body: state.body,
    securityDefiner: state.securityDefiner,
  };
  return kind === "function"
    ? {
        op: "createFunction",
        ...shared,
        returns: state.returns.trim(),
        volatility: state.volatility,
        strict: state.strict,
        parallel: state.parallel,
      }
    : { op: "createProcedure", ...shared };
}

export function RoutineEditorDialog({
  open,
  onOpenChange,
  connectionId,
  kind,
  schema,
  name,
  facts,
  onOps,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  connectionId: string;
  kind: RoutineKind;
  schema: string;
  name?: string;
  facts?: RoutineFacts;
  onOps?: (ops: PgObjectOp[]) => void;
}) {
  const mode = useRef({
    editing: facts !== undefined,
    readOnly:
      facts !== undefined &&
      (facts.language.trim().toLowerCase() === "c" ||
        facts.language.trim().toLowerCase() === "internal"),
  }).current;
  const { editing, readOnly } = mode;
  const [state, setState] = useState(() =>
    routineEditorState(schema, name, facts),
  );
  const [error, setError] = useState<string | null>(null);
  const [reviewOps, setReviewOps] = useState<PgObjectOp[] | null>(null);
  const editorTheme = useAppStore((store) => store.editorTheme);

  if (!open) return null;
  if (reviewOps) {
    return (
      <DdlReviewDialog
        open
        connectionId={connectionId}
        ops={reviewOps}
        onOpenChange={(next) => {
          if (!next) {
            setReviewOps(null);
            onOpenChange(false);
          }
        }}
      />
    );
  }

  const submit = () => {
    if (readOnly) return;
    if (state.name.trim() === "") {
      setError("Name is required.");
      return;
    }
    if (state.language.trim() === "") {
      setError("Language is required.");
      return;
    }
    if (state.body.trim() === "") {
      setError("Body is required.");
      return;
    }
    if (kind === "function" && state.returns.trim() === "") {
      setError("Return type is required.");
      return;
    }
    const ops = [buildRoutineOp(kind, state, editing)];
    if (onOps) {
      onOps(ops);
      onOpenChange(false);
    } else {
      setReviewOps(ops);
    }
  };

  const noun = kind === "function" ? "function" : "procedure";
  return (
    <Dialog open onOpenChange={onOpenChange}>
      <DialogContent size="xl">
        <DialogHeader>
          <DialogTitle>{editing ? `Edit ${noun}` : `New ${noun}`}</DialogTitle>
          <DialogDescription>
            {editing
              ? "Schema is fixed while editing. Name and arguments are reviewed exactly as entered; PostgreSQL may create a different routine or reject signature and return-type changes under OR REPLACE."
              : "Build a typed routine operation, then review its DDL."}
          </DialogDescription>
        </DialogHeader>
        <DialogBody className="space-y-3">
          {readOnly ? (
            <p
              role="note"
              className="border border-border-subtle px-3 py-2 text-xs text-text-muted"
            >
              C and internal routines expose a symbol name, not editable source.
            </p>
          ) : null}
          <div className="grid grid-cols-2 gap-3">
            <RoutineField label="Schema">
              <Input
                aria-label="Routine schema"
                value={state.schema}
                disabled={editing || readOnly}
                onChange={(event) =>
                  setState((current) => ({
                    ...current,
                    schema: event.target.value,
                  }))
                }
              />
            </RoutineField>
            <RoutineField label="Name">
              <Input
                aria-label="Routine name"
                value={state.name}
                disabled={readOnly}
                onChange={(event) =>
                  setState((current) => ({
                    ...current,
                    name: event.target.value,
                  }))
                }
              />
            </RoutineField>
          </div>
          <RoutineField label="Arguments">
            <Input
              aria-label="Routine arguments"
              value={state.arguments}
              disabled={readOnly}
              onChange={(event) =>
                setState((current) => ({
                  ...current,
                  arguments: event.target.value,
                }))
              }
              placeholder="a integer, b text DEFAULT 'x'"
              className="font-mono"
            />
          </RoutineField>
          {kind === "function" ? (
            <RoutineField label="Returns">
              <Input
                aria-label="Routine returns"
                value={state.returns}
                disabled={readOnly}
                onChange={(event) =>
                  setState((current) => ({
                    ...current,
                    returns: event.target.value,
                  }))
                }
                placeholder="integer or SETOF record"
                className="font-mono"
              />
            </RoutineField>
          ) : null}
          <div className="grid grid-cols-3 gap-3">
            <RoutineField label="Language">
              <Input
                aria-label="Routine language"
                list="routine-language-suggestions"
                value={state.language}
                disabled={readOnly}
                onChange={(event) =>
                  setState((current) => ({
                    ...current,
                    language: event.target.value,
                  }))
                }
              />
              <datalist id="routine-language-suggestions">
                <option value="plpgsql">plpgsql</option>
                <option value="sql">sql</option>
              </datalist>
            </RoutineField>
            {kind === "function" ? (
              <RoutineField label="Volatility">
                <select
                  aria-label="Routine volatility"
                  value={state.volatility}
                  disabled={readOnly}
                  onChange={(event) => {
                    const value = event.target.value;
                    if (
                      value === "immutable" ||
                      value === "stable" ||
                      value === "volatile"
                    )
                      setState((current) => ({
                        ...current,
                        volatility: value,
                      }));
                  }}
                  className="h-8 border border-border-subtle bg-surface-input px-2 text-xs"
                >
                  <option value="volatile">VOLATILE</option>
                  <option value="stable">STABLE</option>
                  <option value="immutable">IMMUTABLE</option>
                </select>
              </RoutineField>
            ) : null}
            {kind === "function" ? (
              <RoutineField label="Parallel">
                <select
                  aria-label="Routine parallel"
                  value={state.parallel ?? "default"}
                  disabled={readOnly}
                  onChange={(event) => {
                    const value = event.target.value;
                    setState((current) => ({
                      ...current,
                      parallel:
                        value === "safe" ||
                        value === "restricted" ||
                        value === "unsafe"
                          ? value
                          : null,
                    }));
                  }}
                  className="h-8 border border-border-subtle bg-surface-input px-2 text-xs"
                >
                  <option value="default">Default</option>
                  <option value="safe">SAFE</option>
                  <option value="restricted">RESTRICTED</option>
                  <option value="unsafe">UNSAFE</option>
                </select>
              </RoutineField>
            ) : null}
          </div>
          <div className="flex flex-wrap gap-4 text-xs text-text-secondary">
            {kind === "function" ? (
              <RoutineCheck
                label="STRICT"
                checked={state.strict}
                disabled={readOnly}
                onChange={(strict) =>
                  setState((current) => ({ ...current, strict }))
                }
              />
            ) : null}
            <RoutineCheck
              label="SECURITY DEFINER"
              checked={state.securityDefiner}
              disabled={readOnly}
              onChange={(securityDefiner) =>
                setState((current) => ({ ...current, securityDefiner }))
              }
            />
          </div>
          <RoutineField label="Body">
            <div className="h-64 overflow-hidden border border-border-subtle">
              <MonacoEditor
                language="sql"
                theme={editorTheme}
                value={state.body}
                onChange={(body) =>
                  setState((current) => ({ ...current, body: body ?? "" }))
                }
                options={{
                  minimap: { enabled: false },
                  readOnly,
                  fontSize: 12,
                  lineNumbers: "on",
                  scrollBeyondLastLine: false,
                }}
              />
            </div>
          </RoutineField>
          {error ? (
            <p role="alert" className="text-xs text-danger">
              {error}
            </p>
          ) : null}
        </DialogBody>
        <DialogFooter>
          <Button variant="outline" onClick={() => onOpenChange(false)}>
            Cancel
          </Button>
          <Button disabled={readOnly} onClick={submit}>
            Review DDL
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}

function RoutineField({
  label,
  children,
}: {
  label: string;
  children: React.ReactNode;
}) {
  return (
    <label className="grid gap-1 text-xs text-text-secondary">
      <span>{label}</span>
      {children}
    </label>
  );
}

function RoutineCheck({
  label,
  checked,
  onChange,
  disabled = false,
}: {
  label: string;
  checked: boolean;
  onChange: (checked: boolean) => void;
  disabled?: boolean;
}) {
  return (
    <label className="flex items-center gap-1.5">
      <input
        type="checkbox"
        checked={checked}
        disabled={disabled}
        onChange={(event) => onChange(event.target.checked)}
      />
      {label}
    </label>
  );
}
