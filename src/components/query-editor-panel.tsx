import MonacoEditor from "@monaco-editor/react";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { toast } from "sonner";

import "@/lib/monaco-local";
import {
  type ExplainPlanData,
  type ExplainPlanNode,
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
import type { SqlCompletionContext } from "@/lib/sql-completions";
import { formatSql } from "@/lib/sql-format";
import {
  type QueryOutcome,
  type QueryPreviewData,
  useAppStore,
  type WorkspaceTab,
} from "@/lib/store";
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
    queryStatus,
    queryEdits,
    schemaExplorer,
    tableStructure,
    editorTheme,
    connections,
    activeConnectionId,
    updateQuery,
    runQuery,
    loadTableStructure,
    setQueryEdit,
    discardQueryEdits,
    retargetQueryTab,
  } = useAppStore();

  const status = queryStatus[tab.id];
  const isRunning = status?.state === "running";
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
          setExplainPlan(parseExplainPreview(outcome.preview, outcome));
        } else if (outcome.kind === "failed") {
          setExplainPlan(null);
        }
        return;
      }
      setResultsView("results");
    },
    [setOutcome, setResultsView],
  );

  const activeQueryPreview: QueryPreviewData | null = useMemo(() => {
    if (tab.kind !== "query") return null;
    return (
      queryPreviews[tab.label] ?? {
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

  const editor = useMonacoQueryEditor({
    tabId: tab.id,
    query: tab.query ?? "",
    connectionId: tab.connectionId,
    completionContext,
    isRunning,
    loadTableStructure,
    runQuery,
    onOutcome: handleQueryOutcome,
    onFormat: () => handleFormat(),
  });

  const hasEdits = Object.keys(currentEdits).length > 0;
  const bindNames = useMemo(
    () => extractBindVariables(tab.query ?? ""),
    [tab.query],
  );
  const runCurrentWithBinds = () => {
    editor.runSql(applyBindVariables(editor.currentStatement(), bindValues));
  };

  const handleRetargetConnection = (newConnectionId: string) => {
    if (newConnectionId === tab.connectionId) return;
    if (isRunning) return;
    if (hasEdits) {
      const ok = window.confirm(
        "Switching connections will discard pending edits in the results grid. Continue?",
      );
      if (!ok) return;
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
    if (isRunning) return;
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
    const outcome = await runQuery(requestedTabId, { overrideSql: sql });
    if (requestedTabId !== activeTabIdRef.current) return;
    if (outcome.kind !== "noop") setOutcome(outcome);
    if (outcome.kind === "completed") {
      setExplainPlan(parseExplainPreview(outcome.preview, outcome));
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
  });

  useEffect(() => {
    onStatusItemsChange?.(statusItems);
  }, [onStatusItemsChange, statusItems]);

  const isWorkbench = variant === "workbench";
  const runCurrentHandler =
    bindNames.length > 0 ? runCurrentWithBinds : editor.handleRunCurrent;

  const resultsPane = (
    <QueryResultsView
      view={resultsView}
      onViewChange={setResultsView}
      preview={activeQueryPreview}
      explainPlan={explainPlan}
      currentEdits={currentEdits}
      exportFilenameBase={exportFilenameBase}
      isRunning={isRunning}
      errorMessage={errorMessage}
      hideTabs={isWorkbench}
      onCellEdit={(rowIndex, colIndex, value) =>
        setQueryEdit(tab.id, rowIndex, colIndex, value)
      }
    />
  );

  if (isWorkbench) {
    return (
      <div
        ref={containerRef}
        data-workspace-density={workspaceDensity}
        className="relative flex h-full min-h-0 flex-col bg-surface-app"
      >
        <QueryEditorToolbar
          dbSelectorLabel={dbSelectorLabel}
          connections={connections}
          currentConnectionId={tab.connectionId}
          onRetargetConnection={handleRetargetConnection}
          hasEdits={hasEdits}
          onDiscardEdits={() => discardQueryEdits(tab.id)}
          isRunning={isRunning}
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
          runDisabled={isRunning}
          consoleContent={
            <pre className="px-3 py-2 font-mono text-[12px] leading-relaxed text-foreground">
              {tab.query ?? ""}
            </pre>
          }
          outputContent={resultsPane}
        />
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
      </div>
    );
  }

  return (
    <div
      ref={containerRef}
      data-workspace-density={workspaceDensity}
      className="relative flex h-full min-h-0 bg-surface-app"
    >
      <div className="flex min-h-0 min-w-0 flex-1 flex-col">
        <QueryEditorToolbar
          dbSelectorLabel={dbSelectorLabel}
          connections={connections}
          currentConnectionId={tab.connectionId}
          onRetargetConnection={handleRetargetConnection}
          hasEdits={hasEdits}
          onDiscardEdits={() => discardQueryEdits(tab.id)}
          isRunning={isRunning}
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
            explainPlan={explainPlan}
            currentEdits={currentEdits}
            exportFilenameBase={exportFilenameBase}
            isRunning={isRunning}
            errorMessage={errorMessage}
            onCellEdit={(rowIndex, colIndex, value) =>
              setQueryEdit(tab.id, rowIndex, colIndex, value)
            }
          />
        </div>

        <StatusBar items={statusItems} />
      </div>
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
    </div>
  );
}

function isExplainStatement(sql: string): boolean {
  return /^\s*explain\b/i.test(sql);
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

function parseExplainJson(value: string): Record<string, unknown> | null {
  try {
    const parsed = JSON.parse(value) as unknown;
    if (Array.isArray(parsed) && isRecord(parsed[0])) return parsed[0];
    if (isRecord(parsed)) return parsed;
  } catch {
    return null;
  }
  return null;
}

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

function bufferSummary(node: Record<string, unknown>): string[] {
  return [
    ["Shared Hit", node["Shared Hit Blocks"]],
    ["Shared Read", node["Shared Read Blocks"]],
    ["Shared Dirtied", node["Shared Dirtied Blocks"]],
    ["Shared Written", node["Shared Written Blocks"]],
    ["Temp Read", node["Temp Read Blocks"]],
    ["Temp Written", node["Temp Written Blocks"]],
  ]
    .filter(([, value]) => typeof value === "number" && value > 0)
    .map(([label, value]) => `${label}: ${value}`);
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null;
}

function stringOrDefault(value: unknown, fallback: string): string {
  return typeof value === "string" && value.trim() ? value : fallback;
}

function stringOrUndefined(value: unknown): string | undefined {
  return typeof value === "string" && value.trim() ? value : undefined;
}

function numberOrUndefined(value: unknown): number | undefined {
  return typeof value === "number" && Number.isFinite(value)
    ? value
    : undefined;
}

function numberOrNull(value: unknown): number | null {
  return typeof value === "number" && Number.isFinite(value) ? value : null;
}

function slug(value: string): string {
  return value
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, "-")
    .replace(/^-+|-+$/g, "");
}
