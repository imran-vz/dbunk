import { IconRefresh, IconX } from "@tabler/icons-react";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";

import { Button } from "@/components/ui/button";
import type { ApplyResult, ResultMutationError } from "@/lib/result-mutation";
import {
  analyzeResultSet,
  applyResultMutations,
  cancelResultMutation,
  previewResultMutations,
  type ResultMutationClientResult,
} from "@/lib/result-mutation-client";
import { confirmWriteStatements } from "@/lib/safety-confirmation";
import {
  type MutationDraft,
  type MutationDraftApplyRequest,
  type MutationDraftHandle,
  type MutationDraftPreviewRequest,
  type MutationDraftScope,
  useAppStore,
} from "@/lib/store";

import { MutationChangeList } from "./change-list";
import { DmlPreview } from "./dml-preview";
import {
  formatMutationError,
  groupMutationChanges,
  mutationTargets,
} from "./model";

export type MutationReviewPanelProps = {
  scope: MutationDraftScope;
  onClose: () => void;
  onApplySuccess?: (result: ApplyResult) => void | Promise<void>;
  onRefresh?: () => void | Promise<void>;
  onRerunQuery?: () => void | Promise<void>;
};

type FailedClientResult = Exclude<
  ResultMutationClientResult<unknown>,
  {
    kind: "ok";
  }
>;

const clientResultError = (result: FailedClientResult): ResultMutationError =>
  result.kind === "error" ? result.error : { kind: result.kind };

const draftHandle = (draft: MutationDraft): MutationDraftHandle => ({
  scope: draft.scope,
  generation: draft.generation,
});

const analyzeDraft = (draft: MutationDraft) =>
  analyzeResultSet({
    connectionId: draft.connectionId,
    tabId: draft.owner.tabId,
    source: draft.source,
    refreshStructure: false,
  });

const hasIncludedChanges = (draft: MutationDraft): boolean =>
  draft.changeOrder.some((changeId) => draft.changes[changeId]?.included);

const applyWithSafetyConfirmation = async (
  payload: Parameters<typeof applyResultMutations>[0],
  canRetry: () => boolean,
): Promise<ResultMutationClientResult<ApplyResult>> => {
  const firstAttempt = await applyResultMutations(payload);
  if (
    firstAttempt.kind !== "error" ||
    firstAttempt.error.kind !== "policyNeedsConfirmation"
  ) {
    return firstAttempt;
  }
  const connection = useAppStore
    .getState()
    .connections.find((candidate) => candidate.id === payload.connectionId);
  if (!connection) {
    return { kind: "error", error: { kind: "connectionLost" } };
  }
  const confirmed = await confirmWriteStatements(
    connection,
    firstAttempt.error.statements,
  );
  if (!confirmed) return { kind: "cancelled" };
  if (!canRetry()) return { kind: "superseded" };
  return applyResultMutations({ ...payload, confirmed: true });
};

const statusForDraft = (
  draft: MutationDraft | undefined,
  analysisError: ResultMutationError | null,
): string => {
  if (!draft) return "This mutation draft is no longer available.";
  if (draft.sourceInvalidated) {
    switch (draft.apply.state) {
      case "applying":
        return "The apply is still finishing after the database restore. Its outcome may include committed changes.";
      case "failed":
        return "The apply ended after the database restore invalidated its source. Its final database state cannot be safely inferred from this stale draft.";
      case "success":
        return "The apply reported success after the database restore invalidated its source. Reload to see the final database state.";
      case "idle":
        return "The staged edits are stale after the database restore and cannot be changed or applied.";
    }
  }
  if (analysisError) return formatMutationError(analysisError);
  switch (draft.apply.state) {
    case "applying":
      return "Applying in one transaction. The draft is locked until completion or cancellation.";
    case "failed":
      return `${formatMutationError(draft.apply.error)} All staged changes remain.`;
    case "success": {
      const affected = draft.apply.result.operations
        .map((operation) => operation.rowsAffected)
        .join(", ");
      const stale =
        draft.owner.kind === "query" ? " Query result is stale." : "";
      return `Applied ${draft.apply.result.operations.length} operations. Rows affected: ${affected}.${stale}`;
    }
    case "idle":
      break;
  }
  if (!draft.analysis) return "Analyzing result editability.";
  if (!hasIncludedChanges(draft)) {
    return "Include at least one staged change to generate a preview.";
  }
  switch (draft.preview.state) {
    case "idle":
    case "loading":
      return "Generating a fresh mutation preview.";
    case "error":
      return formatMutationError(draft.preview.error);
    case "ready":
      return draft.preview.reviewed
        ? "Review ready. Included changes will apply in one transaction."
        : "Analysis changed the generated DML. Review the refreshed preview before applying.";
  }
};

const callPanelCallback = async (
  callback: (() => void | Promise<void>) | undefined,
): Promise<boolean> => {
  if (!callback) return true;
  try {
    await callback();
    return true;
  } catch {
    return false;
  }
};

export function MutationReviewPanel({
  scope,
  onClose,
  onApplySuccess,
  onRefresh,
  onRerunQuery,
}: MutationReviewPanelProps) {
  const draft = useAppStore((state) => state.mutationDrafts[scope]);
  const beginPreview = useAppStore((state) => state.beginMutationDraftPreview);
  const resolvePreview = useAppStore(
    (state) => state.resolveMutationDraftPreview,
  );
  const failPreview = useAppStore((state) => state.failMutationDraftPreview);
  const setAnalysis = useAppStore((state) => state.setMutationDraftAnalysis);
  const recoverAnalysis = useAppStore(
    (state) => state.recoverMutationDraftAnalysis,
  );
  const retryApplyAfterRecovery = useAppStore(
    (state) => state.retryMutationDraftApplyAfterRecovery,
  );
  const acknowledgePreview = useAppStore(
    (state) => state.acknowledgeMutationDraftPreview,
  );
  const beginApply = useAppStore((state) => state.beginMutationDraftApply);
  const resolveApply = useAppStore((state) => state.resolveMutationDraftApply);
  const failApply = useAppStore((state) => state.failMutationDraftApply);
  const setIncluded = useAppStore(
    (state) => state.setMutationDraftChangeIncluded,
  );
  const revertChange = useAppStore((state) => state.revertMutationDraftChange);
  const revertAll = useAppStore((state) => state.revertAllMutationDraftChanges);

  const [announcement, setAnnouncement] = useState<string | null>(null);
  const [analysisFailure, setAnalysisFailure] = useState<{
    scope: MutationDraftScope;
    generation: number;
    error: ResultMutationError;
  } | null>(null);
  const [cancelPending, setCancelPending] = useState(false);
  const [cancelRequested, setCancelRequested] = useState(false);
  const analysisRun = useRef<
    { scope: MutationDraftScope; generation: number } | undefined
  >(undefined);
  const previewRun = useRef<
    { scope: MutationDraftScope; token: symbol } | undefined
  >(undefined);

  const visibleAnalysisFailure =
    draft &&
    analysisFailure?.scope === scope &&
    analysisFailure.generation === draft.generation
      ? analysisFailure.error
      : null;

  const groups = useMemo(
    () => (draft ? groupMutationChanges(draft) : []),
    [draft],
  );
  const targets = useMemo(() => (draft ? mutationTargets(draft) : []), [draft]);
  const includedCount = draft
    ? draft.changeOrder.reduce(
        (count, changeId) =>
          count + (draft.changes[changeId]?.included ? 1 : 0),
        0,
      )
    : 0;
  const changeCount = draft?.changeOrder.length ?? 0;
  const applying = draft?.apply.state === "applying";
  const applied = draft?.apply.state === "success";
  const displayedCount =
    draft?.apply.state === "success"
      ? draft.apply.result.operations.length
      : changeCount;
  const controlsDisabled =
    applying || applied || Boolean(draft?.sourceInvalidated);

  const requestAnalysis = useCallback(async () => {
    const current = useAppStore.getState().mutationDrafts[scope];
    if (!current) return;
    const identity = { scope, generation: current.generation };
    if (
      analysisRun.current?.scope === scope &&
      analysisRun.current.generation === current.generation
    ) {
      return;
    }
    analysisRun.current = identity;
    setAnalysisFailure(null);
    setAnnouncement("Analyzing result editability.");
    const result = await analyzeDraft(current);
    if (
      analysisRun.current?.scope === scope &&
      analysisRun.current.generation === current.generation
    ) {
      analysisRun.current = undefined;
    }
    const latest = useAppStore.getState().mutationDrafts[scope];
    if (
      !latest ||
      latest.generation !== current.generation ||
      latest.sourceInvalidated
    ) {
      return;
    }
    if (result.kind === "ok") {
      setAnalysis(draftHandle(current), result.value);
      setAnnouncement(null);
      return;
    }
    const error = clientResultError(result);
    setAnalysisFailure({ scope, generation: current.generation, error });
    setAnnouncement(formatMutationError(error));
  }, [scope, setAnalysis]);

  const requestPreview = useCallback(async () => {
    if (previewRun.current?.scope === scope) return;
    setAnnouncement(null);
    const token = Symbol(scope);
    previewRun.current = { scope, token };

    const execute = async (
      request: MutationDraftPreviewRequest,
      mayRecover: boolean,
    ): Promise<void> => {
      const current = useAppStore.getState().mutationDrafts[request.scope];
      if (!current || current.generation !== request.generation) return;
      const result = await previewResultMutations({
        connectionId: current.connectionId,
        tabId: current.owner.tabId,
        analysisId: request.analysisId,
        plan: request.build.plan,
      });
      if (result.kind === "ok") {
        resolvePreview(request, result.value);
        setAnnouncement(null);
        return;
      }
      const error = clientResultError(result);
      if (error.kind !== "analysisExpired" || !mayRecover) {
        failPreview(request, error);
        setAnnouncement(null);
        return;
      }

      const analysis = await analyzeDraft(current);
      if (analysis.kind !== "ok") {
        failPreview(request, clientResultError(analysis));
        setAnnouncement(null);
        return;
      }
      if (!setAnalysis(draftHandle(current), analysis.value)) return;
      const retry = beginPreview(request.scope);
      if (retry) await execute(retry, false);
    };

    try {
      const request = beginPreview(scope);
      if (request) await execute(request, true);
    } finally {
      if (previewRun.current?.token === token) previewRun.current = undefined;
      const current = useAppStore.getState().mutationDrafts[scope];
      if (
        current?.analysis &&
        !current.sourceInvalidated &&
        hasIncludedChanges(current) &&
        current.preview.state === "idle" &&
        current.apply.state !== "applying"
      ) {
        queueMicrotask(() => void requestPreview());
      }
    }
  }, [beginPreview, failPreview, resolvePreview, scope, setAnalysis]);

  useEffect(() => {
    if (
      draft &&
      !draft.sourceInvalidated &&
      !draft.analysis &&
      draft.changeOrder.length > 0 &&
      !visibleAnalysisFailure
    ) {
      void requestAnalysis();
    }
  }, [draft, requestAnalysis, visibleAnalysisFailure]);

  useEffect(() => {
    if (
      draft?.analysis &&
      !draft.sourceInvalidated &&
      hasIncludedChanges(draft) &&
      draft.preview.state === "idle" &&
      draft.apply.state !== "applying"
    ) {
      void requestPreview();
    }
  }, [draft, requestPreview]);

  const handleApplyResult = useCallback(
    async (
      request: MutationDraftApplyRequest,
      result: ResultMutationClientResult<ApplyResult>,
    ) => {
      if (result.kind !== "ok") {
        const error = clientResultError(result);
        if (failApply(request, error)) {
          const settled = useAppStore.getState().mutationDrafts[request.scope];
          setAnnouncement(
            settled?.sourceInvalidated
              ? "The apply ended after the database restore invalidated its source. Its final database state cannot be safely inferred from this stale draft."
              : formatMutationError(error),
          );
        }
        return;
      }
      if (!resolveApply(request, result.value)) return;
      const affected = result.value.operations
        .map((operation) => operation.rowsAffected)
        .join(", ");
      const appliedDraft = useAppStore.getState().mutationDrafts[request.scope];
      const stale =
        appliedDraft?.owner.kind === "query" ? " Query result is stale." : "";
      setAnnouncement(
        `Applied ${result.value.operations.length} operations. Rows affected: ${affected}.${stale}`,
      );
      if (onApplySuccess) {
        try {
          await onApplySuccess(result.value);
        } catch {
          setAnnouncement("Changes applied, but refreshing the result failed.");
        }
      }
    },
    [failApply, onApplySuccess, resolveApply],
  );

  const recoverExpiredApply = useCallback(
    async (request: MutationDraftApplyRequest): Promise<void> => {
      const current = useAppStore.getState().mutationDrafts[request.scope];
      if (
        !current ||
        current.generation !== request.generation ||
        current.sourceInvalidated
      ) {
        await handleApplyResult(request, { kind: "superseded" });
        return;
      }
      const analysis = await analyzeDraft(current);
      if (analysis.kind !== "ok") {
        await handleApplyResult(request, analysis);
        return;
      }
      const candidate = await previewResultMutations({
        connectionId: current.connectionId,
        tabId: current.owner.tabId,
        analysisId: analysis.value.analysisId,
        plan: request.build.plan,
      });
      if (candidate.kind !== "ok") {
        await handleApplyResult(request, candidate);
        return;
      }
      const recovery = recoverAnalysis(
        draftHandle(current),
        analysis.value,
        candidate.value,
      );
      if (recovery === "stale") {
        await handleApplyResult(request, { kind: "superseded" });
        return;
      }
      if (recovery === "changed") {
        setAnnouncement(
          "Analysis changed the generated DML. Review the refreshed preview before applying.",
        );
        return;
      }
      const retryRequest = retryApplyAfterRecovery(draftHandle(current));
      if (!retryRequest) {
        await handleApplyResult(request, { kind: "superseded" });
        return;
      }
      const retry = await applyWithSafetyConfirmation(
        {
          connectionId: current.connectionId,
          tabId: current.owner.tabId,
          analysisId: retryRequest.analysisId,
          plan: retryRequest.build.plan,
        },
        () => {
          const latest = useAppStore.getState().mutationDrafts[request.scope];
          return (
            latest?.generation === retryRequest.generation &&
            !latest.sourceInvalidated &&
            latest.apply.state === "applying"
          );
        },
      );
      await handleApplyResult(retryRequest, retry);
    },
    [handleApplyResult, recoverAnalysis, retryApplyAfterRecovery],
  );

  const handleApply = useCallback(async () => {
    const request = beginApply(scope);
    if (!request) return;
    const current = useAppStore.getState().mutationDrafts[scope];
    if (!current || current.generation !== request.generation) return;
    setCancelRequested(false);
    setAnnouncement("Applying included changes in one transaction.");
    const result = await applyWithSafetyConfirmation(
      {
        connectionId: current.connectionId,
        tabId: current.owner.tabId,
        analysisId: request.analysisId,
        plan: request.build.plan,
      },
      () => {
        const latest = useAppStore.getState().mutationDrafts[scope];
        return (
          latest?.generation === request.generation &&
          !latest.sourceInvalidated &&
          latest.apply.state === "applying"
        );
      },
    );
    if (result.kind === "error" && result.error.kind === "analysisExpired") {
      await recoverExpiredApply(request);
      return;
    }
    await handleApplyResult(request, result);
  }, [beginApply, handleApplyResult, recoverExpiredApply, scope]);

  const handleCancel = useCallback(async () => {
    const current = useAppStore.getState().mutationDrafts[scope];
    if (!current || current.apply.state !== "applying") return;
    setCancelPending(true);
    const result = await cancelResultMutation({
      connectionId: current.connectionId,
      tabId: current.owner.tabId,
    });
    setCancelPending(false);
    if (result.kind === "ok") {
      setCancelRequested(result.value.cancelRequested);
      setAnnouncement(
        result.value.cancelRequested
          ? "Cancellation requested. Waiting for the apply result; changes may already have committed."
          : "No cancellable mutation request was found.",
      );
      return;
    }
    setAnnouncement(formatMutationError(clientResultError(result)));
  }, [scope]);

  const handleCopy = useCallback(async (text: string, label: string) => {
    try {
      await navigator.clipboard.writeText(text);
      setAnnouncement(`Copied ${label}.`);
    } catch {
      setAnnouncement(`Could not copy ${label}.`);
    }
  }, []);

  const handleRefresh = useCallback(async () => {
    setAnnouncement("Refreshing result data.");
    const succeeded = await callPanelCallback(onRefresh);
    setAnnouncement(
      succeeded ? "Result refresh requested." : "Result refresh failed.",
    );
  }, [onRefresh]);

  const handleRerun = useCallback(async () => {
    setAnnouncement("Re-running the query result.");
    const succeeded = await callPanelCallback(onRerunQuery);
    setAnnouncement(
      succeeded ? "Query re-run requested." : "Query re-run failed.",
    );
  }, [onRerunQuery]);

  const status = announcement ?? statusForDraft(draft, visibleAnalysisFailure);
  const readyPreview = draft?.preview.state === "ready" ? draft.preview : null;
  const canApply =
    Boolean(readyPreview?.reviewed) &&
    includedCount > 0 &&
    !applying &&
    !applied;

  return (
    <aside
      aria-label="Mutation review"
      className="flex h-full min-h-0 w-full flex-col text-xs text-foreground"
    >
      {draft?.sourceInvalidated ? (
        <p
          role="alert"
          className="border-b border-border-subtle p-3 text-warning"
        >
          {applying
            ? "The database was restored while this apply is still finishing. Its changes may already have committed. Wait for the final result before discarding."
            : applied
              ? "The database was restored while these changes were applying. The apply reported success, but these preserved edits are stale. Discard them and reload before editing again."
              : draft.apply.state === "failed"
                ? "The database was restored while these changes were applying. The final database state may include committed changes. Discard these stale edits and reload before editing again."
                : "The database was restored. These staged edits are preserved but stale and cannot be changed or applied. Discard them and reload before editing again."}
        </p>
      ) : null}
      <header className="flex min-h-12 shrink-0 items-center gap-2 border-b border-border-subtle px-3">
        <div className="min-w-0 flex-1">
          <h1 className="truncate text-sm font-semibold">
            {applied ? "Applied" : "Review"} {displayedCount}{" "}
            {displayedCount === 1 ? "change" : "changes"}
          </h1>
          <p className="m-0 text-2xs text-text-muted">
            All included changes apply in one transaction
          </p>
        </div>
        <Button
          type="button"
          size="icon-sm"
          variant="ghost"
          onClick={onClose}
          disabled={applying}
          aria-label="Close mutation review"
          title={applying ? "Wait for apply to finish or cancel it" : "Close"}
        >
          <IconX />
        </Button>
      </header>

      <div className="shrink-0 border-b border-border-subtle px-3 py-2 text-2xs text-text-muted">
        <span>Resolved {targets.length === 1 ? "target" : "targets"} </span>
        {targets.length > 0 ? (
          targets.map((target, index) => (
            <span key={target}>
              {index > 0 ? ", " : null}
              <code className="font-mono text-foreground">{target}</code>
            </span>
          ))
        ) : (
          <span className="text-foreground">none</span>
        )}
      </div>

      <div className="min-h-0 flex-1 overflow-y-auto [content-visibility:auto]">
        {draft ? (
          <>
            <MutationChangeList
              groups={groups}
              disabled={controlsDisabled}
              onIncludedChange={(changeId, included) => {
                if (setIncluded(scope, changeId, included)) {
                  setAnnouncement(null);
                }
              }}
              onRevert={(changeId) => {
                if (revertChange(scope, changeId)) {
                  setAnnouncement(null);
                }
              }}
            />
            {readyPreview ? (
              <DmlPreview preview={readyPreview.result} onCopy={handleCopy} />
            ) : draft.preview.state === "error" ? (
              <div className="m-3 border border-danger/40 p-3 text-2xs text-danger">
                <p className="m-0">
                  {formatMutationError(draft.preview.error)}
                </p>
                <Button
                  type="button"
                  size="sm"
                  variant="ghost"
                  className="mt-2"
                  onClick={() => void requestPreview()}
                >
                  <IconRefresh /> Retry preview
                </Button>
              </div>
            ) : null}
          </>
        ) : (
          <div className="p-4 text-text-muted">
            This mutation draft is no longer available.
          </div>
        )}
      </div>

      <footer className="shrink-0 border-t border-border-subtle px-3 py-2">
        <output
          aria-live="polite"
          className="mb-2 text-2xs text-text-secondary"
        >
          {status}
        </output>
        {visibleAnalysisFailure ? (
          <Button
            type="button"
            size="sm"
            variant="ghost"
            className="mb-2"
            onClick={() => void requestAnalysis()}
          >
            <IconRefresh /> Retry analysis
          </Button>
        ) : null}
        <div className="flex flex-wrap items-center gap-1.5">
          <Button
            type="button"
            size="sm"
            variant="ghost"
            disabled={controlsDisabled || changeCount === 0}
            onClick={() => {
              if (revertAll(scope)) {
                setAnnouncement("All staged changes reverted.");
              }
            }}
          >
            Revert all
          </Button>
          {draft?.apply.state === "failed" &&
          onRefresh &&
          !draft.sourceInvalidated ? (
            <Button
              type="button"
              size="sm"
              variant="ghost"
              onClick={() => void handleRefresh()}
            >
              <IconRefresh /> Refresh rows
            </Button>
          ) : null}
          <span className="min-w-0 flex-1" />
          {applying ? (
            <Button
              type="button"
              size="sm"
              variant="destructive"
              disabled={cancelPending || cancelRequested}
              onClick={() => void handleCancel()}
            >
              {cancelPending
                ? "Requesting cancel"
                : cancelRequested
                  ? "Cancel requested"
                  : "Cancel apply"}
            </Button>
          ) : null}
          {readyPreview &&
          !readyPreview.reviewed &&
          !draft?.sourceInvalidated ? (
            <Button
              type="button"
              size="sm"
              variant="secondary"
              onClick={() => {
                if (acknowledgePreview(scope)) {
                  setAnnouncement("Refreshed DML reviewed and ready to apply.");
                }
              }}
            >
              Review refreshed DML
            </Button>
          ) : null}
          {draft?.apply.state === "success" &&
          draft.owner.kind === "query" &&
          onRerunQuery ? (
            <Button
              type="button"
              size="sm"
              variant="secondary"
              onClick={() => void handleRerun()}
            >
              Re-run result
            </Button>
          ) : null}
          <Button
            type="button"
            size="sm"
            disabled={!canApply}
            onClick={() => void handleApply()}
          >
            {applying
              ? "Applying"
              : applied
                ? "Applied"
                : `Apply ${includedCount} ${includedCount === 1 ? "change" : "changes"}`}
          </Button>
        </div>
      </footer>
    </aside>
  );
}

/**
 * The one right-panel chrome for mutation review (P6, D16): both the
 * query surface and the table editor render the review inside this
 * aside so panel width and surface are identical everywhere. On
 * narrow windows it overlays the grid instead of squeezing it.
 */
export function MutationReviewAside({
  children,
}: {
  children: React.ReactNode;
}) {
  return (
    <aside className="flex h-full w-[min(420px,42vw)] shrink-0 border-l border-border-subtle bg-surface-window max-[820px]:absolute max-[820px]:inset-y-0 max-[820px]:right-0 max-[820px]:z-40 max-[820px]:w-full">
      {children}
    </aside>
  );
}
