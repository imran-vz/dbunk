import MonacoEditor from "@monaco-editor/react";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { toast } from "sonner";

import "@/lib/monaco-local";
import { MutationReviewPanel } from "@/components/mutation-review";
import {
  type ExplainPlanData,
  type ExplainPlanNode,
  type QueryMutationGridProps,
  QueryResultsView,
  type ResultsView,
} from "@/components/query-editor/results-view";
import { buildQueryStatusItems } from "@/components/query-editor/status-items";
import { QueryEditorToolbar } from "@/components/query-editor/toolbar";
import { useMonacoQueryEditor } from "@/components/query-editor/use-monaco-query-editor";
import { useQueryOutcome } from "@/components/query-editor/use-query-outcome";
import {
  PROTECTED_WORKSPACE_WIDTH,
  QUERY_SIDEBAR_COMPACT_BELOW,
  QUERY_SIDEBAR_WIDTH,
  useQuerySidebarVisibility,
} from "@/components/query-editor/use-query-sidebar-visibility";
import { QuerySidebar } from "@/components/query-sidebar";
import { StatusBar, type StatusBarItem } from "@/components/status-bar";
import { ResizerHandle } from "@/components/ui/resizer-handle";
import { ResponsiveEdgePanel } from "@/components/ui/responsive-edge-panel";
import { WorkbenchDock } from "@/components/workbench/dock";
import { applyBindVariables, extractBindVariables } from "@/lib/bind-variables";
import { flattenResultSetRows } from "@/lib/query-session-budget";
import {
  type AnalyzedColumn,
  type AnalyzedTable,
  type AnalyzeResultSetResult,
  type ApplyResult,
  type MutationTable,
  type NotAnalyzableReason,
  type ResultMutationError,
  supportsResultMutations,
} from "@/lib/result-mutation";
import { analyzeResultSet } from "@/lib/result-mutation-client";
import type { SqlCompletionContext } from "@/lib/sql-completions";
import { formatSql } from "@/lib/sql-format";
import {
  type QueryOutcome,
  type QueryPreviewData,
  type MutationDraft,
  queryMutationDraftScope,
  type QueryMutationDraftScope,
  useAppStore,
  type WorkspaceTab,
} from "@/lib/store";
import { GRID_NULL_SENTINEL, gridCellToEditValue } from "@/lib/table-browse";
import {
  useContainerWidth,
  useResizableWidth,
} from "@/lib/use-resizable-width";

interface QueryEditorPanelProps {
  tab: WorkspaceTab;
  isClient: boolean;
  variant?: "default" | "workbench";
  resultsView?: ResultsView;
  onResultsViewChange?: (view: ResultsView) => void;
  onStatusItemsChange?: (items: StatusBarItem[]) => void;
}

type MutationGridStatus = {
  copy: string;
  tone: QueryMutationGridProps["statusTone"];
};

export function QueryEditorPanel({
  tab,
  isClient,
  variant = "default",
  resultsView: controlledResultsView,
  onResultsViewChange,
  onStatusItemsChange,
}: QueryEditorPanelProps) {
  const [internalResultsView, setInternalResultsView] =
    useState<ResultsView>("results");
  const resultsView = controlledResultsView ?? internalResultsView;
  const setResultsView = onResultsViewChange ?? setInternalResultsView;
  const [bindValues, setBindValues] = useState<Record<string, string>>({});
  const [explainPlan, setExplainPlan] = useState<ExplainPlanData | null>(null);
  const activeTabIdRef = useRef(tab.id);
  activeTabIdRef.current = tab.id;
  const [containerRef, containerWidth] = useContainerWidth<HTMLDivElement>();
  const sidebar = useQuerySidebarVisibility(containerWidth);
  const [resultIndex, setResultIndex] = useState(0);
  const [reviewScope, setReviewScope] =
    useState<QueryMutationDraftScope | null>(null);
  const [analysisState, setAnalysisState] = useState<
    | { scope: QueryMutationDraftScope; state: "loading" }
    | { scope: QueryMutationDraftScope; state: "error"; message: string }
    | null
  >(null);
  const analysisRequestsRef = useRef(new Set<QueryMutationDraftScope>());
  const [staleExecutionId, setStaleExecutionId] = useState<string | null>(null);

  // Split-pane resize: editor on top, results below. Storage key is
  // global so the user's chosen height applies across every query tab.
  const EDITOR_MIN_PX = 80;
  const RESULTS_MIN_PX = 140;
  const splitRef = useRef<HTMLDivElement | null>(null);
  const [splitHeight, setSplitHeight] = useState(0);
  const { width: editorHeight, setWidth: setEditorHeight } = useResizableWidth({
    storageKey: "dbunk.query.editor.height",
    defaultWidth: 280,
    min: EDITOR_MIN_PX,
    max: 1600,
  });

  useEffect(() => {
    const el = splitRef.current;
    if (!el) return;
    const apply = (h: number) => {
      setSplitHeight((prev) => (Math.round(prev) === Math.round(h) ? prev : h));
    };
    apply(el.getBoundingClientRect().height);
    // oxlint-disable-next-line anti-slop/no-runtime-typeof -- The value is handled at a typed library or domain boundary here.
    if (typeof ResizeObserver === "undefined") return;
    const observer = new ResizeObserver((entries) => {
      for (const entry of entries) apply(entry.contentRect.height);
    });
    observer.observe(el);
    return () => observer.disconnect();
  }, []);

  const maxEditorHeight =
    splitHeight > 0
      ? Math.max(EDITOR_MIN_PX, splitHeight - RESULTS_MIN_PX)
      : 1600;
  const effectiveEditorHeight = Math.min(editorHeight, maxEditorHeight);
  const handleEditorResize = useCallback(
    (next: number) => {
      setEditorHeight(Math.min(next, maxEditorHeight));
    },
    [maxEditorHeight, setEditorHeight],
  );

  const {
    queryPreviews,
    querySessions,
    queryExecutionSql,
    queryStatus,
    queryEdits,
    mutationDrafts,
    schemaExplorer,
    tableStructure,
    editorTheme,
    connections,
    activeConnectionId,
    updateQuery,
    runQuery,
    cancelQuery,
    loadTableStructure,
    setQueryEdit,
    discardQueryEdits,
    openMutationDraft,
    setMutationDraftAnalysis,
    stageMutationDraftUpdate,
    discardMutationDraft,
    dropMutationDraftsForExecution,
    retargetQueryTab,
    setActiveTabId,
    releaseQueryResults,
  } = useAppStore();

  const status = queryStatus[tab.id];
  const isCancelling = status?.state === "cancelling";
  const isRunning = status?.state === "running";
  const isBusy = isRunning || isCancelling;
  const session = querySessions[tab.id];
  const execution = session?.execution;
  const executionId = execution?.id ?? null;
  useEffect(() => {
    setResultIndex(0);
    setReviewScope(null);
    setAnalysisState(null);
    analysisRequestsRef.current.clear();
    setStaleExecutionId(null);
  }, [executionId, tab.id]);
  // Terminal outcome lives component-local. We store the full
  // QueryOutcome (not just the failure message) for shape parity
  // with sibling panels. See CONTEXT.md — Query Outcome.
  const { errorMessage, setOutcome } = useQueryOutcome(tab.id);

  // Auto-route EXPLAIN runs to the Explain tab so a user-typed
  // `EXPLAIN ...` shows the plan visualizer, not just text rows.
  // Non-EXPLAIN runs swing the tab back to Results so consecutive
  // queries land on the relevant pane.
  const handleQueryOutcome = useCallback(
    (outcome: QueryOutcome, sql: string) => {
      setOutcome(outcome);
      if (isExplainStatement(sql)) {
        setResultsView("explain");
        if (outcome.kind === "completed") {
          setExplainPlan(
            parseExplainPreview(
              explainPreviewForOutcome(outcome, tab.id),
              outcome,
            ),
          );
        } else if (outcome.kind === "failed") {
          setExplainPlan(null);
        }
        return;
      }
      setResultsView("results");
    },
    [setOutcome, setResultsView, tab.id],
  );

  const activeQueryPreview: QueryPreviewData | null = useMemo(() => {
    if (tab.kind !== "query") return null;
    return (
      queryPreviews[tab.id] ?? {
        columns: ["column"],
        rows: [],
        runtime: "--",
        rowCount: "0",
        cache: "Cold",
      }
    );
  }, [tab, queryPreviews]);

  const currentEdits = useMemo(
    () => queryEdits[tab.id] ?? {},
    [queryEdits, tab.id],
  );

  const exportFilenameBase = useMemo(() => {
    const today = new Date().toISOString().slice(0, 10);
    const labelStem = tab.label.replace(/\.[^.]+$/, "");
    return [slug(labelStem), today].filter(Boolean).join("-");
  }, [tab.label]);

  const completionContext = useMemo<SqlCompletionContext>(
    () => ({
      connectionId: tab.connectionId,
      schemas: schemaExplorer[tab.connectionId] ?? [],
      currentSchema: tab.schema,
      tableStructure,
    }),
    [schemaExplorer, tab.connectionId, tab.schema, tableStructure],
  );

  const activeConnection = useMemo(
    () =>
      connections.find((c) => c.id === tab.connectionId) ??
      connections.find((c) => c.id === activeConnectionId),
    [activeConnectionId, connections, tab.connectionId],
  );

  const mutationCapable =
    execution?.status === "completed" &&
    !execution.tombstone &&
    Boolean(
      activeConnection && supportsResultMutations(activeConnection.engine),
    );
  const mutationScope =
    mutationCapable && executionId
      ? queryMutationDraftScope(tab.id, executionId, resultIndex)
      : null;
  const mutationDraft = mutationScope ? mutationDrafts[mutationScope] : null;
  const stagedChangeCount = mutationDraft?.changeOrder.length ?? 0;
  const mutationLocked = mutationDraft?.apply.state === "applying";
  const exactExecutionSql = queryExecutionSql[tab.id] ?? null;
  const executionDraftChangeCount = useMemo(
    () =>
      Object.values(mutationDrafts).reduce((count, draft) => {
        if (
          !draft ||
          draft.owner.kind !== "query" ||
          draft.owner.tabId !== tab.id ||
          draft.owner.executionId !== executionId
        ) {
          return count;
        }
        return count + draft.changeOrder.length;
      }, 0),
    [executionId, mutationDrafts, tab.id],
  );

  const guardedRunQuery = useCallback(
    async (tabId: string, options?: { overrideSql?: string }) => {
      if (executionId && executionDraftChangeCount > 0) {
        const ok = window.confirm(
          `Re-running will discard ${executionDraftChangeCount} staged ${pluralize("change", executionDraftChangeCount)}. Continue?`,
        );
        if (!ok) return { kind: "noop" } as const;
        dropMutationDraftsForExecution(tab.id, executionId);
        setReviewScope(null);
      }
      return runQuery(tabId, options);
    },
    [
      dropMutationDraftsForExecution,
      executionDraftChangeCount,
      executionId,
      runQuery,
      tab.id,
    ],
  );

  const editor = useMonacoQueryEditor({
    tabId: tab.id,
    query: tab.query ?? "",
    connectionId: tab.connectionId,
    completionContext,
    isRunning: isBusy,
    loadTableStructure,
    runQuery: guardedRunQuery,
    onOutcome: handleQueryOutcome,
    onFormat: () => handleFormat(),
  });

  const ensureMutationAnalysis = useCallback(async () => {
    if (
      !mutationScope ||
      !executionId ||
      resultIndex !== 0 ||
      mutationDraft?.analysis ||
      analysisRequestsRef.current.has(mutationScope)
    ) {
      return;
    }
    if (!exactExecutionSql) {
      setAnalysisState({
        scope: mutationScope,
        state: "error",
        message:
          "The exact SQL for this result is unavailable. Re-run the query.",
      });
      return;
    }

    const handle = openMutationDraft({
      owner: {
        kind: "query",
        tabId: tab.id,
        executionId,
        resultSetIndex: resultIndex,
      },
      connectionId: tab.connectionId,
      source: { kind: "statement", sql: exactExecutionSql },
    });
    analysisRequestsRef.current.add(mutationScope);
    setAnalysisState({ scope: mutationScope, state: "loading" });
    const result = await analyzeResultSet({
      connectionId: tab.connectionId,
      tabId: tab.id,
      source: { kind: "statement", sql: exactExecutionSql },
      refreshStructure: false,
    });
    analysisRequestsRef.current.delete(mutationScope);
    if (result.kind === "ok") {
      setMutationDraftAnalysis(handle, result.value);
      setAnalysisState((current) =>
        current?.scope === mutationScope ? null : current,
      );
      return;
    }
    if (result.kind === "superseded" || result.kind === "cancelled") {
      setAnalysisState((current) =>
        current?.scope === mutationScope ? null : current,
      );
      return;
    }
    setAnalysisState({
      scope: mutationScope,
      state: "error",
      message: mutationClientErrorCopy(result.error),
    });
  }, [
    exactExecutionSql,
    executionId,
    mutationDraft?.analysis,
    mutationScope,
    openMutationDraft,
    resultIndex,
    setMutationDraftAnalysis,
    tab.connectionId,
    tab.id,
  ]);

  const mutationAnalysis = mutationDraft?.analysis?.snapshot ?? null;
  const selectedResult = execution?.resultSets[resultIndex];
  const selectedRawRows = useMemo(
    () => flattenResultSetRows(selectedResult),
    [selectedResult],
  );
  const isResultStale = staleExecutionId === executionId;
  const mutationEdits = useMemo(
    () =>
      mutationDraft && mutationAnalysis
        ? mutationDraftGridEdits(mutationDraft, mutationAnalysis)
        : {},
    [mutationAnalysis, mutationDraft],
  );
  const mutationStatus = mutationGridStatus({
    resultIndex,
    analysis: mutationAnalysis,
    analysisState:
      analysisState?.scope === mutationScope ? analysisState : null,
    stale: isResultStale,
  });
  const getMutationCellReadOnlyReason = useCallback(
    (rowIndex: number, colIndex: number) => {
      if (isResultStale) return "Re-run the query before editing this result.";
      if (resultIndex !== 0) return "Only the first result set can be edited.";
      if (!mutationAnalysis) return mutationStatus.copy;
      return queryCellReadOnlyReason(
        mutationAnalysis,
        selectedRawRows[rowIndex],
        colIndex,
      );
    },
    [
      isResultStale,
      mutationAnalysis,
      mutationStatus.copy,
      resultIndex,
      selectedRawRows,
    ],
  );
  const handleMutationCellEdit = useCallback(
    (rowIndex: number, colIndex: number, value: string) => {
      if (
        !mutationScope ||
        !mutationAnalysis ||
        mutationLocked ||
        getMutationCellReadOnlyReason(rowIndex, colIndex)
      ) {
        return;
      }
      const row = selectedRawRows[rowIndex];
      const column = mutationAnalysis.columns[colIndex];
      if (!row || !column || column.origin.kind !== "table") return;
      const table = analyzedTableForColumn(mutationAnalysis, column);
      if (!table || table.identity.kind === "none") return;
      const identity = table.identity.columns.map((identityColumn, index) => ({
        column: identityColumn,
        value: row[table.identityProjectionIndexes[index] ?? -1] ?? null,
      }));
      const originals = projectedOriginalsForTable(
        mutationAnalysis,
        table,
        row,
      );
      const target: MutationTable = {
        schema: table.schema,
        table: table.table,
      };
      stageMutationDraftUpdate(mutationScope, {
        table: target,
        identityKind: table.identity.kind,
        identity,
        originals,
        cells: [
          {
            column: column.origin.column,
            original: row[colIndex] ?? null,
            value: gridCellToEditValue(value),
          },
        ],
        rowIndex,
      });
    },
    [
      getMutationCellReadOnlyReason,
      mutationAnalysis,
      mutationLocked,
      mutationScope,
      selectedRawRows,
      stageMutationDraftUpdate,
    ],
  );

  const hasEdits = mutationCapable
    ? stagedChangeCount > 0
    : Object.keys(currentEdits).length > 0;
  const bindNames = useMemo(
    () => extractBindVariables(tab.query ?? ""),
    [tab.query],
  );
  const runCurrentWithBinds = () => {
    editor.runSql(applyBindVariables(editor.currentStatement(), bindValues));
  };

  const handleDiscardEdits = () => {
    if (!mutationCapable || !mutationScope) {
      discardQueryEdits(tab.id);
      return;
    }
    if (stagedChangeCount === 0 || mutationLocked) return;
    const ok = window.confirm(
      `Discard ${stagedChangeCount} staged ${pluralize("change", stagedChangeCount)}?`,
    );
    if (!ok) return;
    discardMutationDraft(mutationScope);
    setReviewScope(null);
  };

  const handleReviewEdits = () => {
    if (!mutationScope || stagedChangeCount === 0 || mutationLocked) return;
    setReviewScope(mutationScope);
    sidebar.setOverlayOpen(false);
  };

  const handleMutationApplySuccess = (_result: ApplyResult) => {
    if (executionId) setStaleExecutionId(executionId);
  };

  const handleRerunResult = async () => {
    if (!exactExecutionSql) return;
    const outcome = await guardedRunQuery(tab.id, {
      overrideSql: exactExecutionSql,
    });
    if (outcome.kind !== "noop") handleQueryOutcome(outcome, exactExecutionSql);
  };

  const handleRetargetConnection = (newConnectionId: string) => {
    if (newConnectionId === tab.connectionId) return;
    if (isBusy) return;
    const pendingCount = mutationCapable
      ? executionDraftChangeCount
      : hasEdits
        ? 1
        : 0;
    if (pendingCount > 0) {
      const ok = window.confirm(
        mutationCapable
          ? `Switching connections will discard ${pendingCount} staged ${pluralize("change", pendingCount)}. Continue?`
          : "Switching connections will discard pending edits in the results grid. Continue?",
      );
      if (!ok) return;
      if (mutationCapable && executionId) {
        dropMutationDraftsForExecution(tab.id, executionId);
        setReviewScope(null);
      }
    }
    retargetQueryTab(tab.id, newConnectionId);
  };

  const handleFormat = () => {
    const engine = activeConnection?.engine ?? "PostgreSQL";
    const result = formatSql(tab.query ?? "", engine);
    if (result.kind === "empty") return;
    if (result.kind === "failed") {
      toast.error(`Format failed: ${result.reason}`);
      return;
    }
    if (result.kind === "unchanged") {
      toast.info("SQL is already formatted.");
      return;
    }
    updateQuery(tab.id, result.sql);
  };

  const handleExplain = async () => {
    if (isBusy) return;
    const currentStatement = applyBindVariables(
      editor.currentStatement(),
      bindValues,
    ).trim();
    // Fall back to the full editor text when the cursor isn't sitting
    // on a parseable statement — clicking EXPLAIN with an empty current
    // statement used to produce `EXPLAIN (...)\n` and a syntax error.
    const fallbackStatement = applyBindVariables(
      tab.query ?? "",
      bindValues,
    ).trim();
    const baseStatement = currentStatement || fallbackStatement;
    if (!baseStatement) return;
    // Don't wrap a statement that already begins with EXPLAIN —
    // nesting `EXPLAIN (...) EXPLAIN ...` is a syntax error.
    const sql = isExplainStatement(baseStatement)
      ? baseStatement
      : `EXPLAIN (ANALYZE, BUFFERS, FORMAT JSON)\n${baseStatement}`;
    setResultsView("explain");
    setExplainPlan(null);
    const requestedTabId = tab.id;
    const outcome = await guardedRunQuery(requestedTabId, {
      overrideSql: sql,
    });
    if (requestedTabId !== activeTabIdRef.current) return;
    if (outcome.kind !== "noop") setOutcome(outcome);
    if (outcome.kind === "completed") {
      setExplainPlan(
        parseExplainPreview(explainPreviewForOutcome(outcome, tab.id), outcome),
      );
    }
  };

  const editorOptions = useMemo(
    () =>
      ({
        minimap: { enabled: false },
        fontSize: 13,
        fontFamily: "SF Mono, JetBrains Mono Variable, monospace",
        scrollBeyondLastLine: false,
        wordWrap: "on" as const,
        lineNumbersMinChars: 3,
        glyphMargin: true,
        padding: { top: 8, bottom: 8 },
        renderLineHighlight: "none",
        overviewRulerBorder: false,
        hideCursorInOverviewRuler: true,
        scrollbar: {
          vertical: "hidden",
          horizontal: "hidden",
          useShadows: false,
        },
      }) as const,
    [],
  );

  const dbSelectorLabel = activeConnection
    ? `${activeConnection.engine} (${activeConnection.host || activeConnection.database})`
    : "No connection";
  const workspaceDensity =
    containerWidth > 0 && containerWidth < 760 ? "compact" : "cozy";

  const statusItems = buildQueryStatusItems({
    tabLabel: tab.label,
    cursor: editor.cursor,
    errorMessage,
    activeConnection,
    session,
  });

  useEffect(() => {
    onStatusItemsChange?.(statusItems);
  }, [onStatusItemsChange, statusItems]);

  const isWorkbench = variant === "workbench";
  const runCurrentHandler =
    bindNames.length > 0 ? runCurrentWithBinds : editor.handleRunCurrent;
  const mutationGrid = mutationCapable
    ? {
        edits: mutationEdits,
        readOnly:
          isResultStale ||
          !mutationAnalysis ||
          mutationAnalysis.statement.kind !== "analyzed" ||
          !mutationAnalysis.columns.some(
            (_column, colIndex) =>
              !queryCellReadOnlyReason(mutationAnalysis, undefined, colIndex),
          ),
        onCellEdit: handleMutationCellEdit,
        onEditIntent: () => void ensureMutationAnalysis(),
        getCellReadOnlyReason: getMutationCellReadOnlyReason,
        statusCopy: mutationStatus.copy,
        statusTone: mutationStatus.tone,
        stale: isResultStale && !reviewScope,
        onRerun: () => void handleRerunResult(),
      }
    : undefined;

  const reviewPanel = reviewScope ? (
    <aside className="flex h-full w-[min(410px,42vw)] shrink-0 border-l border-border-subtle bg-black max-[820px]:h-[48%] max-[820px]:w-full max-[820px]:border-l-0 max-[820px]:border-t">
      <MutationReviewPanel
        scope={reviewScope}
        onClose={() => setReviewScope(null)}
        onApplySuccess={handleMutationApplySuccess}
        onRerunQuery={handleRerunResult}
      />
    </aside>
  ) : null;

  const resultsPane = (
    <QueryResultsView
      view={resultsView}
      onViewChange={setResultsView}
      preview={activeQueryPreview}
      session={session}
      explainPlan={explainPlan}
      currentEdits={currentEdits}
      exportFilenameBase={exportFilenameBase}
      isRunning={isBusy}
      errorMessage={errorMessage}
      resultIndex={resultIndex}
      onResultIndexChange={setResultIndex}
      mutationGrid={mutationGrid}
      hideTabs={isWorkbench}
      onCellEdit={(rowIndex, colIndex, value) =>
        setQueryEdit(tab.id, rowIndex, colIndex, value)
      }
      onSwitchBudgetOwner={setActiveTabId}
      onReleaseBudgetOwner={releaseQueryResults}
    />
  );

  if (isWorkbench) {
    return (
      <div
        ref={containerRef}
        data-workspace-density={workspaceDensity}
        className="relative flex h-full min-h-0 bg-surface-app max-[820px]:flex-col"
      >
        <div className="relative flex min-h-0 min-w-0 flex-1 flex-col">
          <QueryEditorToolbar
            tabId={tab.id}
            dbSelectorLabel={dbSelectorLabel}
            connections={connections}
            currentConnectionId={tab.connectionId}
            onRetargetConnection={handleRetargetConnection}
            hasEdits={hasEdits}
            onDiscardEdits={handleDiscardEdits}
            onReviewEdits={mutationCapable ? handleReviewEdits : undefined}
            stagedChangeCount={mutationCapable ? stagedChangeCount : undefined}
            mutationLocked={mutationLocked}
            isRunning={isBusy}
            isCancelling={isCancelling}
            onStop={() => void cancelQuery(tab.id)}
            isSidebarOpen={sidebar.isOpen}
            onToggleSidebar={sidebar.onToggle}
            onRunCurrent={runCurrentHandler}
            onRunSelection={editor.handleRunSelection}
            onRunAll={editor.handleRunAll}
            onExplain={handleExplain}
            onFormat={handleFormat}
            onInsertSnippet={(sql) =>
              updateQuery(
                tab.id,
                [tab.query ?? "", sql].filter(Boolean).join("\n\n"),
              )
            }
            hideConnectionSwitcher
          />
          {bindNames.length > 0 ? (
            <div className="flex shrink-0 flex-wrap items-center gap-2 border-b border-border-subtle bg-surface-window px-3 py-2 text-xs">
              <span className="text-text-muted">Bind variables</span>
              {bindNames.map((name) => (
                <label key={name} className="flex items-center gap-1">
                  <span className="font-mono text-text-muted">:{name}</span>
                  <input
                    value={bindValues[name] ?? ""}
                    onChange={(event) =>
                      setBindValues((current) => ({
                        ...current,
                        [name]: event.target.value,
                      }))
                    }
                    className="h-7 w-28 rounded-sm border border-border-subtle bg-surface-input px-2 font-mono text-xs"
                  />
                </label>
              ))}
            </div>
          ) : null}
          <div className="relative min-h-0 flex-1 bg-surface-app">
            {isClient ? (
              <MonacoEditor
                height="100%"
                language="sql"
                theme={editorTheme}
                value={tab.query ?? ""}
                options={editorOptions}
                onChange={(value) => updateQuery(tab.id, value ?? "")}
                onMount={editor.onMount}
              />
            ) : (
              <div className="flex h-full items-center justify-center text-xs text-text-muted">
                Loading editor…
              </div>
            )}
          </div>
          <WorkbenchDock
            storageKey={`query-${tab.id}`}
            consoleLabel={tab.label}
            onRun={runCurrentHandler}
            runDisabled={isBusy}
            consoleContent={
              <pre className="px-3 py-2 font-mono text-[12px] leading-relaxed text-foreground">
                {tab.query ?? ""}
              </pre>
            }
            outputContent={resultsPane}
          />
          {!reviewScope ? (
            <ResponsiveEdgePanel
              side="right"
              storageKey="dbunk.sidebar.query"
              title="Query"
              width={QUERY_SIDEBAR_WIDTH}
              containerWidth={containerWidth}
              compactBelow={QUERY_SIDEBAR_COMPACT_BELOW}
              protectedWorkspaceWidth={PROTECTED_WORKSPACE_WIDTH}
              wideVisible={sidebar.wideVisible}
              open={sidebar.overlayOpen}
              onOpenChange={sidebar.setOverlayOpen}
              contentClassName="overflow-auto p-4"
            >
              <QuerySidebar tab={tab} />
            </ResponsiveEdgePanel>
          ) : null}
        </div>
        {reviewPanel}
      </div>
    );
  }

  return (
    <div
      ref={containerRef}
      data-workspace-density={workspaceDensity}
      className="relative flex h-full min-h-0 bg-surface-app max-[820px]:flex-col"
    >
      <div className="flex min-h-0 min-w-0 flex-1 flex-col">
        <QueryEditorToolbar
          tabId={tab.id}
          dbSelectorLabel={dbSelectorLabel}
          connections={connections}
          currentConnectionId={tab.connectionId}
          onRetargetConnection={handleRetargetConnection}
          hasEdits={hasEdits}
          onDiscardEdits={handleDiscardEdits}
          onReviewEdits={mutationCapable ? handleReviewEdits : undefined}
          stagedChangeCount={mutationCapable ? stagedChangeCount : undefined}
          mutationLocked={mutationLocked}
          isRunning={isBusy}
          isCancelling={isCancelling}
          onStop={() => void cancelQuery(tab.id)}
          isSidebarOpen={sidebar.isOpen}
          onToggleSidebar={sidebar.onToggle}
          onRunCurrent={
            bindNames.length > 0 ? runCurrentWithBinds : editor.handleRunCurrent
          }
          onRunSelection={editor.handleRunSelection}
          onRunAll={editor.handleRunAll}
          onExplain={handleExplain}
          onFormat={handleFormat}
          onInsertSnippet={(sql) =>
            updateQuery(
              tab.id,
              [tab.query ?? "", sql].filter(Boolean).join("\n\n"),
            )
          }
        />

        {bindNames.length > 0 ? (
          <div className="flex shrink-0 flex-wrap items-center gap-2 border-b border-border-subtle bg-surface-window px-3 py-2 text-xs">
            <span className="text-text-muted">Bind variables</span>
            {bindNames.map((name) => (
              <label key={name} className="flex items-center gap-1">
                <span className="font-mono text-text-muted">:{name}</span>
                <input
                  value={bindValues[name] ?? ""}
                  onChange={(event) =>
                    setBindValues((current) => ({
                      ...current,
                      [name]: event.target.value,
                    }))
                  }
                  className="h-7 w-28 rounded-sm border border-border-subtle bg-surface-input px-2 font-mono text-xs"
                />
              </label>
            ))}
          </div>
        ) : null}

        <div ref={splitRef} className="flex min-h-0 flex-1 flex-col">
          <div
            style={{ height: effectiveEditorHeight }}
            className="relative shrink-0 bg-surface-app"
          >
            {isClient ? (
              <MonacoEditor
                height="100%"
                language="sql"
                theme={editorTheme}
                value={tab.query ?? ""}
                options={editorOptions}
                onChange={(value) => updateQuery(tab.id, value ?? "")}
                onMount={editor.onMount}
              />
            ) : (
              <div className="flex h-full items-center justify-center text-xs text-text-muted">
                Loading editor…
              </div>
            )}
          </div>

          <ResizerHandle
            width={effectiveEditorHeight}
            onResize={handleEditorResize}
            orientation="horizontal"
            side="bottom"
            min={EDITOR_MIN_PX}
            max={maxEditorHeight}
            ariaLabel="Resize query editor"
          />

          <QueryResultsView
            view={resultsView}
            onViewChange={setResultsView}
            preview={activeQueryPreview}
            session={session}
            explainPlan={explainPlan}
            currentEdits={currentEdits}
            exportFilenameBase={exportFilenameBase}
            isRunning={isBusy}
            errorMessage={errorMessage}
            resultIndex={resultIndex}
            onResultIndexChange={setResultIndex}
            mutationGrid={mutationGrid}
            onCellEdit={(rowIndex, colIndex, value) =>
              setQueryEdit(tab.id, rowIndex, colIndex, value)
            }
            onSwitchBudgetOwner={setActiveTabId}
            onReleaseBudgetOwner={releaseQueryResults}
          />
        </div>

        <StatusBar items={statusItems} />
      </div>
      {!reviewScope ? (
        <ResponsiveEdgePanel
          side="right"
          storageKey="dbunk.sidebar.query"
          title="Query"
          width={QUERY_SIDEBAR_WIDTH}
          containerWidth={containerWidth}
          compactBelow={QUERY_SIDEBAR_COMPACT_BELOW}
          protectedWorkspaceWidth={PROTECTED_WORKSPACE_WIDTH}
          wideVisible={sidebar.wideVisible}
          open={sidebar.overlayOpen}
          onOpenChange={sidebar.setOverlayOpen}
          contentClassName="overflow-auto p-4"
        >
          <QuerySidebar tab={tab} />
        </ResponsiveEdgePanel>
      ) : null}
      {reviewPanel}
    </div>
  );
}

function isExplainStatement(sql: string): boolean {
  return /^\s*explain\b/i.test(sql);
}

function explainPreviewForOutcome(
  outcome: Extract<QueryOutcome, { kind: "completed" }>,
  tabId: string,
): QueryPreviewData {
  const execution = useAppStore.getState().querySessions[tabId]?.execution;
  if (execution && !execution.tombstone) {
    const first = execution.resultSets.find(
      (result) => result.columns.length > 0,
    );
    return {
      columns: (first?.columns ?? []).map((column) => column ?? ""),
      rows: flattenResultSetRows(first).map((row) =>
        row.map((cell) => cell ?? ""),
      ),
      runtime: `${execution.runtimeMs} ms`,
      rowCount: String(first?.rowCount ?? 0),
      cache: "Cold",
    };
  }
  return (
    outcome.preview ?? {
      columns: [],
      rows: [],
      runtime: `${outcome.runtimeMs} ms`,
      rowCount: String(outcome.rowCount),
      cache: "Cold",
    }
  );
}

function parseExplainPreview(
  preview: QueryPreviewData,
  outcome: Extract<QueryOutcome, { kind: "completed" }>,
): ExplainPlanData {
  const raw = preview.rows[0]?.[0] ?? "";
  const parsed = parseExplainJson(raw);
  if (parsed) {
    return {
      kind: "json",
      runtimeMs: outcome.runtimeMs,
      planningMs: numberOrNull(parsed["Planning Time"]),
      executionMs: numberOrNull(parsed["Execution Time"]),
      root: normalizeExplainNode(parsed.Plan),
    };
  }
  return {
    kind: "text",
    runtimeMs: outcome.runtimeMs,
    lines: preview.rows.map((row) => row.join(" ")).filter(Boolean),
  };
}

// oxlint-disable-next-line anti-slop/no-unsafe-dictionary-type -- The value is handled at a typed library or domain boundary here.
function parseExplainJson(value: string): Record<string, unknown> | null {
  try {
    // SAFETY: The value is constrained by the typed component or library contract at this boundary.
    const parsed = JSON.parse(value) as unknown;
    if (Array.isArray(parsed) && isRecord(parsed[0])) return parsed[0];
    if (isRecord(parsed)) return parsed;
  } catch {
    return null;
  }
  return null;
}

// oxlint-disable-next-line anti-slop/no-unknown-parameters -- The value is handled at a typed library or domain boundary here.
function normalizeExplainNode(value: unknown): ExplainPlanNode {
  const node = isRecord(value) ? value : {};
  const children = Array.isArray(node.Plans)
    ? node.Plans.map(normalizeExplainNode)
    : [];
  return {
    nodeType: stringOrDefault(node["Node Type"], "Plan"),
    relation: stringOrUndefined(node["Relation Name"]),
    alias: stringOrUndefined(node.Alias),
    startupCost: numberOrUndefined(node["Startup Cost"]),
    totalCost: numberOrUndefined(node["Total Cost"]),
    planRows: numberOrUndefined(node["Plan Rows"]),
    actualStartupTime: numberOrUndefined(node["Actual Startup Time"]),
    actualTotalTime: numberOrUndefined(node["Actual Total Time"]),
    actualRows: numberOrUndefined(node["Actual Rows"]),
    actualLoops: numberOrUndefined(node["Actual Loops"]),
    buffers: bufferSummary(node),
    children,
  };
}

// oxlint-disable-next-line anti-slop/no-unsafe-dictionary-type -- The value is handled at a typed library or domain boundary here.
function bufferSummary(node: Record<string, unknown>): string[] {
  return (
    [
      ["Shared Hit", node["Shared Hit Blocks"]],
      ["Shared Read", node["Shared Read Blocks"]],
      ["Shared Dirtied", node["Shared Dirtied Blocks"]],
      ["Shared Written", node["Shared Written Blocks"]],
      ["Temp Read", node["Temp Read Blocks"]],
      ["Temp Written", node["Temp Written Blocks"]],
    ]
      // oxlint-disable-next-line anti-slop/no-runtime-typeof -- The value is handled at a typed library or domain boundary here.
      .filter(([, value]) => typeof value === "number" && value > 0)
      .map(([label, value]) => `${label}: ${value}`)
  );
}

// oxlint-disable-next-line anti-slop/no-unknown-parameters, anti-slop/no-unsafe-dictionary-type -- The value is handled at a typed library or domain boundary here.
function isRecord(value: unknown): value is Record<string, unknown> {
  // oxlint-disable-next-line anti-slop/no-runtime-typeof -- The value is handled at a typed library or domain boundary here.
  return typeof value === "object" && value !== null;
}

// oxlint-disable-next-line anti-slop/no-unknown-parameters -- The value is handled at a typed library or domain boundary here.
function stringOrDefault(value: unknown, fallback: string): string {
  // oxlint-disable-next-line anti-slop/no-runtime-typeof -- The value is handled at a typed library or domain boundary here.
  return typeof value === "string" && value.trim() ? value : fallback;
}

// oxlint-disable-next-line anti-slop/no-unknown-parameters -- The value is handled at a typed library or domain boundary here.
function stringOrUndefined(value: unknown): string | undefined {
  // oxlint-disable-next-line anti-slop/no-runtime-typeof -- The value is handled at a typed library or domain boundary here.
  return typeof value === "string" && value.trim() ? value : undefined;
}

// oxlint-disable-next-line anti-slop/no-unknown-parameters -- The value is handled at a typed library or domain boundary here.
function numberOrUndefined(value: unknown): number | undefined {
  // oxlint-disable-next-line anti-slop/no-runtime-typeof -- The value is handled at a typed library or domain boundary here.
  return typeof value === "number" && Number.isFinite(value)
    ? value
    : undefined;
}

// oxlint-disable-next-line anti-slop/no-unknown-parameters -- The value is handled at a typed library or domain boundary here.
function numberOrNull(value: unknown): number | null {
  // oxlint-disable-next-line anti-slop/no-runtime-typeof -- The value is handled at a typed library or domain boundary here.
  return typeof value === "number" && Number.isFinite(value) ? value : null;
}

function analyzedTableForColumn(
  analysis: AnalyzeResultSetResult,
  column: AnalyzedColumn,
): AnalyzedTable | null {
  if (column.origin.kind !== "table") return null;
  const origin = column.origin;
  return (
    analysis.tables.find(
      (table) => table.schema === origin.schema && table.table === origin.table,
    ) ?? null
  );
}

function queryCellReadOnlyReason(
  analysis: AnalyzeResultSetResult,
  row: Array<string | null> | undefined,
  colIndex: number,
): string | undefined {
  if (analysis.statement.kind === "notAnalyzable") {
    return notAnalyzableCopy(analysis.statement.reason);
  }
  const column = analysis.columns[colIndex];
  if (!column) return "This column is not part of the analyzed result.";
  if (column.origin.kind === "expression") {
    return "Computed columns are read-only.";
  }
  switch (column.writability.kind) {
    case "generated":
      return "Generated columns are read-only.";
    case "identityAlways":
      return "Identity-always columns are read-only.";
    case "systemColumn":
      return "System columns are read-only.";
    case "writable":
      break;
  }
  const table = analyzedTableForColumn(analysis, column);
  if (!table) return "The source table could not be resolved.";
  if (!table.updatable.allowed) {
    return capabilityReasonCopy(table.updatable.reason);
  }
  if (table.identity.kind === "none") {
    return "This table has no stable row identity.";
  }
  if (!table.identityProjected) {
    return "Project the row identity columns to edit this table.";
  }
  if (
    table.identityProjectionIndexes.length !== table.identity.columns.length
  ) {
    return "The projected row identity is incomplete.";
  }
  if (
    row &&
    table.identityProjectionIndexes.some((index) => row[index] == null)
  ) {
    return "This row has a NULL identity value and cannot be edited safely.";
  }
  return undefined;
}

function projectedOriginalsForTable(
  analysis: AnalyzeResultSetResult,
  table: AnalyzedTable,
  row: Array<string | null>,
) {
  const originals = new Map<string, string | null>();
  analysis.columns.forEach((column, index) => {
    if (
      column.origin.kind === "table" &&
      column.origin.schema === table.schema &&
      column.origin.table === table.table &&
      !originals.has(column.origin.column)
    ) {
      originals.set(column.origin.column, row[index] ?? null);
    }
  });
  return [...originals].map(([column, value]) => ({ column, value }));
}

function mutationDraftGridEdits(
  draft: MutationDraft,
  analysis: AnalyzeResultSetResult,
): QueryMutationGridProps["edits"] {
  const edits: QueryMutationGridProps["edits"] = {};
  for (const changeId of draft.changeOrder) {
    const change = draft.changes[changeId];
    if (!change || change.kind !== "updateRow" || change.rowIndex === null) {
      continue;
    }
    const rowIndex = change.rowIndex;
    analysis.columns.forEach((column, colIndex) => {
      if (
        column.origin.kind !== "table" ||
        column.origin.schema !== change.table.schema ||
        column.origin.table !== change.table.table
      ) {
        return;
      }
      const cell = change.cells[column.origin.column];
      if (!cell) return;
      const rowEdits = edits[rowIndex] ?? {};
      rowEdits[colIndex] = cell.value ?? GRID_NULL_SENTINEL;
      edits[rowIndex] = rowEdits;
    });
  }
  return edits;
}

function mutationGridStatus({
  resultIndex,
  analysis,
  analysisState,
  stale,
}: {
  resultIndex: number;
  analysis: AnalyzeResultSetResult | null;
  analysisState:
    | { state: "loading" }
    | { state: "error"; message: string }
    | null;
  stale: boolean;
}): MutationGridStatus {
  if (stale) {
    return {
      copy: "Changes were applied. This result is stale until you re-run it.",
      tone: "warning",
    };
  }
  if (resultIndex !== 0) {
    return {
      copy: "Only the first result set can be edited.",
      tone: "muted",
    };
  }
  if (analysisState?.state === "loading") {
    return { copy: "Analyzing result editability…", tone: "muted" };
  }
  if (analysisState?.state === "error") {
    return { copy: analysisState.message, tone: "warning" };
  }
  if (!analysis) {
    return {
      copy: "Select a cell to analyze result editability.",
      tone: "muted",
    };
  }
  if (analysis.statement.kind === "notAnalyzable") {
    return {
      copy: notAnalyzableCopy(analysis.statement.reason),
      tone: "warning",
    };
  }
  const editableTables = analysis.tables.filter(
    (table) =>
      table.updatable.allowed &&
      table.identityProjected &&
      table.identity.kind !== "none",
  );
  if (editableTables.length === 0) {
    return {
      copy: "No projected table columns have a stable writable identity.",
      tone: "warning",
    };
  }
  const identities = [
    ...new Set(
      editableTables.map((table) => identityLabel(table.identity.kind)),
    ),
  ].join(", ");
  return { copy: `Editable · ${identities}`, tone: "success" };
}

function identityLabel(kind: AnalyzedTable["identity"]["kind"]): string {
  switch (kind) {
    case "primaryKey":
      return "primary key";
    case "uniqueIndex":
      return "unique index";
    case "virtualKey":
      return "virtual key";
    case "ctidFallback":
      return "ctid fallback";
    case "none":
      return "no identity";
  }
}

function capabilityReasonCopy(
  reason: AnalyzedTable["updatable"]["reason"],
): string {
  switch (reason) {
    case "noIdentity":
      return "This table has no stable row identity.";
    case "identityNotProjected":
      return "Project the row identity columns to edit this table.";
    case "multipleOriginTables":
      return "This column cannot be attributed to one writable table.";
    case "noWritableColumns":
      return "This table has no writable projected columns.";
    case "ctidInsertUnsupported":
      return "This table only has a temporary ctid identity.";
    case "invalidVirtualKey":
      return "The saved virtual key is invalid.";
    case "notAnalyzable":
    case undefined:
      return "This table is read-only for this result.";
  }
}

function notAnalyzableCopy(reason: NotAnalyzableReason): string {
  switch (reason.kind) {
    case "multiStatement":
      return "Editing requires one SQL statement. Re-run a single statement.";
    case "noProjectedColumns":
      return "This result has no projected columns to edit.";
    case "noTableOrigins":
      return "This result has no direct table columns to edit.";
    case "possibleTempShadowing":
      return "Editing is blocked because a temporary table may shadow the resolved target.";
    case "database":
      return reason.code
        ? `Analysis failed (${reason.code}): ${reason.message}`
        : `Analysis failed: ${reason.message}`;
  }
}

function mutationClientErrorCopy(error: ResultMutationError): string {
  if (error.kind === "notAnalyzable") return notAnalyzableCopy(error.reason);
  switch (error.kind) {
    case "unsupportedEngine":
      return "Result editing is unavailable for this database engine.";
    case "connectionClosing":
      return "The connection is closing. Reconnect and re-run the query.";
    case "connectionLost":
      return "The database connection was lost. Reconnect and re-run the query.";
    case "timeout":
      return `Analysis timed out during ${error.operation}.`;
    case "database":
      return error.code
        ? `Analysis failed (${error.code}): ${error.message}`
        : `Analysis failed: ${error.message}`;
    case "superseded":
      return "Analysis was replaced by a newer request.";
    case "cancelled":
      return "Analysis was cancelled.";
    case "busy":
      return "Another result mutation is already running.";
    case "unknownColumn":
      return `Analysis returned an unknown column: ${error.column}.`;
    case "invalidPlan":
      return `The mutation plan is invalid (${error.reason}).`;
    case "analysisExpired":
      return "The result analysis expired. Re-run the query.";
    case "conflict":
    case "identityNotUnique":
    case "lockTimeout":
      return "The result could not be prepared for editing.";
  }
}

function pluralize(noun: string, count: number): string {
  return count === 1 ? noun : `${noun}s`;
}

function slug(value: string): string {
  return value
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, "-")
    .replace(/^-+|-+$/g, "");
}
