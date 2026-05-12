import type { OnMount } from "@monaco-editor/react";
import { useCallback, useEffect, useRef, useState } from "react";

import { getSqlStatementAtPosition, getSqlStatements } from "@/lib/sql";
import {
  getSqlCompletions,
  getSqlPredicateTableReference,
  type SqlCompletionContext,
} from "@/lib/sql-completions";
import { type QueryOutcome, tableStructureKey, useAppStore } from "@/lib/store";

import type {
  MonacoCompletionDisposable,
  MonacoEditorInstance,
  MonacoPosition,
  MonacoTextModel,
} from "./monaco-types";

interface UseMonacoQueryEditorArgs {
  tabId: string;
  query: string;
  connectionId: string;
  completionContext: SqlCompletionContext;
  isRunning: boolean;
  loadTableStructure: (
    connectionId: string,
    schema: string,
    table: string,
  ) => Promise<void>;
  runQuery: (
    tabId: string,
    options?: { overrideSql?: string },
  ) => Promise<QueryOutcome>;
  onOutcome: (outcome: QueryOutcome) => void;
}

export interface MonacoQueryEditor {
  onMount: OnMount;
  cursor: MonacoPosition;
  handleRunCurrent: () => void;
  handleRunSelection: () => void;
  handleRunAll: () => void;
}

export function useMonacoQueryEditor({
  tabId,
  query,
  connectionId,
  completionContext,
  isRunning,
  loadTableStructure,
  runQuery,
  onOutcome,
}: UseMonacoQueryEditorArgs): MonacoQueryEditor {
  const [cursor, setCursor] = useState<MonacoPosition>({
    lineNumber: 1,
    column: 1,
  });

  const editorRef = useRef<MonacoEditorInstance | null>(null);
  const completionDisposableRef = useRef<MonacoCompletionDisposable | null>(
    null,
  );
  const editorDisposablesRef = useRef<MonacoCompletionDisposable[]>([]);
  const decorationsCollectionRef = useRef<{
    set: (decorations: unknown[]) => void;
    clear: () => void;
  } | null>(null);
  const completionContextRef = useRef<SqlCompletionContext>(completionContext);
  completionContextRef.current = completionContext;

  const getEditorSelectionText = useCallback((): string => {
    const editor = editorRef.current;
    if (!editor) return "";
    const selection = editor.getSelection();
    const model = editor.getModel();
    if (!selection || !model) return "";
    return model.getValueInRange(selection) ?? "";
  }, []);

  const getCurrentStatementText = useCallback((): string => {
    const editor = editorRef.current;
    const model = editor?.getModel();
    const position = editor?.getPosition();
    if (!model || !position) return query;
    return (
      getSqlStatementAtPosition(
        model.getValue(),
        position.lineNumber,
        position.column,
      )?.sql ?? ""
    );
  }, [query]);

  const runSql = useCallback(
    async (sql: string) => {
      if (!sql.trim() || isRunning) return;
      const requestedTabId = tabId;
      const outcome = await runQuery(requestedTabId, { overrideSql: sql });
      // Drop the outcome if the user switched tabs while the query
      // was in flight — the panel instance is reused across query
      // tabs (workspace-view.tsx renders no React `key`), so without
      // this guard the closure's `onOutcome` would write tab A's
      // result into tab B's banner.
      if (requestedTabId !== tabId) return;
      // Skip `noop` so a rapid double-click (the second call short-
      // circuits while the first is still running) can't wipe the
      // completed banner the first call just produced.
      if (outcome.kind !== "noop") onOutcome(outcome);
    },
    [isRunning, onOutcome, runQuery, tabId],
  );

  const handleRunCurrent = useCallback(() => {
    void runSql(getCurrentStatementText());
  }, [getCurrentStatementText, runSql]);

  const handleRunSelection = useCallback(() => {
    void runSql(getEditorSelectionText());
  }, [getEditorSelectionText, runSql]);

  const handleRunAll = useCallback(() => {
    void runSql(query);
  }, [runSql, query]);

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

  const onMount = useCallback<OnMount>(
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
          void runSql(query);
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
        if (event.target.type !== 2 || !event.target.position) return;
        const latestModel = editor.getModel() as MonacoTextModel | null;
        if (!latestModel) return;
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
                connectionId,
                predicateTable.schema,
                predicateTable.table,
              );
              const latestState = useAppStore.getState();
              const status = latestState.tableStructureStatus[key]?.state;
              if (!latestState.tableStructure[key] && status !== "loading") {
                await loadTableStructure(
                  connectionId,
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
            ).map((item) => ({
              label: item.label,
              insertText: item.insertText,
              kind: kindByType[item.kind],
              detail: item.detail,
              sortText: item.sortText,
              range,
            }));

            return { suggestions };
          },
        });
    },
    [
      connectionId,
      getEditorSelectionText,
      loadTableStructure,
      query,
      runSql,
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

  return {
    onMount,
    cursor,
    handleRunCurrent,
    handleRunSelection,
    handleRunAll,
  };
}
