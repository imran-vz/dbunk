import MonacoEditor, { type OnMount } from "@monaco-editor/react";
import {
  IconAlertCircle,
  IconDatabase,
  IconDeviceFloppy,
  IconLoader2,
  IconPlayerPlay,
  IconSearch,
  IconTerminal2,
  IconX,
} from "@tabler/icons-react";
import { useCallback, useEffect, useMemo, useRef } from "react";
import { DataGrid } from "@/components/data-grid";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { getSqlStatementAtPosition, getSqlStatements } from "@/lib/sql";
import {
  getSqlCompletions,
  getSqlPredicateTableReference,
  type SqlCompletionContext,
} from "@/lib/sql-completions";
import {
  type QueryPreviewData,
  tableStructureKey,
  useAppStore,
  type WorkspaceTab,
} from "@/lib/store";

// Minimal shape we need from the Monaco editor instance. Avoids pulling the
// full monaco-editor types in (the package isn't installed for runtime use).
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

export function QueryEditorPanel({ tab, isClient }: QueryEditorPanelProps) {
  const {
    queryPreviews,
    queryStatus,
    queryEdits,
    schemaExplorer,
    tableStructure,
    editorTheme,
    updateQuery,
    runQuery,
    loadTableStructure,
    setQueryEdit,
    discardQueryEdits,
  } = useAppStore();

  const status = queryStatus[tab.id] ?? { state: "idle" as const };
  const isRunning = status.state === "running";
  const errorMessage = status.state === "error" ? status.error : null;

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
    // Drop any extension on the label (e.g. "query_1.sql" -> "query-1") so
    // the grid can append its own.
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
    (sql: string) => {
      if (!sql.trim() || isRunning) {
        return;
      }
      void runQuery(tab.id, { overrideSql: sql });
    },
    [isRunning, runQuery, tab.id],
  );

  const handleRunCurrent = useCallback(() => {
    runSql(getCurrentStatementText());
  }, [getCurrentStatementText, runSql]);

  const handleRunSelection = useCallback(() => {
    runSql(getEditorSelectionText());
  }, [getEditorSelectionText, runSql]);

  const handleRunAll = useCallback(() => {
    runSql(tab.query ?? "");
  }, [runSql, tab.query]);

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

      const runCurrent = () => {
        const latestEditor = editorRef.current;
        const latestModel = latestEditor?.getModel();
        const latestPosition = latestEditor?.getPosition();
        if (!latestModel || !latestPosition) {
          runSql(tab.query ?? "");
          return;
        }
        runSql(
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
        run: () => runSql(getEditorSelectionText()),
      });
      const allAction = editor.addAction?.({
        id: "dbunk.executeAll",
        label: "Execute all",
        contextMenuGroupId: "navigation",
        contextMenuOrder: 3,
        run: () => runSql(editor.getModel()?.getValue() ?? ""),
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
        runSql(statement.sql);
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
        fontFamily: "JetBrains Mono Variable, monospace",
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

  return (
    <div className="flex h-full flex-col bg-background">
      {/* Top Toolbar */}
      <div className="flex h-10 shrink-0 items-center justify-between border-b px-3">
        <div className="flex items-center gap-3">
          <div className="flex items-center gap-2 text-sm text-muted-foreground">
            <IconTerminal2 className="size-4" />
            <span className="font-medium text-foreground">{tab.label}</span>
          </div>
          <div className="h-4 w-px bg-border" />
          <div className="flex items-center gap-1.5 text-xs text-muted-foreground">
            <IconDatabase className="size-3" />
            <span>{tab.schema}</span>
          </div>
        </div>
        <div className="flex items-center gap-2">
          {hasEdits && (
            <>
              <Button
                size="sm"
                variant="ghost"
                className="h-7 px-2 text-xs"
                onClick={() => discardQueryEdits(tab.id)}
              >
                <IconX className="mr-1 size-3.5" /> Discard
              </Button>
              <Button size="sm" className="h-7 px-2 text-xs">
                <IconDeviceFloppy className="mr-1 size-3.5" /> Save changes
              </Button>
              <div className="h-4 w-px bg-border" />
            </>
          )}

          <Button
            size="sm"
            variant="secondary"
            className="h-7 px-3 text-xs shadow-none"
            onClick={handleRunCurrent}
            disabled={isRunning}
            aria-busy={isRunning}
          >
            {isRunning ? (
              <>
                <IconLoader2 className="mr-1.5 size-3.5 animate-spin" />
                Running...
              </>
            ) : (
              <>
                <IconPlayerPlay className="mr-1.5 size-3.5" />
                Run current
              </>
            )}
          </Button>
          <Button
            size="sm"
            variant="outline"
            className="h-7 px-3 text-xs shadow-none"
            onClick={handleRunSelection}
            disabled={isRunning}
          >
            <IconPlayerPlay className="mr-1.5 size-3.5" />
            Selection
          </Button>
          <Button
            size="sm"
            variant="outline"
            className="h-7 px-3 text-xs shadow-none"
            onClick={handleRunAll}
            disabled={isRunning}
          >
            <IconPlayerPlay className="mr-1.5 size-3.5" />
            All
          </Button>
          <Button
            size="sm"
            variant="outline"
            className="h-7 px-3 text-xs shadow-none"
          >
            <IconSearch className="mr-1.5 size-3.5" />
            Explain
          </Button>
        </div>
      </div>

      {/* Split Pane Container */}
      <div className="flex min-h-0 flex-1 flex-col">
        {/* Editor Section - Fixed Height for now */}
        <div className="relative h-60 shrink-0 border-b">
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
            <div className="flex h-full items-center justify-center text-xs text-muted-foreground">
              Loading editor...
            </div>
          )}
        </div>

        {/* Results Section */}
        <div className="flex min-h-0 flex-1 flex-col bg-background max-w-[calc(100vw-16rem)]">
          {/* Results Info Bar */}
          <div className="flex h-8 shrink-0 items-center gap-2 border-b bg-muted/20 px-2">
            <Badge
              variant="outline"
              className="h-5 rounded-sm border-transparent bg-transparent px-1 text-[0.65rem] font-normal text-muted-foreground hover:bg-muted"
            >
              {activeQueryPreview?.rowCount ?? 0} rows
            </Badge>
            <div className="h-3 w-px bg-border" />
            <Badge
              variant="outline"
              className="h-5 rounded-sm border-transparent bg-transparent px-1 text-[0.65rem] font-normal text-muted-foreground hover:bg-muted"
            >
              {activeQueryPreview?.runtime ?? "--"}
            </Badge>
            <div className="h-3 w-px bg-border" />
            <Badge
              variant="outline"
              className="h-5 rounded-sm border-transparent bg-transparent px-1 text-[0.65rem] font-normal text-muted-foreground hover:bg-muted"
            >
              ReadOnly
            </Badge>
          </div>

          {errorMessage && (
            <div
              role="alert"
              className="flex shrink-0 items-start gap-2 border-b border-destructive/30 bg-destructive/10 px-3 py-2 text-xs text-destructive"
            >
              <IconAlertCircle className="mt-0.5 size-3.5 shrink-0" />
              <div className="flex-1 whitespace-pre-wrap wrap-break-word font-mono">
                {errorMessage}
              </div>
            </div>
          )}

          {/* Data Grid */}
          <div className="flex-1 overflow-hidden max-w-[calc(100vw-16rem)]">
            {isRunning ? (
              <div className="flex h-full flex-col items-center justify-center gap-2 text-muted-foreground">
                <IconLoader2 className="size-6 animate-spin opacity-70" />
                <div className="text-xs">Running query...</div>
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
              <div className="flex h-full flex-col items-center justify-center gap-2 text-muted-foreground">
                <div className="rounded-full bg-destructive/10 p-3">
                  <IconAlertCircle className="size-6 text-destructive opacity-70" />
                </div>
                <div className="text-xs">Query did not return results</div>
              </div>
            ) : (
              <div className="flex h-full flex-col items-center justify-center gap-2 text-muted-foreground">
                <div className="rounded-full bg-muted p-3">
                  <IconTerminal2 className="size-6 opacity-50" />
                </div>
                <div className="text-xs">Run the query to see results</div>
              </div>
            )}
          </div>
        </div>
      </div>
    </div>
  );
}
