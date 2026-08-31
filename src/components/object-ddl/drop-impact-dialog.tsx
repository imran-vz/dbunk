import { useEffect, useMemo, useRef, useState } from "react";

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
import { ErrorState, LoadingState } from "@/components/ui/state-panel";
import { formatObjectDdlError, loadPgDropImpact } from "@/lib/object-ddl";
import type {
  PgDropImpact,
  PgObjectError,
  PgObjectOp,
  PgObjectRef,
} from "@/lib/store";

const DEPENDENT_RENDER_LIMIT = 200;

type ImpactState =
  | { state: "loading" }
  | { state: "ready"; impact: PgDropImpact }
  | { state: "error"; error: PgObjectError };

export function DropImpactDialog({
  open,
  onOpenChange,
  connectionId,
  reference,
  appendOps = [],
  onReview,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  connectionId: string;
  reference: PgObjectRef;
  appendOps?: PgObjectOp[];
  onReview: (ops: PgObjectOp[]) => void;
}) {
  const session = useRef({ connectionId, reference, appendOps }).current;
  const [impactState, setImpactState] = useState<ImpactState>({
    state: "loading",
  });
  const [cascade, setCascade] = useState(false);

  const loadImpact = async () => {
    setImpactState({ state: "loading" });
    const result = await loadPgDropImpact({
      connectionId: session.connectionId,
      reference: session.reference,
    });
    if (result.kind === "ok") {
      setImpactState({ state: "ready", impact: result.value });
    } else if (result.kind === "error") {
      setImpactState({ state: "error", error: result.error });
    }
  };

  useEffect(() => {
    if (!open) return;
    setCascade(false);
    void loadImpact();
    // Reference is frozen by the mounted action flow.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [open]);

  const recreatesMaterializedView = session.appendOps.some(
    (operation) => operation.op === "createMaterializedView",
  );

  const depthGroups = useMemo(() => {
    if (impactState.state !== "ready") return [];
    const grouped = new Map<number, PgDropImpact["dependents"]>();
    for (const dependent of impactState.impact.dependents.slice(
      0,
      DEPENDENT_RENDER_LIMIT,
    )) {
      const entries = grouped.get(dependent.depth) ?? [];
      entries.push(dependent);
      grouped.set(dependent.depth, entries);
    }
    return [...grouped.entries()].sort(([left], [right]) => left - right);
  }, [impactState]);

  if (!open) return null;
  const impact = impactState.state === "ready" ? impactState.impact : null;
  const dependentCount = impact?.dependents.length ?? 0;

  return (
    <Dialog open onOpenChange={onOpenChange}>
      <DialogContent size="lg" className="border border-danger/40">
        <DialogHeader>
          <DialogTitle>Review drop impact</DialogTitle>
          <DialogDescription>
            PostgreSQL found the transitive objects affected by this drop.
          </DialogDescription>
        </DialogHeader>
        <DialogBody className="space-y-3">
          {impactState.state === "loading" ? (
            <LoadingState label="Loading drop impact…" className="min-h-24" />
          ) : impactState.state === "error" ? (
            <ErrorState
              message={formatObjectDdlError(impactState.error)}
              onRetry={() => void loadImpact()}
            />
          ) : impactState.impact.dependents.length === 0 ? (
            <p className="py-4 text-center text-xs text-text-muted">
              No dependents found.
            </p>
          ) : (
            <div className="divide-y divide-border-subtle border-y border-border-subtle">
              {depthGroups.map(([depth, dependents]) => (
                <section key={depth}>
                  <h3 className="bg-surface-panel px-3 py-1.5 text-2xs font-semibold text-text-secondary">
                    Depth {depth}
                  </h3>
                  <ul className="divide-y divide-border-subtle">
                    {dependents.map((dependent) => (
                      <li
                        key={`${dependent.objectType}:${dependent.identity}`}
                        className="grid grid-cols-[8rem_1fr] gap-3 px-3 py-2 text-xs"
                      >
                        <span className="text-text-muted">
                          {dependent.objectType}
                        </span>
                        <span className="font-mono text-text-secondary">
                          {dependent.identity}
                        </span>
                      </li>
                    ))}
                  </ul>
                </section>
              ))}
            </div>
          )}
          {impact && impact.dependents.length > DEPENDENT_RENDER_LIMIT ? (
            <p className="text-xs text-warning">
              Showing the first {DEPENDENT_RENDER_LIMIT} dependents.
            </p>
          ) : null}
          {impact?.truncated ? (
            <p className="text-xs text-warning">
              The server truncated the dependent list.
            </p>
          ) : null}
          {recreatesMaterializedView ? (
            <p className="border-l-2 border-danger bg-danger/10 px-3 py-2 text-xs text-foreground">
              Recreating this materialized view drops its indexes and grants.
            </p>
          ) : null}
          {impact ? (
            <label className="flex items-start gap-2 border-l-2 border-danger bg-danger/10 px-3 py-2 text-xs text-foreground">
              <input
                type="checkbox"
                checked={cascade}
                onChange={(event) => setCascade(event.target.checked)}
              />
              <span>
                Also drop these {dependentCount} dependent objects, CASCADE
              </span>
            </label>
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
          <Button
            type="button"
            variant="destructive"
            disabled={!impact}
            onClick={() => {
              onReview([
                {
                  op: "dropObject",
                  reference: session.reference,
                  cascade,
                },
                ...session.appendOps,
              ]);
              onOpenChange(false);
            }}
          >
            Review drop DDL
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
