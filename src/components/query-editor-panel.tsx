import MonacoEditor from "@monaco-editor/react";
import { useMemo, useState } from "react";
import {
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
import { StatusBar } from "@/components/status-bar";
import { ResponsiveEdgePanel } from "@/components/ui/responsive-edge-panel";
import type { SqlCompletionContext } from "@/lib/sql-completions";
import {
  type QueryPreviewData,
  useAppStore,
  type WorkspaceTab,
} from "@/lib/store";
import { useContainerWidth } from "@/lib/use-resizable-width";

interface QueryEditorPanelProps {
  tab: WorkspaceTab;
  isClient: boolean;
}

export function QueryEditorPanel({ tab, isClient }: QueryEditorPanelProps) {
  const [resultsView, setResultsView] = useState<ResultsView>("results");
  const [containerRef, containerWidth] = useContainerWidth<HTMLDivElement>();
  const sidebar = useQuerySidebarVisibility(containerWidth);

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
  } = useAppStore();

  const status = queryStatus[tab.id];
  const isRunning = status?.state === "running";
  // Terminal outcome lives component-local. We store the full
  // QueryOutcome (not just the failure message) for shape parity
  // with sibling panels. See CONTEXT.md — Query Outcome.
  const { errorMessage, setOutcome } = useQueryOutcome(tab.id);

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
    onOutcome: setOutcome,
  });

  const hasEdits = Object.keys(currentEdits).length > 0;

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
  });

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
          hasEdits={hasEdits}
          onDiscardEdits={() => discardQueryEdits(tab.id)}
          isRunning={isRunning}
          isSidebarOpen={sidebar.isOpen}
          onToggleSidebar={sidebar.onToggle}
          onRunCurrent={editor.handleRunCurrent}
          onRunSelection={editor.handleRunSelection}
          onRunAll={editor.handleRunAll}
        />

        <div className="flex min-h-0 flex-1 flex-col">
          <div className="relative h-[17.5rem] shrink-0 border-b border-border-subtle bg-surface-app">
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

          <QueryResultsView
            view={resultsView}
            onViewChange={setResultsView}
            preview={activeQueryPreview}
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

function slug(value: string): string {
  return value
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, "-")
    .replace(/^-+|-+$/g, "");
}
