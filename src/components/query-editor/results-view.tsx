/**
 * Results pane (DESIGN-SYSTEM §5.2) — the bottom half of the editor
 * split. Owns the Results/Explain toggle, result-set chips, pinned
 * results, and the wired Export/Copy format menus. Server notices and
 * the query log stream to the global console dock (§5.6), not here.
 */

import {
  IconChevronDown,
  IconCopy,
  IconDownload,
  IconPin,
  IconX,
} from "@tabler/icons-react";
import { useMemo } from "react";
import { toast } from "sonner";

import { DataGrid } from "@/components/data-grid";
import { ExplainView } from "@/components/query-editor/explain/explain-view";
import { ElapsedTime } from "@/components/query-editor/results-status-strip";
import { Button } from "@/components/ui/button";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { Segmented } from "@/components/ui/segmented";
import {
  EmptyState,
  ErrorState,
  LoadingState,
} from "@/components/ui/state-panel";
import {
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from "@/components/ui/tooltip";
import { downloadFile } from "@/lib/download";
import {
  type ExportTable,
  toCsv,
  toJson,
  toMarkdown,
  toSqlInserts,
  toTxt,
} from "@/lib/export";
import { flattenResultSetRows } from "@/lib/query-session-budget";
import { shortcutKeys } from "@/lib/shortcuts";
import type { QueryPreviewData, QuerySessionState } from "@/lib/store";
import { useAppStore } from "@/lib/store";
import { browseCellsToGrid } from "@/lib/table-browse";
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

/** A pinned snapshot of one result set — not overwritten by later runs. */
export type PinnedResult = {
  id: string;
  label: string;
  columns: string[];
  rows: Array<Array<string | null>>;
  runtime: string;
  rowCount: string;
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
  resultIndex: number;
  onResultIndexChange: (index: number) => void;
  mutationGrid?: QueryMutationGridProps;
  onSwitchBudgetOwner?: (tabId: string) => void;
  onReleaseBudgetOwner?: (tabId: string) => void;
  pinnedResults?: PinnedResult[];
  activePinnedId?: string | null;
  onSelectPinned?: (id: string | null) => void;
  onPinResult?: (pinned: PinnedResult) => void;
  onUnpinResult?: (id: string) => void;
  /** Collapse the pane to the status strip (§5.3). */
  onCollapse?: () => void;
}

export interface QueryMutationGridProps {
  edits: Record<number, Record<number, string>>;
  readOnly: boolean;
  onCellEdit: (rowIndex: number, colIndex: number, value: string) => void;
  onEditIntent: (rowIndex: number, colIndex: number) => void;
  getCellReadOnlyReason: (
    rowIndex: number,
    colIndex: number,
  ) => string | undefined;
  statusCopy: string;
  statusTone: "muted" | "success" | "warning";
  stale: boolean;
  onRerun: () => void;
}

/** §5.2 Export/Copy format menu: CSV / JSON / TSV / INSERT / Markdown. */
const RESULT_FORMATS = [
  { id: "csv", label: "CSV", ext: "csv", mime: "text/csv" },
  { id: "json", label: "JSON", ext: "json", mime: "application/json" },
  { id: "tsv", label: "TSV", ext: "tsv", mime: "text/tab-separated-values" },
  { id: "insert", label: "INSERT", ext: "sql", mime: "application/sql" },
  { id: "markdown", label: "Markdown", ext: "md", mime: "text/markdown" },
] as const;

type ResultFormat = (typeof RESULT_FORMATS)[number];

function renderResultText(
  format: ResultFormat["id"],
  table: ExportTable,
  tableName: string,
): string {
  switch (format) {
    case "csv":
      return toCsv(table, { nullAs: "NULL" });
    case "json":
      return toJson(table, { pretty: true });
    case "tsv":
      return toTxt(table, "NULL");
    case "insert":
      return toSqlInserts(table, { tableName: tableName || "query_result" });
    case "markdown":
      return toMarkdown(table, "NULL");
  }
}

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
  resultIndex,
  onResultIndexChange,
  mutationGrid,
  onSwitchBudgetOwner,
  onReleaseBudgetOwner,
  pinnedResults = [],
  activePinnedId = null,
  onSelectPinned,
  onPinResult,
  onUnpinResult,
  onCollapse,
}: QueryResultsViewProps) {
  const appendConsoleEvent = useAppStore((state) => state.appendConsoleEvent);
  const execution = session?.execution;
  const selectedResult = execution?.resultSets[resultIndex];
  const activePinned =
    pinnedResults.find((pinned) => pinned.id === activePinnedId) ?? null;

  const selectedPreview = useMemo(() => {
    if (activePinned) {
      return {
        columns: activePinned.columns,
        rows: browseCellsToGrid(activePinned.rows),
        runtime: activePinned.runtime,
        rowCount: activePinned.rowCount,
        cache: "Cold",
      };
    }
    return session && execution && selectedResult && !execution.tombstone
      ? {
          columns: selectedResult.columns.map((column) => column ?? ""),
          rows: browseCellsToGrid(flattenResultSetRows(selectedResult)),
          runtime: `${execution.runtimeMs} ms`,
          rowCount: String(selectedResult.rowCount),
          cache: "Cold",
        }
      : session
        ? null
        : preview;
  }, [activePinned, execution, preview, selectedResult, session]);

  const buildExportTable = (): ExportTable | null => {
    if (activePinned) {
      return { columns: activePinned.columns, rows: activePinned.rows };
    }
    if (session && execution && selectedResult && !execution.tombstone) {
      return {
        columns: selectedResult.columns.map((column) => column ?? ""),
        rows: flattenResultSetRows(selectedResult),
      };
    }
    if (preview) return { columns: preview.columns, rows: preview.rows };
    return null;
  };

  const handleCopy = async (format: ResultFormat) => {
    const table = buildExportTable();
    if (!table) return;
    const text = renderResultText(format.id, table, exportFilenameBase);
    try {
      await navigator.clipboard.writeText(text);
      toast.success(
        `Copied ${table.rows.length} ${pluralRows(table.rows.length)} as ${format.label}`,
      );
    } catch {
      toast.error("Copy failed.");
    }
  };

  const handleExport = (format: ResultFormat) => {
    const table = buildExportTable();
    if (!table) return;
    const text = renderResultText(format.id, table, exportFilenameBase);
    const filename = `${exportFilenameBase || "query-results"}.${format.ext}`;
    downloadFile(filename, `${format.mime};charset=utf-8`, text);
    appendConsoleEvent({
      severity: "info",
      source: "task",
      message: `Exported ${table.rows.length} ${pluralRows(table.rows.length)} as ${format.label} — ${filename}`,
    });
    toast.success(`Exported ${filename}`);
  };

  const handlePin = () => {
    if (!onPinResult) return;
    const table = buildExportTable();
    if (!table || activePinned) return;
    const rowCount = selectedResult
      ? String(selectedResult.rowCount)
      : String(table.rows.length);
    onPinResult({
      id: crypto.randomUUID(),
      label: `Pinned · ${rowCount} ${pluralRows(table.rows.length)}`,
      columns: table.columns,
      rows: table.rows,
      runtime: execution
        ? `${execution.runtimeMs} ms`
        : (preview?.runtime ?? "—"),
      rowCount,
    });
  };

  const hasExportableResult = Boolean(
    activePinned ||
    (session && execution && selectedResult && !execution.tombstone) ||
    (!session && preview?.rows.length),
  );

  const returnedRowCount = session
    ? (execution?.tombstone?.rowCount ??
      execution?.resultSets.reduce(
        (total, result) => total + result.rowCount,
        0,
      ) ??
      0)
    : (preview?.rowCount ?? 0);
  const returnedRuntime = session
    ? execution
      ? `${execution.runtimeMs} ms`
      : "—"
    : (preview?.runtime ?? "—");
  const omittedRows = execution?.omittedRows ?? 0;

  const showChips =
    (execution?.resultSets.length ?? 0) > 1 || pinnedResults.length > 0;

  return (
    <div className="flex min-h-0 flex-1 flex-col bg-surface-app">
      <div className="flex h-(--h-tab) shrink-0 items-center gap-2 border-b border-border-subtle bg-surface-window px-2">
        <Segmented<ResultsView>
          value={view}
          onChange={onViewChange}
          options={[
            { id: "results", label: "Results" },
            { id: "explain", label: "Explain" },
          ]}
        />
        {showChips ? (
          <div className="flex min-w-0 items-center gap-1 overflow-x-auto">
            {pinnedResults.map((pinned) => (
              <span
                key={pinned.id}
                className={cn(
                  "flex shrink-0 items-center gap-1 rounded-sm border border-border-subtle pl-2 text-2xs text-text-muted",
                  pinned.id === activePinnedId &&
                    "border-accent text-foreground",
                )}
              >
                <button
                  type="button"
                  onClick={() => onSelectPinned?.(pinned.id)}
                  aria-current={
                    pinned.id === activePinnedId ? "true" : undefined
                  }
                  className="whitespace-nowrap py-1"
                >
                  <IconPin className="mr-1 inline size-3" />
                  {pinned.label}
                </button>
                <Button
                  size="icon-xs"
                  variant="ghost"
                  aria-label={`Unpin ${pinned.label}`}
                  onClick={() => onUnpinResult?.(pinned.id)}
                  className="size-5"
                >
                  <IconX />
                </Button>
              </span>
            ))}
            {execution && execution.resultSets.length > 1
              ? execution.resultSets.map((result, index) => (
                  <button
                    key={result.index}
                    type="button"
                    onClick={() => {
                      onSelectPinned?.(null);
                      onResultIndexChange(index);
                    }}
                    aria-current={
                      !activePinned && index === resultIndex
                        ? "true"
                        : undefined
                    }
                    className="shrink-0 whitespace-nowrap rounded-sm border border-border-subtle px-2 py-1 text-2xs text-text-muted aria-[current=true]:border-accent aria-[current=true]:text-foreground"
                  >
                    {index + 1} ·{" "}
                    {result.columns.length
                      ? `${result.rowCount} ${pluralRows(result.rowCount)}`
                      : "command"}
                    {result.partial ? " · partial" : ""}
                  </button>
                ))
              : null}
          </div>
        ) : null}
        <div className="ml-auto flex shrink-0 items-center gap-1">
          <span className="px-1 text-2xs tabular-nums text-text-muted">
            {isRunning ? (
              <>
                Running · <ElapsedTime />
              </>
            ) : (
              <>
                {returnedRowCount} {pluralRows(Number(returnedRowCount) || 0)} ·{" "}
                {returnedRuntime}
                {omittedRows > 0 ? ` · ${omittedRows} omitted` : ""}
              </>
            )}
          </span>
          {onPinResult ? (
            <Tooltip>
              <TooltipTrigger
                render={
                  <Button
                    size="icon-xs"
                    variant="ghost"
                    aria-label="Pin this result"
                    disabled={!hasExportableResult || Boolean(activePinned)}
                    onClick={handlePin}
                  />
                }
              >
                <IconPin />
              </TooltipTrigger>
              <TooltipContent>
                Pin this result — the next run won't overwrite it
              </TooltipContent>
            </Tooltip>
          ) : null}
          <DropdownMenu>
            <Tooltip>
              <TooltipTrigger
                render={
                  <DropdownMenuTrigger
                    aria-label="Export results"
                    disabled={!hasExportableResult}
                    className="inline-flex size-6 items-center justify-center rounded-sm text-text-muted hover:bg-surface-panel-elevated hover:text-foreground disabled:pointer-events-none disabled:opacity-50"
                  />
                }
              >
                <IconDownload className="size-3.5" />
              </TooltipTrigger>
              <TooltipContent>Export results</TooltipContent>
            </Tooltip>
            <DropdownMenuContent align="end">
              {RESULT_FORMATS.map((format) => (
                <DropdownMenuItem
                  key={format.id}
                  onClick={() => handleExport(format)}
                >
                  {format.label}
                </DropdownMenuItem>
              ))}
            </DropdownMenuContent>
          </DropdownMenu>
          <DropdownMenu>
            <Tooltip>
              <TooltipTrigger
                render={
                  <DropdownMenuTrigger
                    aria-label="Copy results"
                    disabled={!hasExportableResult}
                    className="inline-flex size-6 items-center justify-center rounded-sm text-text-muted hover:bg-surface-panel-elevated hover:text-foreground disabled:pointer-events-none disabled:opacity-50"
                  />
                }
              >
                <IconCopy className="size-3.5" />
              </TooltipTrigger>
              <TooltipContent>Copy results</TooltipContent>
            </Tooltip>
            <DropdownMenuContent align="end">
              {RESULT_FORMATS.map((format) => (
                <DropdownMenuItem
                  key={format.id}
                  onClick={() => void handleCopy(format)}
                >
                  {format.label}
                </DropdownMenuItem>
              ))}
            </DropdownMenuContent>
          </DropdownMenu>
          {onCollapse ? (
            <Tooltip>
              <TooltipTrigger
                render={
                  <Button
                    size="icon-xs"
                    variant="ghost"
                    aria-label="Collapse results pane"
                    onClick={onCollapse}
                  />
                }
              >
                <IconChevronDown />
              </TooltipTrigger>
              <TooltipContent kbd={shortcutKeys("toggle-results")}>
                Collapse results pane
              </TooltipContent>
            </Tooltip>
          ) : null}
        </div>
      </div>

      {errorMessage && !session?.policyRefusal ? (
        <ErrorState message={errorMessage} className="shrink-0" />
      ) : null}

      {session?.policyRefusal ? (
        <ErrorState message={session.policyRefusal} className="shrink-0" />
      ) : null}

      {mutationGrid?.statusCopy && !activePinned ? (
        <div
          data-testid="query-mutation-status"
          className={cn(
            "flex shrink-0 items-center gap-2 border-b border-border-subtle px-3 py-1.5 text-xs",
            mutationGrid.statusTone === "success" && "text-success",
            mutationGrid.statusTone === "warning" && "text-warning",
            mutationGrid.statusTone === "muted" && "text-text-muted",
          )}
        >
          <span>{mutationGrid.statusCopy}</span>
          {mutationGrid.stale ? (
            <Button size="sm" variant="ghost" onClick={mutationGrid.onRerun}>
              Re-run result
            </Button>
          ) : null}
        </div>
      ) : null}

      {session && session.budgetOwners.length > 0 && !activePinned ? (
        <BudgetOwnersBanner
          owners={session.budgetOwners}
          onSwitchBudgetOwner={onSwitchBudgetOwner}
          onReleaseBudgetOwner={onReleaseBudgetOwner}
        />
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
          mutationGrid={activePinned ? undefined : mutationGrid}
          readOnly={Boolean(activePinned)}
        />
      </div>
    </div>
  );
}

function pluralRows(count: number): string {
  return count === 1 ? "row" : "rows";
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
  mutationGrid,
  readOnly,
}: {
  view: ResultsView;
  preview: QueryPreviewData | null;
  session?: QuerySessionState;
  explainPlan: ExplainPlanData | null;
  currentEdits: Record<number, Record<number, string>>;
  exportFilenameBase: string;
  isRunning: boolean;
  errorMessage: string | null;
  onCellEdit: (rowIndex: number, colIndex: number, value: string) => void;
  mutationGrid?: QueryMutationGridProps;
  readOnly: boolean;
}) {
  if (view === "explain") {
    return (
      <ExplainContent
        explainPlan={explainPlan}
        isRunning={isRunning}
        errorMessage={errorMessage}
      />
    );
  }
  if (isRunning && !preview?.rows.length) {
    return <LoadingState label="Running query…" />;
  }
  if (!readOnly && session?.execution?.tombstone) return <ReleasedState />;
  if (preview?.rows.length) {
    return (
      <DataGrid
        data={preview.rows}
        columns={preview.columns}
        edits={readOnly ? {} : (mutationGrid?.edits ?? currentEdits)}
        onEdit={readOnly ? undefined : (mutationGrid?.onCellEdit ?? onCellEdit)}
        onEditIntent={mutationGrid?.onEditIntent}
        getCellReadOnlyReason={mutationGrid?.getCellReadOnlyReason}
        readOnly={readOnly || mutationGrid?.readOnly}
        exportFilenameBase={exportFilenameBase}
      />
    );
  }
  if (errorMessage) {
    return <EmptyState title="Query did not return results" />;
  }
  return <EmptyState title="Run the query to see results" />;
}

function BudgetOwnersBanner({
  owners,
  onSwitchBudgetOwner,
  onReleaseBudgetOwner,
}: {
  owners: NonNullable<QuerySessionState["budgetOwners"]>;
  onSwitchBudgetOwner?: (tabId: string) => void;
  onReleaseBudgetOwner?: (tabId: string) => void;
}) {
  return (
    <div className="shrink-0 border-b border-warning/40 px-3 py-2 text-xs">
      <div className="text-warning">
        Result memory is full. Release another tab's results to continue
        retaining rows.
      </div>
      {owners.map((owner) => (
        <div key={owner.tabId} className="mt-1 flex items-center gap-2">
          <span className="min-w-0 flex-1 truncate text-text-secondary">
            {owner.label} · {Math.ceil(owner.retainedBytes / 1048576)} MiB
          </span>
          <Button
            size="xs"
            variant="ghost"
            onClick={() => onSwitchBudgetOwner?.(owner.tabId)}
          >
            Switch
          </Button>
          <Button
            size="xs"
            variant="ghost"
            onClick={() => onReleaseBudgetOwner?.(owner.tabId)}
          >
            Release results
          </Button>
        </div>
      ))}
    </div>
  );
}

function ReleasedState() {
  return (
    <EmptyState title="This result display was released to stay within the 128 MiB global budget. Summary retained. Rerun the query to view rows again." />
  );
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
  if (isRunning) return <LoadingState label="Running EXPLAIN…" />;
  if (errorMessage) {
    return <EmptyState title="Query did not return results" />;
  }
  if (!explainPlan) {
    return (
      <EmptyState
        title="Explain plan"
        description="Run EXPLAIN to render the active statement's plan tree here."
      />
    );
  }
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
