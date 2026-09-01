import { IconX } from "@tabler/icons-react";
import { useEffect, useRef, useState } from "react";

import { DdlPlanPreviewGroups } from "@/components/object-ddl/plan-preview";
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
import {
  applyObjectDdlWithSafetyConfirmation,
  formatObjectDdlError,
  objectDdlRefreshScope,
  previewObjectDdl,
} from "@/lib/object-ddl";
import { parseCanonicalPgObjectRefKey } from "@/lib/pg-object-ref";
import {
  isConnectedStatus,
  pgObjectDescriptionKey,
  pgObjectDdlApplyKey,
  type DdlApplyResult,
  type DdlPlanPreview,
  type PgObjectError,
  type PgObjectOp,
  useAppStore,
} from "@/lib/store";
import { errorToMessage } from "@/lib/tauri";

type PreviewState =
  | { state: "loading" }
  | { state: "ready"; preview: DdlPlanPreview }
  | { state: "error"; error: PgObjectError };

export type DdlReviewTerminal = {
  result: "success" | "partial";
  outcome: string;
  preview: DdlPlanPreview;
  reviewedDdlVersion: number;
  reviewedConnectionEpoch: number;
};

export type DdlReviewDialogProps = {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  connectionId: string;
  ops: PgObjectOp[];
  variant?: "dialog" | "inline";
  onApplied?: (
    result: DdlApplyResult,
    expectedGeneration: number,
  ) => void | Promise<void>;
  onRefresh?: () => void | Promise<void>;
  initialTerminal?: DdlReviewTerminal | null;
  onTerminalChange?: (terminal: DdlReviewTerminal | null) => void;
};

/** Shared reviewed gate for typed PostgreSQL object operations. */
export function DdlReviewDialog({
  open,
  onOpenChange,
  connectionId,
  ops,
  variant = "dialog",
  onApplied,
  onRefresh,
  initialTerminal = null,
  onTerminalChange,
}: DdlReviewDialogProps) {
  // Freeze the complete execution identity. Even if a parent accidentally
  // reuses this component while switching tabs, reviewed operations can never
  // drift onto a different connection.
  const session = useRef({ connectionId, ops }).current;
  const frozenConnectionId = session.connectionId;
  const frozenOps = session.ops;
  const applyKey = useRef(pgObjectDdlApplyKey(frozenConnectionId)).current;
  const globallyApplying = useAppStore(
    (state) => state.pgObjectDdlApplying[applyKey] === true,
  );
  const ddlVersion = useAppStore((state) => state.pgObjectDdlVersion);
  const connectionEpoch = useAppStore(
    (state) => state.connectionEpochs[frozenConnectionId] ?? 0,
  );
  const connectionReady = useAppStore((state) => {
    const connection = state.connections.find(
      (candidate) => candidate.id === frozenConnectionId,
    );
    return (
      connection?.engine === "PostgreSQL" &&
      isConnectedStatus(connection.status) &&
      !state.connectionTransitionIds.includes(frozenConnectionId)
    );
  });
  const applyingRef = useRef(false);
  const [previewState, setPreviewState] = useState<PreviewState>(() =>
    initialTerminal
      ? { state: "ready", preview: initialTerminal.preview }
      : { state: "loading" },
  );
  const [applying, setApplying] = useState(false);
  const [terminalResult, setTerminalResult] = useState<
    "success" | "partial" | null
  >(initialTerminal?.result ?? null);
  const [reviewedDdlVersion, setReviewedDdlVersion] = useState<number | null>(
    initialTerminal?.reviewedDdlVersion ?? null,
  );
  const [reviewedConnectionEpoch, setReviewedConnectionEpoch] = useState<
    number | null
  >(initialTerminal?.reviewedConnectionEpoch ?? null);
  const [outcome, setOutcome] = useState<string | null>(
    initialTerminal?.outcome ?? null,
  );
  const recreatesMaterializedView =
    frozenOps.some(
      (operation) =>
        operation.op === "dropObject" &&
        operation.reference.kind === "materialized-view",
    ) &&
    frozenOps.some((operation) => operation.op === "createMaterializedView");

  const loadPreview = async (preserveOutcome = false) => {
    const stateAtStart = useAppStore.getState();
    const connectionAtStart = stateAtStart.connections.find(
      (candidate) => candidate.id === frozenConnectionId,
    );
    const previewDdlVersion = stateAtStart.pgObjectDdlVersion;
    const previewConnectionEpoch =
      stateAtStart.connectionEpochs[frozenConnectionId] ?? 0;
    setPreviewState({ state: "loading" });
    setTerminalResult(null);
    onTerminalChange?.(null);
    if (!preserveOutcome) setOutcome(null);
    if (
      connectionAtStart?.engine !== "PostgreSQL" ||
      !isConnectedStatus(connectionAtStart.status) ||
      stateAtStart.connectionTransitionIds.includes(frozenConnectionId)
    ) {
      setPreviewState({
        state: "error",
        error: {
          kind: "connection",
          message: "Connect to the PostgreSQL database before reviewing DDL.",
        },
      });
      return;
    }
    const result = await previewObjectDdl({
      connectionId: frozenConnectionId,
      ops: frozenOps,
    });
    if (
      (useAppStore.getState().connectionEpochs[frozenConnectionId] ?? 0) !==
      previewConnectionEpoch
    ) {
      setPreviewState({
        state: "error",
        error: {
          kind: "connection",
          message: "Connection changed. Regenerate the DDL preview.",
        },
      });
    } else if (result.kind === "ok") {
      setReviewedDdlVersion(previewDdlVersion);
      setReviewedConnectionEpoch(previewConnectionEpoch);
      setPreviewState({ state: "ready", preview: result.value });
    } else if (result.kind === "error") {
      setPreviewState({ state: "error", error: result.error });
    }
  };

  useEffect(() => {
    if (!open || terminalResult !== null) return;
    void loadPreview();
    // frozenOps deliberately identifies this one mounted review session.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [open, terminalResult]);

  const publishTerminal = (
    result: DdlReviewTerminal["result"],
    terminalOutcome: string,
  ) => {
    setTerminalResult(result);
    setOutcome(terminalOutcome);
    if (
      previewState.state === "ready" &&
      reviewedDdlVersion !== null &&
      reviewedConnectionEpoch !== null
    ) {
      onTerminalChange?.({
        result,
        outcome: terminalOutcome,
        preview: previewState.preview,
        reviewedDdlVersion,
        reviewedConnectionEpoch,
      });
    }
  };

  const refreshCaches = async (expectedGeneration: number) => {
    const scope = objectDdlRefreshScope(frozenOps);
    const before = useAppStore.getState();
    if (
      (before.pgObjectCatalog[frozenConnectionId]?.generation ?? 0) !==
      expectedGeneration
    ) {
      return false;
    }
    const described = new Set(Object.keys(before.pgObjectDescriptions));
    const references = new Map(
      scope.references.map((reference) => [
        pgObjectDescriptionKey(frozenConnectionId, reference),
        reference,
      ]),
    );
    if (scope.revalidateAllDescriptions) {
      const prefix = `${frozenConnectionId}:`;
      for (const [key, state] of Object.entries(before.pgObjectDescriptions)) {
        if (!key.startsWith(prefix)) continue;
        const reference =
          state.description?.reference ??
          (state.error?.kind === "objectNotFound"
            ? state.error.reference
            : parseCanonicalPgObjectRefKey(key.slice(prefix.length)));
        if (reference) references.set(key, reference);
      }
    }
    if (scope.catalog) {
      const result = await before.loadPgObjectCatalog(
        frozenConnectionId,
        expectedGeneration,
      );
      if (result === "stale") return false;
    }
    for (const [key, reference] of references) {
      if (described.has(key)) {
        await useAppStore
          .getState()
          .loadPgObjectDescription(
            frozenConnectionId,
            reference,
            expectedGeneration,
          );
        if (
          (useAppStore.getState().pgObjectCatalog[frozenConnectionId]
            ?.generation ?? 0) !== expectedGeneration
        ) {
          return false;
        }
      }
    }
    await onRefresh?.();
    return (
      (useAppStore.getState().pgObjectCatalog[frozenConnectionId]?.generation ??
        0) === expectedGeneration
    );
  };

  const handleApply = async () => {
    const stateAtApply = useAppStore.getState();
    const connectionAtApply = stateAtApply.connections.find(
      (candidate) => candidate.id === frozenConnectionId,
    );
    if (
      applyingRef.current ||
      stateAtApply.pgObjectDdlApplying[applyKey] ||
      reviewedDdlVersion !== stateAtApply.pgObjectDdlVersion ||
      reviewedConnectionEpoch !==
        (stateAtApply.connectionEpochs[frozenConnectionId] ?? 0) ||
      connectionAtApply?.engine !== "PostgreSQL" ||
      !isConnectedStatus(connectionAtApply.status) ||
      stateAtApply.connectionTransitionIds.includes(frozenConnectionId) ||
      previewState.state !== "ready" ||
      !open
    ) {
      return;
    }
    if (!useAppStore.getState().beginPgObjectDdlApply(applyKey)) return;
    applyingRef.current = true;
    setApplying(true);
    setOutcome(null);
    const expectedGeneration =
      useAppStore.getState().pgObjectCatalog[frozenConnectionId]?.generation ??
      0;
    const expectedConnectionEpoch = reviewedConnectionEpoch;
    const isConnectionCurrent = () => {
      const current = useAppStore.getState();
      const connection = current.connections.find(
        (candidate) => candidate.id === frozenConnectionId,
      );
      return (
        expectedConnectionEpoch !== null &&
        (current.connectionEpochs[frozenConnectionId] ?? 0) ===
          expectedConnectionEpoch &&
        connection?.engine === "PostgreSQL" &&
        isConnectedStatus(connection.status) &&
        !current.connectionTransitionIds.includes(frozenConnectionId)
      );
    };
    try {
      const connection = useAppStore
        .getState()
        .connections.find((candidate) => candidate.id === frozenConnectionId);
      const result = await applyObjectDdlWithSafetyConfirmation(
        { connectionId: frozenConnectionId, ops: frozenOps },
        connection,
        isConnectionCurrent,
      );
      if (result.kind === "ok") {
        useAppStore.getState().markPgObjectDdlApplied();
        const appliedOutcome = `Applied in ${result.value.runtimeMs} ms.`;
        publishTerminal("success", appliedOutcome);
        try {
          await refreshCaches(expectedGeneration);
          if (
            isConnectionCurrent() &&
            (useAppStore.getState().pgObjectCatalog[frozenConnectionId]
              ?.generation ?? 0) === expectedGeneration
          ) {
            await onApplied?.(result.value, expectedGeneration);
          }
        } catch (refreshError) {
          publishTerminal(
            "success",
            `Applied in ${result.value.runtimeMs} ms, but refresh failed: ${errorToMessage(refreshError)}`,
          );
        }
      } else if (result.kind === "cancelled") {
        setOutcome("Apply cancelled.");
      } else {
        const partialApply =
          (result.error.kind === "database" ||
            result.error.kind === "lockTimeout") &&
          result.error.appliedStatements > 0;
        if (partialApply) {
          useAppStore.getState().markPgObjectDdlApplied();
        }
        const failureOutcome = formatObjectDdlError(
          result.error,
          previewState.preview.statements,
        );
        if (partialApply) {
          publishTerminal("partial", failureOutcome);
        } else {
          setOutcome(failureOutcome);
        }
        if (partialApply) {
          try {
            await refreshCaches(expectedGeneration);
          } catch (refreshError) {
            publishTerminal(
              "partial",
              `${failureOutcome} Metadata refresh failed: ${errorToMessage(refreshError)}`,
            );
          }
        }
      }
    } catch (error) {
      setOutcome(`DDL apply did not finish cleanly: ${errorToMessage(error)}`);
    } finally {
      useAppStore.getState().endPgObjectDdlApply(applyKey);
      applyingRef.current = false;
      setApplying(false);
    }
  };

  const handleOpenChange = (next: boolean) => {
    if (!next && (applyingRef.current || globallyApplying)) return;
    onOpenChange(next);
  };

  if (!open) return null;

  const previewBody = (() => {
    if (previewState.state === "loading") {
      return (
        <LoadingState label="Generating DDL preview…" className="min-h-24" />
      );
    }
    if (previewState.state === "error") {
      return (
        <ErrorState
          message={formatObjectDdlError(previewState.error)}
          onRetry={() => void loadPreview()}
        />
      );
    }
    return (
      <>
        {recreatesMaterializedView ? (
          <p className="border-l-2 border-danger bg-danger/10 px-3 py-2 text-xs text-foreground">
            Recreating this materialized view drops its indexes and grants.
          </p>
        ) : null}
        <DdlPlanPreviewGroups preview={previewState.preview} />
      </>
    );
  })();

  const busy = applying || globallyApplying;
  const regenerateRequired =
    previewState.state === "ready" &&
    ((reviewedDdlVersion !== null && reviewedDdlVersion !== ddlVersion) ||
      (reviewedConnectionEpoch !== null &&
        reviewedConnectionEpoch !== connectionEpoch)) &&
    terminalResult === null;
  const applyDisabled =
    previewState.state !== "ready" ||
    busy ||
    terminalResult !== null ||
    !connectionReady;
  const actions = (
    <>
      <Button
        type="button"
        variant="outline"
        disabled={busy}
        onClick={() => handleOpenChange(false)}
      >
        Close
      </Button>
      <Button
        type="button"
        disabled={applyDisabled}
        onClick={() =>
          void (regenerateRequired ? loadPreview(true) : handleApply())
        }
      >
        {busy
          ? "Applying…"
          : terminalResult === "success"
            ? "Applied"
            : terminalResult === "partial"
              ? "Partially applied"
              : regenerateRequired
                ? "Regenerate preview"
                : "Apply DDL"}
      </Button>
    </>
  );

  if (variant === "inline") {
    return (
      <section
        aria-label="DDL review"
        className="shrink-0 border-y border-border-subtle bg-surface-window"
      >
        <header className="flex items-center gap-2 border-b border-border-subtle px-4 py-2">
          <h2 className="text-sm font-semibold text-foreground">DDL review</h2>
          <span className="ml-auto text-xs text-text-muted">
            {frozenOps.length} operation{frozenOps.length === 1 ? "" : "s"}
          </span>
          <Button
            type="button"
            size="icon-sm"
            variant="ghost"
            aria-label="Close DDL review"
            disabled={busy}
            onClick={() => handleOpenChange(false)}
          >
            <IconX />
          </Button>
        </header>
        <div className="max-h-80 overflow-auto">{previewBody}</div>
        <footer className="flex items-center gap-3 border-t border-border-subtle px-4 py-2">
          {outcome ? (
            <output className="mr-auto text-xs text-text-secondary">
              {outcome}
            </output>
          ) : (
            <span className="mr-auto" />
          )}
          {actions}
        </footer>
      </section>
    );
  }

  return (
    <Dialog open onOpenChange={handleOpenChange}>
      <DialogContent size="xl">
        <DialogHeader showClose={!busy}>
          <DialogTitle>Review DDL</DialogTitle>
          <DialogDescription>
            Review every generated statement before applying it.
          </DialogDescription>
        </DialogHeader>
        <DialogBody className="p-0">{previewBody}</DialogBody>
        <DialogFooter>
          {outcome ? (
            <output className="mr-auto self-center text-xs text-text-secondary">
              {outcome}
            </output>
          ) : null}
          {actions}
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
