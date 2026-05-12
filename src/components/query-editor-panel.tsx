import MonacoEditor, { type OnMount } from "@monaco-editor/react";
import {
  IconAlertCircle,
  IconChevronDown,
  IconCopy,
  IconDeviceFloppy,
  IconDownload,
  IconLoader2,
  IconPlayerPlay,
  IconSparkles,
  IconTerminal2,
  IconX,
} from "@tabler/icons-react";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { DataGrid } from "@/components/data-grid";
import { QuerySidebar } from "@/components/query-sidebar";
import { StatusBar, type StatusBarItem } from "@/components/status-bar";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { getSqlStatementAtPosition, getSqlStatements } from "@/lib/sql";
import {
  getSqlCompletions,
  getSqlPredicateTableReference,
  type SqlCompletionContext,
} from "@/lib/sql-completions";
import {
  type QueryOutcome,
  type QueryPreviewData,
  tableStructureKey,
  useAppStore,
  type WorkspaceTab,
} from "@/lib/store";
import { cn } from "@/lib/utils";

type MonacoEditorInstance = {
  getPosition: () => MonacoPosition | null;
  getSelection: () => unknown;
  getModel: () => MonacoTextModel | null;
  addAction?: (descriptor: {
    id: string;
    label: string;
    keybindings?: number[];
    contextMenuGroupId?: string;
    contextMenuOrder?: number;
    run: () => void;
  }) => MonacoCompletionDisposable;
  createDecorationsCollection?: (decorations?: unknown[]) => {
    set: (decorations: unknown[]) => void;
    clear: () => void;
  };
  onMouseDown?: (
    listener: (event: MonacoMouseEvent) => void,
  ) => MonacoCompletionDisposable;
  onDidChangeModelContent?: (
    listener: () => void,
  ) => MonacoCompletionDisposable;
  onDidChangeCursorPosition?: (
    listener: (event: { position: MonacoPosition }) => void,
  ) => MonacoCompletionDisposable;
};

type MonacoCompletionDisposable = {
  dispose: () => void;
};

type MonacoPosition = {
  lineNumber: number;
  column: number;
};

type MonacoTextModel = {
  getValue: () => string;
  getWordUntilPosition: (position: MonacoPosition) => {
    startColumn: number;
    endColumn: number;
  };
  getValueInRange: (range: unknown) => string;
};

type MonacoMouseEvent = {
  target: {
    type: number;
    position?: MonacoPosition | null;
  };
  event?: {
    preventDefault?: () => void;
  };
};

interface QueryEditorPanelProps {
  tab: WorkspaceTab;
  isClient: boolean;
}

type ResultsView = "results" | "explain";

export function QueryEditorPanel({ tab, isClient }: QueryEditorPanelProps) {
  const [resultsView, setResultsView] = useState<ResultsView>("results");
  const [cursor, setCursor] = useState<MonacoPosition>({
    lineNumber: 1,
    column: 1,
  });

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
  // with the sibling table-structure-view, and to leave room for a
  // future "Completed in Xms" toast. Today only `failed` is rendered;
  // the success-path setLastOutcome triggers one no-op re-render that
  // could be optimised by narrowing the state if it shows up in
  // profiling. See CONTEXT.md — Query Outcome.
  const [lastOutcome, setLastOutcome] = useState<QueryOutcome | null>(null);
  // Render-phase reset on tab switch — the panel instance is reused
  // across tabs (no React key), so without this an error banner from
  // tab A could leak into tab B's view. Using the tracked-prev-prop
  // pattern (React docs: "Resetting all state when a prop changes")
  // rather than a useEffect so biome's exhaustive-deps rule stays
  // happy with the trigger-only dependency.
  const [trackedTabId, setTrackedTabId] = useState(tab.id);
  if (trackedTabId !== tab.id) {
    setTrackedTabId(tab.id);
    setLastOutcome(null);
  }
  const errorMessage =
    lastOutcome?.kind === "failed" ? lastOutcome.reason : null;

  const activeQueryPreview: QueryPreviewData | null = useMemo(() => {
    if (tab.kind !== "query") {
      return null;
    }
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
    const slug = (value: string) =>
      value
        .toLowerCase()
        .replace(/[^a-z0-9]+/g, "-")
        .replace(/^-+|-+$/g, "");
    const labelStem = tab.label.replace(/\.[^.]+$/, "");
    return [slug(labelStem), today].filter(Boolean).join("-");
  }, [tab.label]);

  const editorRef = useRef<MonacoEditorInstance | null>(null);
  const completionDisposableRef = useRef<MonacoCompletionDisposable | null>(
    null,
  );
  const editorDisposablesRef = useRef<MonacoCompletionDisposable[]>([]);
  const decorationsCollectionRef = useRef<{
    set: (decorations: unknown[]) => void;
    clear: () => void;
  } | null>(null);
  const completionContextRef = useRef<SqlCompletionContext>({
    schemas: [],
    currentSchema: tab.schema,
  });

  const completionContext = useMemo<SqlCompletionContext>(
    () => ({
      connectionId: tab.connectionId,
      schemas: schemaExplorer[tab.connectionId] ?? [],
      currentSchema: tab.schema,
      tableStructure,
    }),
    [schemaExplorer, tab.connectionId, tab.schema, tableStructure],
  );

  completionContextRef.current = completionContext;

  const activeConnection = useMemo(
    () =>
      connections.find((c) => c.id === tab.connectionId) ??
      connections.find((c) => c.id === activeConnectionId),
    [activeConnectionId, connections, tab.connectionId],
  );

  const getEditorSelectionText = useCallback((): string => {
    const editor = editorRef.current;
    if (!editor) {
      return "";
    }
    const selection = editor.getSelection();
    const model = editor.getModel();
    if (!selection || !model) {
      return "";
    }
    return model.getValueInRange(selection) ?? "";
  }, []);

  const getCurrentStatementText = useCallback((): string => {
    const editor = editorRef.current;
    const model = editor?.getModel();
    const position = editor?.getPosition();
    if (!model || !position) {
      return tab.query ?? "";
    }
    return (
      getSqlStatementAtPosition(
        model.getValue(),
        position.lineNumber,
        position.column,
      )?.sql ?? ""
    );
  }, [tab.query]);

  const runSql = useCallback(
    async (sql: string) => {
      if (!sql.trim() || isRunning) {
        return;
      }
      const requestedTabId = tab.id;
      const outcome = await runQuery(requestedTabId, { overrideSql: sql });
      // Drop the outcome if the user switched tabs while the query
      // was in flight — the panel instance is reused across query
      // tabs (workspace-view.tsx renders no React `key`), so without
      // this guard the closure's `setLastOutcome` would write tab A's
      // result into tab B's banner. The setter is stable so it always
      // targets the *current* tab's component state.
      if (requestedTabId !== tab.id) {
        return;
      }
      // Skip `noop` so a rapid double-click (the second call short-
      // circuits while the first is still running) can't wipe the
      // completed banner the first call just produced.
      if (outcome.kind !== "noop") {
        setLastOutcome(outcome);
      }
    },
    [isRunning, runQuery, tab.id],
  );

  const handleRunCurrent = useCallback(() => {
    void runSql(getCurrentStatementText());
  }, [getCurrentStatementText, runSql]);

  const handleRunSelection = useCallback(() => {
    void runSql(getEditorSelectionText());
  }, [getEditorSelectionText, runSql]);

  const handleRunAll = useCallback(() => {
    void runSql(tab.query ?? "");
  }, [runSql, tab.query]);

  const handleFormat = useCallback(() => {
    // TODO(designs/FOLLOWUPS.md Phase 6): wire actual SQL formatter
  }, []);

  const updateQueryRunDecorations = useCallback(
    (monaco: Parameters<OnMount>[1], model: MonacoTextModel) => {
      decorationsCollectionRef.current?.set(
        getSqlStatements(model.getValue()).map((statement) => ({
          range: new monaco.Range(
            statement.startLine,
            1,
            statement.startLine,
            1,
          ),
          options: {
            glyphMarginClassName: "dbunk-query-run-glyph",
            glyphMarginHoverMessage: {
              value: "Execute this query",
            },
          },
        })),
      );
    },
    [],
  );

  const handleEditorMount = useCallback<OnMount>(
    (editor, monaco) => {
      editorRef.current = editor as MonacoEditorInstance;
      completionDisposableRef.current?.dispose();
      editorDisposablesRef.current.forEach((disposable) => {
        disposable.dispose();
      });
      editorDisposablesRef.current = [];

      const model = editor.getModel() as MonacoTextModel | null;
      if (model) {
        const collection = editor.createDecorationsCollection?.();
        decorationsCollectionRef.current = collection
          ? {
              set: (decorations) => {
                collection.set(decorations as never[]);
              },
              clear: () => collection.clear(),
            }
          : null;
        updateQueryRunDecorations(monaco, model);
        const contentDisposable = editor.onDidChangeModelContent?.(() => {
          const latestModel = editor.getModel() as MonacoTextModel | null;
          if (latestModel) {
            updateQueryRunDecorations(monaco, latestModel);
          }
        });
        if (contentDisposable) {
          editorDisposablesRef.current.push(contentDisposable);
        }
      }

      const cursorDisposable = (
        editor as MonacoEditorInstance
      ).onDidChangeCursorPosition?.(({ position }) => {
        setCursor(position);
      });
      if (cursorDisposable) {
        editorDisposablesRef.current.push(cursorDisposable);
      }

      const runCurrent = () => {
        const latestEditor = editorRef.current;
        const latestModel = latestEditor?.getModel();
        const latestPosition = latestEditor?.getPosition();
        if (!latestModel || !latestPosition) {
          void runSql(tab.query ?? "");
          return;
        }
        void runSql(
          getSqlStatementAtPosition(
            latestModel.getValue(),
            latestPosition.lineNumber,
            latestPosition.column,
          )?.sql ?? "",
        );
      };

      const currentAction = editor.addAction?.({
        id: "dbunk.executeCurrentQuery",
        label: "Execute current query",
        keybindings: [monaco.KeyMod.CtrlCmd | monaco.KeyCode.Enter],
        contextMenuGroupId: "navigation",
        contextMenuOrder: 1,
        run: runCurrent,
      });
      const selectionAction = editor.addAction?.({
        id: "dbunk.executeSelection",
        label: "Execute selection",
        contextMenuGroupId: "navigation",
        contextMenuOrder: 2,
        run: () => {
          void runSql(getEditorSelectionText());
        },
      });
      const allAction = editor.addAction?.({
        id: "dbunk.executeAll",
        label: "Execute all",
        contextMenuGroupId: "navigation",
        contextMenuOrder: 3,
        run: () => {
          void runSql(editor.getModel()?.getValue() ?? "");
        },
      });
      const mouseDisposable = editor.onMouseDown?.((event) => {
        if (event.target.type !== 2 || !event.target.position) {
          return;
        }
        const latestModel = editor.getModel() as MonacoTextModel | null;
        if (!latestModel) {
          return;
        }
        const statement = getSqlStatementAtPosition(
          latestModel.getValue(),
          event.target.position.lineNumber,
          1,
        );
        if (
          !statement ||
          statement.startLine !== event.target.position.lineNumber
        ) {
          return;
        }
        event.event?.preventDefault?.();
        void runSql(statement.sql);
      });
      editorDisposablesRef.current.push(
        ...[currentAction, selectionAction, allAction, mouseDisposable].filter(
          (item): item is MonacoCompletionDisposable => Boolean(item),
        ),
      );

      completionDisposableRef.current =
        monaco.languages.registerCompletionItemProvider("sql", {
          triggerCharacters: [" ", ".", "\n"],
          provideCompletionItems: async (
            model: MonacoTextModel,
            position: MonacoPosition,
          ) => {
            const word = model.getWordUntilPosition(position);
            const range = new monaco.Range(
              position.lineNumber,
              word.startColumn,
              position.lineNumber,
              word.endColumn,
            );
            const textBeforeCursor = model.getValueInRange(
              new monaco.Range(1, 1, position.lineNumber, position.column),
            );
            const predicateTable = getSqlPredicateTableReference(
              textBeforeCursor,
              completionContextRef.current,
            );

            if (predicateTable) {
              const key = tableStructureKey(
                tab.connectionId,
                predicateTable.schema,
                predicateTable.table,
              );
              const latestState = useAppStore.getState();
              const status = latestState.tableStructureStatus[key]?.state;
              if (!latestState.tableStructure[key] && status !== "loading") {
                await loadTableStructure(
                  tab.connectionId,
                  predicateTable.schema,
                  predicateTable.table,
                );
                completionContextRef.current = {
                  ...completionContextRef.current,
                  tableStructure: useAppStore.getState().tableStructure,
                };
              }
            }

            const kindByType = {
              column: monaco.languages.CompletionItemKind.Field,
              keyword: monaco.languages.CompletionItemKind.Keyword,
              schema: monaco.languages.CompletionItemKind.Module,
              table: monaco.languages.CompletionItemKind.Struct,
              view: monaco.languages.CompletionItemKind.Interface,
            };

            const suggestions = getSqlCompletions(
              textBeforeCursor,
              completionContextRef.current,
            ).map((item) => {
              return {
                label: item.label,
                insertText: item.insertText,
                kind: kindByType[item.kind],
                detail: item.detail,
                sortText: item.sortText,
                range,
              };
            });

            return { suggestions };
          },
        });
    },
    [
      getEditorSelectionText,
      loadTableStructure,
      runSql,
      tab.connectionId,
      tab.query,
      updateQueryRunDecorations,
    ],
  );

  useEffect(
    () => () => {
      completionDisposableRef.current?.dispose();
      decorationsCollectionRef.current?.clear();
      editorDisposablesRef.current.forEach((disposable) => {
        disposable.dispose();
      });
    },
    [],
  );

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
        padding: { top: 12, bottom: 12 },
        renderLineHighlight: "none",
        overviewRulerBorder: false,
        hideCursorInOverviewRuler: true,
        scrollbar: {
          vertical: "visible",
          horizontal: "visible",
          useShadows: false,
        },
      }) as const,
    [],
  );

  const dbSelectorLabel = activeConnection
    ? `${activeConnection.engine} (${activeConnection.host || activeConnection.database})`
    : "No connection";

  const statusItems: StatusBarItem[] = [
    {
      id: "tab",
      label: "Tab",
      value: tab.label,
    },
    {
      id: "cursor",
      label: "",
      value: `Ln ${cursor.lineNumber}, Col ${cursor.column}`,
    },
    {
      id: "diagnostics",
      tone: errorMessage ? "danger" : "healthy",
      value: errorMessage ? "Has errors" : "No errors",
    },
    {
      id: "tx",
      label: "Auto-commit",
      tone: "healthy",
      value: "ON",
      align: "right",
    },
  ];

  return (
    <div className="grid h-full min-h-0 grid-cols-[minmax(0,1fr)] bg-surface-app xl:grid-cols-[minmax(0,1fr)_19rem]">
      <div className="flex min-h-0 min-w-0 flex-col">
        {/* Editor toolbar */}
        <div className="flex h-14 shrink-0 items-center justify-between gap-3 border-b border-border-subtle bg-surface-window px-5">
          <div className="flex min-w-0 items-center gap-2.5">
            <span className="flex size-8 items-center justify-center rounded-md border border-accent-green/30 bg-accent-green/10 text-accent-green">
              <IconTerminal2 className="size-4" />
            </span>
            <h1 className="truncate text-sm font-semibold tracking-tight text-foreground">
              Query Editor
            </h1>
            <DropdownMenu>
              <DropdownMenuTrigger
                aria-label="Connection selector"
                className="ml-2 inline-flex h-8 items-center gap-2 rounded-md border border-border-subtle bg-surface-panel px-3 text-xs font-medium text-foreground transition-colors hover:bg-surface-panel-elevated"
              >
                <span className="size-1.5 rounded-full bg-accent-green" />
                <span className="truncate">{dbSelectorLabel}</span>
                <IconChevronDown className="size-3 text-text-muted" />
              </DropdownMenuTrigger>
              <DropdownMenuContent align="start">
                {connections.map((connection) => (
                  <DropdownMenuItem
                    key={connection.id}
                    // TODO(FOLLOWUPS): switch the editor's connection
                    onClick={() => {}}
                  >
                    {connection.name} · {connection.engine}
                  </DropdownMenuItem>
                ))}
              </DropdownMenuContent>
            </DropdownMenu>
          </div>
          <div className="flex items-center gap-2">
            {hasEdits ? (
              <>
                <Button
                  size="sm"
                  variant="ghost"
                  onClick={() => discardQueryEdits(tab.id)}
                >
                  <IconX className="size-3.5" /> Discard
                </Button>
                <Button size="sm">
                  <IconDeviceFloppy className="size-3.5" /> Save
                </Button>
                <div className="h-5 w-px bg-border-subtle" />
              </>
            ) : null}

            <Button size="sm" variant="outline" onClick={handleFormat}>
              <IconSparkles className="size-3.5" />
              Format
            </Button>

            <div className="flex items-center">
              <Button
                size="sm"
                onClick={handleRunCurrent}
                disabled={isRunning}
                aria-busy={isRunning}
                className="rounded-r-none"
              >
                {isRunning ? (
                  <>
                    <IconLoader2 className="size-3.5 animate-spin" />
                    Running…
                  </>
                ) : (
                  <>
                    <IconPlayerPlay className="size-3.5" />
                    Run
                  </>
                )}
              </Button>
              <DropdownMenu>
                <DropdownMenuTrigger
                  aria-label="Run options"
                  className="inline-flex h-8 items-center justify-center rounded-r-md border-l border-primary-foreground/20 bg-primary px-2 text-primary-foreground hover:bg-accent-green-hover disabled:opacity-50"
                  disabled={isRunning}
                >
                  <IconChevronDown className="size-3.5" />
                </DropdownMenuTrigger>
                <DropdownMenuContent align="end">
                  <DropdownMenuItem onClick={handleRunSelection}>
                    Run selection
                  </DropdownMenuItem>
                  <DropdownMenuItem onClick={handleRunCurrent}>
                    Run current statement
                  </DropdownMenuItem>
                  <DropdownMenuItem onClick={handleRunAll}>
                    Run all
                  </DropdownMenuItem>
                </DropdownMenuContent>
              </DropdownMenu>
            </div>
          </div>
        </div>

        {/* Editor + results */}
        <div className="flex min-h-0 flex-1 flex-col">
          <div className="relative h-[19rem] shrink-0 border-b border-border-subtle bg-surface-app">
            {isClient ? (
              <MonacoEditor
                height="100%"
                language="sql"
                theme={editorTheme}
                value={tab.query ?? ""}
                options={editorOptions}
                onChange={(value) => updateQuery(tab.id, value ?? "")}
                onMount={handleEditorMount}
              />
            ) : (
              <div className="flex h-full items-center justify-center text-xs text-text-muted">
                Loading editor…
              </div>
            )}
          </div>

          <div className="flex min-h-0 flex-1 flex-col bg-surface-app">
            {/* Results tabs + meta */}
            <div className="flex h-10 shrink-0 items-center gap-3 border-b border-border-subtle bg-surface-window px-5">
              <div className="flex items-end gap-1">
                {(
                  [
                    { id: "results", label: "Results" },
                    { id: "explain", label: "Explain" },
                  ] as const
                ).map(({ id, label }) => {
                  const isActive = resultsView === id;
                  return (
                    <button
                      key={id}
                      type="button"
                      onClick={() => setResultsView(id)}
                      aria-current={isActive ? "page" : undefined}
                      className={cn(
                        "relative h-9 px-2.5 text-xs font-medium transition-colors",
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
              <div className="flex items-center gap-2 text-[0.6875rem] text-text-muted">
                <span>
                  Returned {activeQueryPreview?.rowCount ?? 0} rows in{" "}
                  {activeQueryPreview?.runtime ?? "—"}
                </span>
              </div>
              <div className="ml-auto flex items-center gap-1">
                <Button
                  size="icon-sm"
                  variant="ghost"
                  aria-label="Download results"
                >
                  <IconDownload className="size-3.5" />
                </Button>
                <Button
                  size="icon-sm"
                  variant="ghost"
                  aria-label="Copy results"
                >
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
              {resultsView === "explain" ? (
                <div className="flex h-full items-center justify-center p-6">
                  <div className="max-w-md rounded-lg border border-dashed border-border-subtle bg-surface-panel/40 p-6 text-center">
                    <div className="text-sm font-semibold text-foreground">
                      Explain plan
                    </div>
                    <p className="mt-1 text-xs text-text-muted">
                      The execution plan for the active statement will appear
                      here.
                    </p>
                    <Badge variant="outline" className="mt-3">
                      Coming soon
                    </Badge>
                  </div>
                </div>
              ) : isRunning ? (
                <div className="flex h-full flex-col items-center justify-center gap-2 text-text-muted">
                  <IconLoader2 className="size-6 animate-spin opacity-70" />
                  <div className="text-xs">Running query…</div>
                </div>
              ) : activeQueryPreview?.rows.length ? (
                <DataGrid
                  data={activeQueryPreview?.rows ?? []}
                  columns={activeQueryPreview?.columns ?? []}
                  edits={currentEdits}
                  onEdit={(rowIndex, colIndex, value) =>
                    setQueryEdit(tab.id, rowIndex, colIndex, value)
                  }
                  exportFilenameBase={exportFilenameBase}
                />
              ) : errorMessage ? (
                <div className="flex h-full flex-col items-center justify-center gap-2 text-text-muted">
                  <div className="rounded-full bg-danger/10 p-3">
                    <IconAlertCircle className="size-6 text-danger opacity-70" />
                  </div>
                  <div className="text-xs">Query did not return results</div>
                </div>
              ) : (
                <div className="flex h-full flex-col items-center justify-center gap-2 text-text-muted">
                  <div className="rounded-full bg-surface-panel-elevated p-3">
                    <IconTerminal2 className="size-6 opacity-50" />
                  </div>
                  <div className="text-xs">Run the query to see results</div>
                </div>
              )}
            </div>
          </div>
        </div>

        <StatusBar items={statusItems} />
      </div>
      <aside className="hidden min-h-0 border-l border-border-subtle bg-surface-window p-4 xl:block">
        <QuerySidebar tab={tab} />
      </aside>
    </div>
  );
}
