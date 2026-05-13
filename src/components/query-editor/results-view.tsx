import {
  IconAlertCircle,
  IconCopy,
  IconDownload,
  IconLoader2,
  IconRoute,
  IconTerminal2,
} from "@tabler/icons-react";

import { DataGrid } from "@/components/data-grid";
import { Button } from "@/components/ui/button";
import type { QueryPreviewData } from "@/lib/store";
import { cn } from "@/lib/utils";

export type ResultsView = "results" | "explain";

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
  explainPlan: ExplainPlanData | null;
  currentEdits: Record<number, Record<number, string>>;
  exportFilenameBase: string;
  isRunning: boolean;
  errorMessage: string | null;
  onCellEdit: (rowIndex: number, colIndex: number, value: string) => void;
}

const TABS: ReadonlyArray<{ id: ResultsView; label: string }> = [
  { id: "results", label: "Results" },
  { id: "explain", label: "Explain" },
];

export function QueryResultsView({
  view,
  onViewChange,
  preview,
  explainPlan,
  currentEdits,
  exportFilenameBase,
  isRunning,
  errorMessage,
  onCellEdit,
}: QueryResultsViewProps) {
  return (
    <div className="flex min-h-0 flex-1 flex-col bg-surface-app">
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
                  <span className="absolute inset-x-2 bottom-0 h-0.5 rounded-full bg-accent-green" />
                ) : null}
              </button>
            );
          })}
        </div>
        <div className="flex items-center gap-2 text-[0.625rem] text-text-muted">
          <span className="dbunk-optional-label">
            Returned {preview?.rowCount ?? 0} rows in {preview?.runtime ?? "—"}
          </span>
        </div>
        <div className="ml-auto flex items-center gap-1">
          <Button size="icon-sm" variant="ghost" aria-label="Download results">
            <IconDownload className="size-3.5" />
          </Button>
          <Button size="icon-sm" variant="ghost" aria-label="Copy results">
            <IconCopy className="size-3.5" />
          </Button>
        </div>
      </div>

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
          preview={preview}
          explainPlan={explainPlan}
          currentEdits={currentEdits}
          exportFilenameBase={exportFilenameBase}
          isRunning={isRunning}
          errorMessage={errorMessage}
          onCellEdit={onCellEdit}
        />
      </div>
    </div>
  );
}

function ResultsContent({
  view,
  preview,
  explainPlan,
  currentEdits,
  exportFilenameBase,
  isRunning,
  errorMessage,
  onCellEdit,
}: Omit<QueryResultsViewProps, "onViewChange">) {
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
  return <ExplainPlanTree data={explainPlan} />;
}

function ExplainEmptyState() {
  return (
    <div className="flex h-full items-center justify-center p-6">
      <div className="max-w-md rounded-lg border border-dashed border-border-subtle bg-surface-panel/40 p-6 text-center">
        <div className="mx-auto mb-3 flex size-10 items-center justify-center rounded-full bg-accent-green/10 text-accent-green">
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

function ExplainPlanTree({
  data,
}: {
  data: Extract<ExplainPlanData, { kind: "json" }>;
}) {
  return (
    <div className="flex h-full min-h-0 flex-col overflow-hidden">
      <div className="grid shrink-0 gap-2 border-b border-border-subtle bg-surface-window p-3 text-xs sm:grid-cols-3">
        <Metric label="Runtime" value={`${data.runtimeMs} ms`} />
        <Metric
          label="Planning"
          value={data.planningMs === null ? "—" : `${data.planningMs} ms`}
        />
        <Metric
          label="Execution"
          value={data.executionMs === null ? "—" : `${data.executionMs} ms`}
        />
      </div>
      <div className="min-h-0 flex-1 overflow-auto p-3">
        <ExplainNode node={data.root} depth={0} />
      </div>
    </div>
  );
}

function ExplainNode({
  node,
  depth,
}: {
  node: ExplainPlanNode;
  depth: number;
}) {
  return (
    <div
      className={cn(
        "relative rounded-sm border border-border-subtle bg-surface-window p-3",
        depth > 0 ? "mt-2" : "",
      )}
      style={{ marginLeft: depth > 0 ? 16 : 0 }}
    >
      <div className="flex flex-wrap items-center gap-2">
        <span className="text-sm font-semibold text-foreground">
          {node.nodeType}
        </span>
        {node.relation ? (
          <span className="rounded-sm bg-surface-panel px-1.5 py-0.5 font-mono text-[0.6875rem] text-text-secondary">
            {node.relation}
          </span>
        ) : null}
        {node.alias && node.alias !== node.relation ? (
          <span className="text-[0.6875rem] text-text-muted">
            alias {node.alias}
          </span>
        ) : null}
      </div>
      <div className="mt-2 grid gap-2 text-[0.6875rem] text-text-muted sm:grid-cols-4">
        <Metric label="Cost" value={range(node.startupCost, node.totalCost)} />
        <Metric label="Plan rows" value={formatNumber(node.planRows)} />
        <Metric
          label="Actual time"
          value={range(node.actualStartupTime, node.actualTotalTime, " ms")}
        />
        <Metric
          label="Actual rows"
          value={[
            formatNumber(node.actualRows),
            node.actualLoops === undefined
              ? null
              : `${node.actualLoops.toLocaleString()} loops`,
          ]
            .filter(Boolean)
            .join(" · ")}
        />
      </div>
      {node.buffers.length > 0 ? (
        <div className="mt-2 flex flex-wrap gap-1">
          {node.buffers.map((buffer) => (
            <span
              key={buffer}
              className="rounded-sm border border-border-subtle bg-surface-panel px-1.5 py-0.5 text-[0.625rem] text-text-muted"
            >
              {buffer}
            </span>
          ))}
        </div>
      ) : null}
      {node.children.length > 0 ? (
        <div className="mt-2 border-l border-border-subtle pl-2">
          {node.children.map((child, index) => (
            <ExplainNode
              key={`${child.nodeType}-${child.relation ?? "node"}-${index}`}
              node={child}
              depth={depth + 1}
            />
          ))}
        </div>
      ) : null}
    </div>
  );
}

function Metric({ label, value }: { label: string; value: string }) {
  return (
    <div className="min-w-0 rounded-sm border border-border-subtle bg-surface-panel px-2 py-1">
      <div className="text-[0.625rem] uppercase tracking-normal text-text-muted">
        {label}
      </div>
      <div className="truncate font-mono text-xs text-foreground">{value}</div>
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

function range(
  start: number | undefined,
  end: number | undefined,
  suffix = "",
): string {
  if (start === undefined && end === undefined) return "—";
  if (start === undefined) return `${formatNumber(end)}${suffix}`;
  if (end === undefined) return `${formatNumber(start)}${suffix}`;
  return `${formatNumber(start)}..${formatNumber(end)}${suffix}`;
}

function formatNumber(value: number | undefined): string {
  return value === undefined ? "—" : value.toLocaleString();
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
