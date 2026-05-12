import {
  IconAlertCircle,
  IconCopy,
  IconDownload,
  IconLoader2,
  IconTerminal2,
} from "@tabler/icons-react";

import { DataGrid } from "@/components/data-grid";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import type { QueryPreviewData } from "@/lib/store";
import { cn } from "@/lib/utils";

export type ResultsView = "results" | "explain";

interface QueryResultsViewProps {
  view: ResultsView;
  onViewChange: (view: ResultsView) => void;
  preview: QueryPreviewData | null;
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
  currentEdits,
  exportFilenameBase,
  isRunning,
  errorMessage,
  onCellEdit,
}: Omit<QueryResultsViewProps, "onViewChange">) {
  if (view === "explain") return <ExplainPlaceholder />;
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

function ExplainPlaceholder() {
  return (
    <div className="flex h-full items-center justify-center p-6">
      <div className="max-w-md rounded-lg border border-dashed border-border-subtle bg-surface-panel/40 p-6 text-center">
        <div className="text-sm font-semibold text-foreground">
          Explain plan
        </div>
        <p className="mt-1 text-xs text-text-muted">
          The execution plan for the active statement will appear here.
        </p>
        <Badge variant="outline" className="mt-3">
          Coming soon
        </Badge>
      </div>
    </div>
  );
}

function RunningState() {
  return (
    <div className="flex h-full flex-col items-center justify-center gap-2 text-text-muted">
      <IconLoader2 className="size-6 animate-spin opacity-70" />
      <div className="text-xs">Running query…</div>
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
