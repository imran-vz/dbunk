import {
  IconAlertCircle,
  IconCopy,
  IconDownload,
  IconLoader2,
  IconRoute,
  IconTerminal2,
} from "@tabler/icons-react";
import { useState } from "react";

import { DataGrid } from "@/components/data-grid";
import { ExplainView } from "@/components/query-editor/explain/explain-view";
import { Button } from "@/components/ui/button";
import type { QueryPreviewData, QuerySessionState } from "@/lib/store";
import { cn } from "@/lib/utils";

export type ResultsView = "results" | "explain" | "output";

export type ExplainPlanData =
  | {
      kind: "json";
      runtimeMs: number;
      planningMs: number | null;
      executionMs: number | null;
      root: ExplainPlanNode;
    }
  | {
      kind: "text";
      runtimeMs: number;
      lines: string[];
    };

export type ExplainPlanNode = {
  nodeType: string;
  relation?: string;
  alias?: string;
  startupCost?: number;
  totalCost?: number;
  planRows?: number;
  actualStartupTime?: number;
  actualTotalTime?: number;
  actualRows?: number;
  actualLoops?: number;
  buffers: string[];
  children: ExplainPlanNode[];
};

interface QueryResultsViewProps {
  view: ResultsView;
  onViewChange: (view: ResultsView) => void;
  preview: QueryPreviewData | null;
  session?: QuerySessionState;
  explainPlan: ExplainPlanData | null;
  currentEdits: Record<number, Record<number, string>>;
  exportFilenameBase: string;
  isRunning: boolean;
  errorMessage: string | null;
  onCellEdit: (rowIndex: number, colIndex: number, value: string) => void;
  onSwitchBudgetOwner?: (tabId: string) => void;
  onReleaseBudgetOwner?: (tabId: string) => void;
  hideTabs?: boolean;
}

const TABS: ReadonlyArray<{ id: ResultsView; label: string }> = [
  { id: "results", label: "Results" },
  { id: "explain", label: "Explain" },
  { id: "output", label: "Output" },
];

export function QueryResultsView({
  view,
  onViewChange,
  preview,
  session,
  explainPlan,
  currentEdits,
  exportFilenameBase,
  isRunning,
  errorMessage,
  onCellEdit,
  onSwitchBudgetOwner,
  onReleaseBudgetOwner,
  hideTabs = false,
}: QueryResultsViewProps) {
  const [resultIndex, setResultIndex] = useState(0);
  const execution = session?.execution;
  const selectedResult = execution?.resultSets[resultIndex];
  const selectedPreview = selectedResult
    ? {
        columns: selectedResult.columns.map((column) => column ?? ""),
        rows: selectedResult.rows.map((row) => row.map((cell) => cell ?? "")),
        runtime: `${execution.runtimeMs} ms`,
        rowCount: String(selectedResult.rowCount),
        cache: "Cold",
      }
    : preview;
  return (
    <div className="flex min-h-0 flex-1 flex-col bg-surface-app">
      {!hideTabs ? (
        <div className="flex h-8 shrink-0 items-center gap-2 border-b border-border-subtle bg-surface-window px-3">
          <div className="flex items-end gap-1">
            {TABS.map(({ id, label }) => {
              const isActive = view === id;
              return (
                <button
                  key={id}
                  type="button"
                  aria-label={`Show ${label} view`}
                  onClick={() => onViewChange(id)}
                  aria-current={isActive ? "page" : undefined}
                  className={cn(
                    "relative h-7 px-2 text-xs font-medium transition-colors",
                    isActive
                      ? "text-foreground"
                      : "text-text-muted hover:text-foreground",
                  )}
                >
                  {label}
                  {isActive ? (
                    <span className="absolute inset-x-2 bottom-0 h-0.5 rounded-full bg-accent" />
                  ) : null}
                </button>
              );
            })}
          </div>
          <div className="flex items-center gap-2 text-[0.625rem] text-text-muted">
            <span className="dbunk-optional-label">
              Returned {preview?.rowCount ?? 0} rows in{" "}
              {preview?.runtime ?? "—"}
            </span>
          </div>
          {execution && execution.resultSets.length > 1 ? (
            <div className="flex min-w-0 items-center gap-1 overflow-x-auto">
              {execution.resultSets.map((result, index) => (
                <button
                  key={result.index}
                  type="button"
                  onClick={() => setResultIndex(index)}
                  aria-current={index === resultIndex ? "true" : undefined}
                  className="whitespace-nowrap border border-border-subtle px-2 py-1 text-[0.625rem] text-text-muted aria-[current=true]:border-accent aria-[current=true]:text-foreground"
                >
                  {index + 1} ·{" "}
                  {result.columns.length
                    ? `${result.rowCount} rows`
                    : "command"}
                  {result.partial ? " · partial" : ""}
                </button>
              ))}
            </div>
          ) : null}
          <div className="ml-auto flex items-center gap-1">
            <Button
              size="icon-sm"
              variant="ghost"
              aria-label="Download results"
            >
              <IconDownload className="size-3.5" />
            </Button>
            <Button size="icon-sm" variant="ghost" aria-label="Copy results">
              <IconCopy className="size-3.5" />
            </Button>
          </div>
        </div>
      ) : null}

      {errorMessage ? (
        <div
          role="alert"
          className="flex shrink-0 items-start gap-2 border-b border-danger/30 bg-danger/10 px-3 py-2 text-xs text-danger"
        >
          <IconAlertCircle className="mt-0.5 size-3.5 shrink-0" />
          <div className="flex-1 whitespace-pre-wrap wrap-break-word font-mono">
            {errorMessage}
          </div>
        </div>
      ) : null}

      <div className="min-h-0 flex-1 overflow-hidden">
        <ResultsContent
          view={view}
          preview={selectedPreview}
          session={session}
          explainPlan={explainPlan}
          currentEdits={currentEdits}
          exportFilenameBase={exportFilenameBase}
          isRunning={isRunning}
          errorMessage={errorMessage}
          onCellEdit={onCellEdit}
          onSwitchBudgetOwner={onSwitchBudgetOwner}
          onReleaseBudgetOwner={onReleaseBudgetOwner}
        />
      </div>
    </div>
  );
}

function ResultsContent({
  view,
  preview,
  session,
  explainPlan,
  currentEdits,
  exportFilenameBase,
  isRunning,
  errorMessage,
  onCellEdit,
  onSwitchBudgetOwner,
  onReleaseBudgetOwner,
}: Omit<QueryResultsViewProps, "onViewChange">) {
  if (view === "output") {
    const execution = session?.execution;
    if (!execution) return <EmptyState />;
    return (
      <div className="h-full overflow-auto p-3 font-mono text-xs text-foreground">
        <div
          aria-live="polite"
          className="mb-3 border-b border-border-subtle pb-2"
        >
          {execution.status} · {execution.runtimeMs} ms ·{" "}
          {execution.resultSets.length} result sets
        </div>
        {execution.notices.map((notice) => (
          <div
            key={`${notice.severity}-${notice.message}`}
            className="border-l-2 border-warning px-2 py-1"
          >
            <b>{notice.severity}</b> {notice.message}
          </div>
        ))}
        {execution.error ? (
          <div
            role="alert"
            className="mt-2 border-l-2 border-danger px-2 py-1 text-danger"
          >
            {execution.error.code ? `${execution.error.code} · ` : ""}
            {execution.error.message}
          </div>
        ) : null}
        {execution.omittedRows ||
        execution.omittedResultSets ||
        execution.omittedNotices ? (
          <div className="mt-3 text-text-muted">
            Omitted: {execution.omittedRows} rows ·{" "}
            {execution.omittedResultSets} result sets ·{" "}
            {execution.omittedNotices} notices
          </div>
        ) : null}
        {execution.tombstone ? (
          <div className="mt-3 border border-border-subtle p-3">
            This result display was released to stay within the 128 MiB global
            budget. Summary retained. Rerun the query to view rows again.
          </div>
        ) : null}
        {session?.budgetOwners.length ? (
          <div className="mt-3 border border-warning/40 p-3">
            <div>
              Result memory is full. Release another tab's results to continue
              retaining rows.
            </div>
            {session.budgetOwners.map((owner) => (
              <div key={owner.tabId} className="mt-2 flex items-center gap-2">
                <span className="min-w-0 flex-1 truncate">
                  {owner.label} · {Math.ceil(owner.retainedBytes / 1048576)} MiB
                </span>
                <Button
                  size="sm"
                  variant="ghost"
                  onClick={() => onSwitchBudgetOwner?.(owner.tabId)}
                >
                  Switch
                </Button>
                <Button
                  size="sm"
                  variant="ghost"
                  onClick={() => onReleaseBudgetOwner?.(owner.tabId)}
                >
                  Release results
                </Button>
              </div>
            ))}
          </div>
        ) : null}
      </div>
    );
  }
  if (view === "explain") {
    return (
      <ExplainContent
        explainPlan={explainPlan}
        isRunning={isRunning}
        errorMessage={errorMessage}
      />
    );
  }
  if (isRunning) return <RunningState />;
  if (preview?.rows.length) {
    return (
      <DataGrid
        data={preview.rows}
        columns={preview.columns}
        edits={currentEdits}
        onEdit={onCellEdit}
        exportFilenameBase={exportFilenameBase}
      />
    );
  }
  if (errorMessage) return <NoResultsAfterError />;
  return <EmptyState />;
}

function ExplainContent({
  explainPlan,
  isRunning,
  errorMessage,
}: {
  explainPlan: ExplainPlanData | null;
  isRunning: boolean;
  errorMessage: string | null;
}) {
  if (isRunning) return <RunningState label="Running EXPLAIN…" />;
  if (errorMessage) return <NoResultsAfterError />;
  if (!explainPlan) return <ExplainEmptyState />;
  if (explainPlan.kind === "text") {
    return (
      <div className="h-full overflow-auto p-3">
        <pre className="min-h-full whitespace-pre-wrap rounded-sm border border-border-subtle bg-surface-window p-3 font-mono text-xs text-foreground">
          {explainPlan.lines.join("\n") || "EXPLAIN returned no plan rows."}
        </pre>
      </div>
    );
  }
  return <ExplainView data={explainPlan} />;
}

function ExplainEmptyState() {
  return (
    <div className="flex h-full items-center justify-center p-6">
      <div className="max-w-md rounded-lg border border-dashed border-border-subtle bg-surface-panel/40 p-6 text-center">
        <div className="mx-auto mb-3 flex size-10 items-center justify-center rounded-full bg-accent/10 text-accent">
          <IconRoute className="size-5" />
        </div>
        <div className="text-sm font-semibold text-foreground">
          Explain plan
        </div>
        <p className="mt-1 text-xs text-text-muted">
          Run EXPLAIN to render the active statement's plan tree here.
        </p>
      </div>
    </div>
  );
}

function RunningState({ label = "Running query…" }: { label?: string }) {
  return (
    <div className="flex h-full flex-col items-center justify-center gap-2 text-text-muted">
      <IconLoader2 className="size-6 animate-spin opacity-70" />
      <div className="text-xs">{label}</div>
    </div>
  );
}

function NoResultsAfterError() {
  return (
    <div className="flex h-full flex-col items-center justify-center gap-2 text-text-muted">
      <div className="rounded-full bg-danger/10 p-3">
        <IconAlertCircle className="size-6 text-danger opacity-70" />
      </div>
      <div className="text-xs">Query did not return results</div>
    </div>
  );
}

function EmptyState() {
  return (
    <div className="flex h-full flex-col items-center justify-center gap-2 text-text-muted">
      <div className="rounded-full bg-surface-panel-elevated p-3">
        <IconTerminal2 className="size-6 opacity-50" />
      </div>
      <div className="text-xs">Run the query to see results</div>
    </div>
  );
}
