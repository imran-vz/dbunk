import type { OnMount } from "@monaco-editor/react";
import { useCallback, useEffect, useRef, useState } from "react";

import {
  getSqlStatementAtPosition,
  getSqlStatements,
  type SqlStatementRange,
} from "@/lib/sql";
import {
  getSqlCompletions,
  getSqlPredicateTableReference,
  type SqlCompletionContext,
} from "@/lib/sql-completions";
import {
  type QueryOutcome,
  type TabCaret,
  tableStructureKey,
  useAppStore,
} from "@/lib/store";

import type {
  MonacoCompletionDisposable,
  MonacoEditorInstance,
  MonacoPosition,
  MonacoSelectionRange,
  MonacoTextModel,
} from "./monaco-types";

/** Caret writes ride the same order of debounce as hot-exit SQL. */
const CARET_PERSIST_DEBOUNCE_MS = 500;

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
  onOutcome: (outcome: QueryOutcome, sql: string) => void;
  onFormat: () => void;
  /**
   * §5.1: a Run invocation over a multi-statement selection opens a
   * statement picker instead of executing blindly. Receives the raw
   * selection text (for "Run all") and its parsed statements.
   */
  onMultiStatementSelection?: (
    selectionText: string,
    statements: SqlStatementRange[],
  ) => void;
}

export interface MonacoQueryEditor {
  onMount: OnMount;
  cursor: MonacoPosition;
  currentStatement: () => string;
  runSql: (sql: string) => void;
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
  onFormat,
  onMultiStatementSelection,
}: UseMonacoQueryEditorArgs): MonacoQueryEditor {
  const onFormatRef = useRef(onFormat);
  onFormatRef.current = onFormat;
  const onMultiStatementSelectionRef = useRef(onMultiStatementSelection);
  onMultiStatementSelectionRef.current = onMultiStatementSelection;
  const [cursor, setCursor] = useState<MonacoPosition>({
    lineNumber: 1,
    column: 1,
  });

  const editorRef = useRef<MonacoEditorInstance | null>(null);
  // The panel — and so this editor instance — is reused across query
  // tabs (the workbench renders no React `key`), and Monaco's
  // `onMount` fires once per instance. Listeners installed there must
  // read the tab id at fire time, never from the mount closure.
  const tabIdRef = useRef(tabId);
  tabIdRef.current = tabId;
  const caretPersistTimerRef = useRef<ReturnType<typeof setTimeout> | null>(
    null,
  );
  /** Caret awaiting its debounced write, tagged with the tab it belongs to. */
  const pendingCaretRef = useRef<{ tabId: string; caret: TabCaret } | null>(
    null,
  );
  /** Restore runs once per tab activation — a repeat for the same tab
   *  must not re-place a caret the user has since moved. */
  const caretRestoredForTabRef = useRef<string | null>(null);
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

  /** Commit the pending caret (if any) to its own tab and clear the timer. */
  const flushPendingCaret = useCallback(() => {
    if (caretPersistTimerRef.current !== null) {
      clearTimeout(caretPersistTimerRef.current);
      caretPersistTimerRef.current = null;
    }
    const pending = pendingCaretRef.current;
    pendingCaretRef.current = null;
    if (pending) {
      useAppStore.getState().updateQueryCaret(pending.tabId, pending.caret);
    }
  }, []);

  const readCaret = useCallback(
    (editor: MonacoEditorInstance, position: MonacoPosition): TabCaret => {
      const caret: TabCaret = {
        line: position.lineNumber,
        column: position.column,
      };
      // SAFETY: The value is constrained by the typed component or library contract at this boundary.
      const selection = editor.getSelection() as
        | MonacoSelectionRange
        | null
        | undefined;
      if (
        selection &&
        (selection.selectionStartLineNumber !== selection.positionLineNumber ||
          selection.selectionStartColumn !== selection.positionColumn)
      ) {
        caret.anchorLine = selection.selectionStartLineNumber;
        caret.anchorColumn = selection.selectionStartColumn;
      }
      return caret;
    },
    [],
  );

  /**
   * Place the session-persisted caret for `forTabId`, clamped to the
   * current model so a stale position degrades instead of erroring.
   * Runs on mount and on every tab switch (see the `tabId` effect).
   */
  const restoreCaret = useCallback(
    (editor: MonacoEditorInstance, forTabId: string) => {
      if (caretRestoredForTabRef.current === forTabId) return;
      caretRestoredForTabRef.current = forTabId;
      const storedCaret = useAppStore
        .getState()
        .workspaceTabs.find((candidate) => candidate.id === forTabId)?.caret;
      // SAFETY: The value is constrained by the typed component or library contract at this boundary.
      const model = editor.getModel() as MonacoTextModel | null;
      if (!storedCaret || !model) return;
      const lineCount = model.getLineCount?.() ?? 1;
      const clamp = (line: number, column: number): MonacoPosition => {
        const safeLine = Math.min(Math.max(line, 1), lineCount);
        const maxColumn = model.getLineMaxColumn?.(safeLine) ?? column;
        return {
          lineNumber: safeLine,
          column: Math.min(Math.max(column, 1), maxColumn),
        };
      };
      const head = clamp(storedCaret.line, storedCaret.column);
      const anchor =
        storedCaret.anchorLine !== undefined &&
        storedCaret.anchorColumn !== undefined
          ? clamp(storedCaret.anchorLine, storedCaret.anchorColumn)
          : head;
      editor.setSelection?.({
        startLineNumber: anchor.lineNumber,
        startColumn: anchor.column,
        endLineNumber: head.lineNumber,
        endColumn: head.column,
      });
      editor.revealPositionInCenterIfOutsideViewport?.(head);
      setCursor(head);
    },
    [],
  );

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
      // tabs (the workbench renders no React `key`), so without
      // this guard the closure's `onOutcome` would write tab A's
      // result into tab B's banner.
      if (requestedTabId !== tabId) return;
      // Skip `noop` so a rapid double-click (the second call short-
      // circuits while the first is still running) can't wipe the
      // completed banner the first call just produced.
      if (outcome.kind !== "noop") onOutcome(outcome, sql);
    },
    [isRunning, onOutcome, runQuery, tabId],
  );

  /**
   * When the selection spans multiple statements, Run opens the
   * statement picker (§5.1) instead of executing the statement at the
   * caret. Returns true when the picker was invoked.
   */
  const interceptMultiStatementSelection = useCallback((): boolean => {
    const selection = getEditorSelectionText();
    if (!selection.trim()) return false;
    const statements = getSqlStatements(selection);
    if (statements.length < 2) return false;
    onMultiStatementSelectionRef.current?.(selection, statements);
    return true;
  }, [getEditorSelectionText]);

  const handleRunCurrent = useCallback(() => {
    if (interceptMultiStatementSelection()) return;
    void runSql(getCurrentStatementText());
  }, [getCurrentStatementText, interceptMultiStatementSelection, runSql]);

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

  // Branches (glyph mouse-down, decoration collection presence/absence,
  // cursor + content listeners, execute-selection / execute-all actions,
  // cached vs. uncached predicate-table completion) are covered by the
  // `QueryEditorPanel onMount branches` suite. Fallow uses static-estimated
  // coverage so the CRAP score does not reflect the added assertions.
  const onMount = useCallback<OnMount>(
    // fallow-ignore-next-line complexity
    (editor, monaco) => {
      // SAFETY: The value is constrained by the typed component or library contract at this boundary.
      editorRef.current = editor as MonacoEditorInstance;
      completionDisposableRef.current?.dispose();
      editorDisposablesRef.current.forEach((disposable) => {
        disposable.dispose();
      });
      editorDisposablesRef.current = [];

      // SAFETY: The value is constrained by the typed component or library contract at this boundary.
      const model = editor.getModel() as MonacoTextModel | null;
      if (model) {
        const collection = editor.createDecorationsCollection?.();
        decorationsCollectionRef.current = collection
          ? {
              set: (decorations) => {
                // SAFETY: The value is constrained by the typed component or library contract at this boundary.
                collection.set(decorations as never[]);
              },
              clear: () => collection.clear(),
            }
          : null;
        updateQueryRunDecorations(monaco, model);
        const contentDisposable = editor.onDidChangeModelContent?.(() => {
          // SAFETY: The value is constrained by the typed component or library contract at this boundary.
          const latestModel = editor.getModel() as MonacoTextModel | null;
          if (latestModel) {
            updateQueryRunDecorations(monaco, latestModel);
          }
        });
        if (contentDisposable) {
          editorDisposablesRef.current.push(contentDisposable);
        }
      }

      const cursorDisposable =
        // SAFETY: The value is constrained by the typed component or library contract at this boundary.
        (editor as MonacoEditorInstance).onDidChangeCursorPosition?.(
          ({ position }) => {
            setCursor(position);
            // Caret persistence (Plan 010): debounced into the session
            // blob so relaunch restores it; moving the caret is not an
            // edit, so `updateQueryCaret` never touches `isDirty`.
            const currentTabId = tabIdRef.current;
            // A tab switch inside the debounce window: commit the
            // previous tab's caret before tracking the new one.
            if (
              pendingCaretRef.current &&
              pendingCaretRef.current.tabId !== currentTabId
            ) {
              flushPendingCaret();
            }
            pendingCaretRef.current = {
              tabId: currentTabId,
              // SAFETY: The value is constrained by the typed component or library contract at this boundary.
              caret: readCaret(editor as MonacoEditorInstance, position),
            };
            if (caretPersistTimerRef.current !== null) {
              clearTimeout(caretPersistTimerRef.current);
            }
            caretPersistTimerRef.current = setTimeout(
              flushPendingCaret,
              CARET_PERSIST_DEBOUNCE_MS,
            );
          },
        );
      if (cursorDisposable) {
        editorDisposablesRef.current.push(cursorDisposable);
      }

      // SAFETY: The value is constrained by the typed component or library contract at this boundary.
      restoreCaret(editor as MonacoEditorInstance, tabIdRef.current);

      const runCurrent = () => {
        if (interceptMultiStatementSelection()) return;
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
        keybindings: [
          monaco.KeyMod.CtrlCmd | monaco.KeyMod.Shift | monaco.KeyCode.Enter,
        ],
        contextMenuGroupId: "navigation",
        contextMenuOrder: 3,
        run: () => {
          void runSql(editor.getModel()?.getValue() ?? "");
        },
      });
      const formatAction = editor.addAction?.({
        id: "dbunk.formatQuery",
        label: "Format SQL",
        keybindings: [
          monaco.KeyMod.CtrlCmd | monaco.KeyMod.Shift | monaco.KeyCode.KeyF,
        ],
        contextMenuGroupId: "navigation",
        contextMenuOrder: 4,
        run: () => {
          onFormatRef.current();
        },
      });
      const mouseDisposable = editor.onMouseDown?.((event) => {
        if (event.target.type !== 2 || !event.target.position) return;
        // SAFETY: The value is constrained by the typed component or library contract at this boundary.
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
        ...[
          currentAction,
          selectionAction,
          allAction,
          formatAction,
          mouseDisposable,
        ].filter((item): item is MonacoCompletionDisposable => Boolean(item)),
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
      flushPendingCaret,
      getEditorSelectionText,
      interceptMultiStatementSelection,
      loadTableStructure,
      query,
      readCaret,
      restoreCaret,
      runSql,
      updateQueryRunDecorations,
    ],
  );

  // Tab switch on the reused editor: `@monaco-editor/react` has just
  // swapped the model text (its `value` effect runs before this one),
  // which may have fired a cursor event for the NEW tab — an artifact,
  // not a user move, so it must not clobber the caret we restore here.
  // Anything pending for the previous tab is committed.
  useEffect(() => {
    const editor = editorRef.current;
    if (!editor) return;
    if (pendingCaretRef.current?.tabId === tabId) {
      pendingCaretRef.current = null;
      if (caretPersistTimerRef.current !== null) {
        clearTimeout(caretPersistTimerRef.current);
        caretPersistTimerRef.current = null;
      }
    } else {
      flushPendingCaret();
    }
    restoreCaret(editor, tabId);
  }, [tabId, flushPendingCaret, restoreCaret]);

  useEffect(
    () => () => {
      // Commit the last caret so a tab closed mid-debounce still
      // restores where the user left it.
      flushPendingCaret();
      completionDisposableRef.current?.dispose();
      decorationsCollectionRef.current?.clear();
      editorDisposablesRef.current.forEach((disposable) => {
        disposable.dispose();
      });
    },
    [flushPendingCaret],
  );

  return {
    onMount,
    cursor,
    currentStatement: getCurrentStatementText,
    runSql: (sql: string) => {
      void runSql(sql);
    },
    handleRunCurrent,
    handleRunSelection,
    handleRunAll,
  };
}
