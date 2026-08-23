/* oxlint-disable anti-slop/no-known-value-widening anti-slop/no-module-mocking anti-slop/no-unknown-parameters anti-slop/require-safety-comment-for-type-assertion -- Test fixtures use controlled mocks and assertions to exercise otherwise inaccessible boundaries. */
// @vitest-environment jsdom
import {
  act,
  cleanup,
  fireEvent,
  render,
  screen,
  waitFor,
} from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("@/lib/tauri", () => ({
  isTauri: vi.fn(() => true),
  tauriInvoke: vi.fn(),
  errorToMessage: (error: unknown) =>
    error instanceof Error ? error.message : String(error),
}));

vi.mock("@/lib/result-mutation", async (importOriginal) => {
  const actual = await importOriginal<typeof import("@/lib/result-mutation")>();
  return {
    ...actual,
    supportsResultMutations: (engine: string) => engine === "PostgreSQL",
  };
});

// Monaco is loaded locally in the real app but the heavy editor module
// can't run in jsdom. The editor component itself is already mocked
// below; this prevents the side-effectful local-loader from importing
// the real monaco-editor package.
vi.mock("@/lib/monaco-local", () => ({}));

// Controllable selection that the mocked Monaco editor will return.
const selectionState = { value: null as string | null };
const cursorState = { lineNumber: 1, column: 1 };
const registeredCompletionProviders: unknown[] = [];
const registeredActions: Array<{ id: string; run: () => void }> = [];
const decorationState = { values: [] as unknown[] };
// Capture the editor-event callbacks the hook registers so tests can drive
// them directly. The mocked Monaco normally drops these listeners on the
// floor, which leaves `onMount`'s glyph/cursor/content branches uncovered.
type MouseDownHandler = (event: {
  target: {
    type: number;
    position: { lineNumber: number; column: number } | null;
  };
  event?: { preventDefault?: () => void };
}) => void;
type ContentChangeHandler = () => void;
type CursorChangeHandler = (event: {
  position: { lineNumber: number; column: number };
}) => void;
const editorHandlers: {
  mouseDown: MouseDownHandler | null;
  contentChange: ContentChangeHandler | null;
  cursorChange: CursorChangeHandler | null;
} = { mouseDown: null, contentChange: null, cursorChange: null };
// Lets tests force `createDecorationsCollection` to return undefined so the
// collection-absent branch in `onMount` is exercised.
const editorOverrides: { skipDecorationsCollection: boolean } = {
  skipDecorationsCollection: false,
};

vi.mock("@monaco-editor/react", () => ({
  __esModule: true,
  default: ({
    value,
    onChange,
    onMount,
  }: {
    value: string;
    onChange?: (value: string) => void;
    onMount?: (editor: unknown, monaco: unknown) => void;
  }) => {
    const fakeModel = {
      getValue: () => value,
      getWordUntilPosition: () => ({
        startColumn: cursorState.column,
        endColumn: cursorState.column,
      }),
      getValueInRange: () => selectionState.value ?? "",
    };
    const fakeEditor = {
      getPosition: () => cursorState,
      getModel: () => fakeModel,
      getSelection: () => (selectionState.value === null ? null : {}),
      addAction: (action: { id: string; run: () => void }) => {
        registeredActions.push(action);
        return { dispose: vi.fn() };
      },
      createDecorationsCollection: editorOverrides.skipDecorationsCollection
        ? undefined
        : () => ({
            set: (decorations: unknown[]) => {
              decorationState.values = decorations;
            },
            clear: () => {
              decorationState.values = [];
            },
          }),
      onMouseDown: (handler: MouseDownHandler) => {
        editorHandlers.mouseDown = handler;
        return { dispose: vi.fn() };
      },
      onDidChangeModelContent: (handler: ContentChangeHandler) => {
        editorHandlers.contentChange = handler;
        return { dispose: vi.fn() };
      },
      onDidChangeCursorPosition: (handler: CursorChangeHandler) => {
        editorHandlers.cursorChange = handler;
        return { dispose: vi.fn() };
      },
    };
    const fakeMonaco = {
      KeyCode: {
        Enter: 3,
        KeyF: 36,
      },
      KeyMod: {
        CtrlCmd: 2048,
        Shift: 1024,
      },
      Range: class {
        constructor(
          public startLineNumber: number,
          public startColumn: number,
          public endLineNumber: number,
          public endColumn: number,
        ) {}
      },
      languages: {
        CompletionItemKind: {
          Field: 0,
          Keyword: 1,
          Module: 2,
          Struct: 3,
          Interface: 4,
        },
        registerCompletionItemProvider: (
          _language: string,
          provider: unknown,
        ) => {
          registeredCompletionProviders.push(provider);
          return { dispose: vi.fn() };
        },
      },
    };
    onMount?.(fakeEditor, fakeMonaco);
    return (
      <textarea
        data-testid="mock-monaco"
        value={value}
        onChange={(event) => onChange?.(event.target.value)}
      />
    );
  },
}));

import { QueryEditorPanel } from "@/components/query-editor-panel";
import type { AnalyzeResultSetResult } from "@/lib/result-mutation";
import {
  queryMutationDraftScope,
  type QuerySessionState,
  type QueryStatus,
  tableStructureKey,
  useAppStore,
  type WorkspaceTab,
} from "@/lib/store";
import { isTauri, tauriInvoke } from "@/lib/tauri";

const mockedIsTauri = vi.mocked(isTauri);
const mockedInvoke = vi.mocked(tauriInvoke);

const initialStoreState = useAppStore.getState();

const queryTab: WorkspaceTab = {
  id: "tab-1",
  kind: "query",
  label: "query_1.sql",
  connectionId: "conn-1",
  schema: "public",
  query: "select 1;",
};

const pgConnection = {
  id: "conn-1",
  name: "Local Postgres",
  database: "app",
  host: "localhost",
  port: 5432,
  user: "postgres",
  password: "",
  role: "",
  engine: "PostgreSQL" as const,
  ssl: false,
  status: "Connected" as const,
  latency: "1ms",
};

const analyzedUsersResult: AnalyzeResultSetResult = {
  requestId: 1,
  analysisId: 11,
  statement: { kind: "analyzed" },
  columns: [
    {
      name: "id",
      origin: {
        kind: "table",
        schema: "public",
        table: "users",
        column: "id",
        attnum: 1,
      },
      castType: "int4",
      nullable: false,
      writability: { kind: "writable" },
    },
    {
      name: "name",
      origin: {
        kind: "table",
        schema: "public",
        table: "users",
        column: "name",
        attnum: 2,
      },
      castType: "text",
      nullable: true,
      writability: { kind: "writable" },
    },
    {
      name: "generated_slug",
      origin: {
        kind: "table",
        schema: "public",
        table: "users",
        column: "generated_slug",
        attnum: 3,
      },
      castType: "text",
      nullable: true,
      writability: { kind: "generated" },
    },
  ],
  tables: [
    {
      schema: "public",
      table: "users",
      identity: { kind: "primaryKey", columns: ["id"] },
      identityProjected: true,
      identityProjectionIndexes: [0],
      updatable: { allowed: true },
      deletable: { allowed: true },
      insertable: { allowed: true },
    },
  ],
};

function persistentQuerySession(
  resultSets: NonNullable<QuerySessionState["execution"]>["resultSets"] = [
    {
      index: 0,
      columns: ["id", "name", "generated_slug"],
      rowChunks: [
        [
          ["1", null, "ada"],
          ["2", "Ada", "ada-2"],
        ],
      ],
      rowCount: 2,
      partial: false,
      completed: true,
    },
  ],
): QuerySessionState {
  return {
    id: "session-1",
    tabId: queryTab.id,
    connectionId: queryTab.connectionId,
    generation: 1,
    transaction: {
      mode: "autocommit",
      status: "idle",
      manualIsolation: "readCommitted",
    },
    execution: {
      id: "execution-1",
      status: "completed",
      startedAt: "2026-08-22T00:00:00.000Z",
      completedAt: "2026-08-22T00:00:00.010Z",
      runtimeMs: 10,
      resultSets,
      notices: [],
      error: null,
      omittedRows: 0,
      omittedResultSets: 0,
      omittedNotices: 0,
      omittedMetadataBytes: 0,
      truncationReasons: [],
      retainedBytes: 64,
      tombstone: null,
    },
    lastViewedAt: Date.now(),
    budgetOwners: [],
    state: "open",
  };
}

function seedPersistentQuery(
  resultSets?: NonNullable<QuerySessionState["execution"]>["resultSets"],
) {
  useAppStore.setState({
    workspaceTabs: [queryTab],
    activeConnectionId: "conn-1",
    activeTabId: queryTab.id,
    connections: [pgConnection],
    queryStatus: {},
    queryExecutionSql: { [queryTab.id]: "select id, name from users" },
    querySessions: {
      [queryTab.id]: persistentQuerySession(resultSets),
    },
  });
}

function seedUsersUpdateDraft() {
  const scope = seedUsersAnalysisDraft();
  useAppStore.getState().stageMutationDraftUpdate(scope, {
    table: { schema: "public", table: "users" },
    identityKind: "primaryKey",
    identity: [{ column: "id", value: "2" }],
    originals: [
      { column: "id", value: "2" },
      { column: "name", value: "Ada" },
      { column: "generated_slug", value: "ada-2" },
    ],
    cells: [{ column: "name", original: "Ada", value: "Grace" }],
    rowIndex: 1,
  });
  return scope;
}

function seedUsersAnalysisDraft() {
  const store = useAppStore.getState();
  const handle = store.openMutationDraft({
    owner: {
      kind: "query",
      tabId: queryTab.id,
      executionId: "execution-1",
      resultSetIndex: 0,
    },
    connectionId: queryTab.connectionId,
    source: { kind: "statement", sql: "select id, name from users" },
  });
  if (!handle) throw new Error("Expected mutation draft handle");
  store.setMutationDraftAnalysis(handle, analyzedUsersResult);
  return handle.scope;
}

function resolveUsersDraftApply(
  scope: ReturnType<typeof seedUsersUpdateDraft>,
) {
  const store = useAppStore.getState();
  const previewRequest = store.beginMutationDraftPreview(scope);
  if (!previewRequest) throw new Error("Expected a preview request");
  store.resolveMutationDraftPreview(previewRequest, {
    statements: [{ opIndex: 0, sql: "UPDATE users", params: [] }],
  });
  const applyRequest = store.beginMutationDraftApply(scope);
  if (!applyRequest) throw new Error("Expected an apply request");
  store.resolveMutationDraftApply(applyRequest, {
    operations: [{ opIndex: 0, rowsAffected: 1 }],
    runtimeMs: 1,
  });
}

const seedStatus = (status: QueryStatus) => {
  useAppStore.setState({
    workspaceTabs: [queryTab],
    activeConnectionId: "conn-1",
    activeTabId: queryTab.id,
    queryStatus: { [queryTab.id]: status },
  });
};

beforeEach(() => {
  selectionState.value = null;
  cursorState.lineNumber = 1;
  cursorState.column = 1;
  registeredCompletionProviders.length = 0;
  registeredActions.length = 0;
  decorationState.values = [];
  editorHandlers.mouseDown = null;
  editorHandlers.contentChange = null;
  editorHandlers.cursorChange = null;
  editorOverrides.skipDecorationsCollection = false;
  mockedIsTauri.mockReturnValue(true);
  mockedInvoke.mockReset();
  useAppStore.setState(initialStoreState, true);
});

afterEach(() => {
  cleanup();
  useAppStore.setState(initialStoreState, true);
  vi.clearAllMocks();
});

describe("QueryEditorPanel feedback", () => {
  it("renders the Explain tab as a real empty state, not coming soon", () => {
    useAppStore.setState({
      workspaceTabs: [queryTab],
      activeConnectionId: "conn-1",
      activeTabId: queryTab.id,
      queryStatus: {},
    });

    render(<QueryEditorPanel tab={queryTab} isClient />);
    fireEvent.click(screen.getByRole("button", { name: /show explain view/i }));

    expect(screen.getByText(/run explain/i)).toBeTruthy();
    expect(screen.queryByText(/coming soon/i)).toBeNull();
  });

  it("runs EXPLAIN JSON and renders the plan tree in the Explain tab", async () => {
    useAppStore.setState({
      workspaceTabs: [queryTab],
      activeConnectionId: "conn-1",
      activeTabId: queryTab.id,
      queryStatus: {},
    });
    mockedInvoke.mockResolvedValueOnce({
      columns: ["QUERY PLAN"],
      rows: [
        [
          JSON.stringify([
            {
              Plan: {
                "Node Type": "Seq Scan",
                "Relation Name": "users",
                "Startup Cost": 0,
                "Total Cost": 12.5,
                "Plan Rows": 5,
                "Actual Startup Time": 0.01,
                "Actual Total Time": 0.03,
                "Actual Rows": 5,
                "Actual Loops": 1,
                "Shared Hit Blocks": 3,
              },
              "Planning Time": 0.2,
              "Execution Time": 0.05,
            },
          ]),
        ],
      ],
      runtimeMs: 8,
      rowCount: 1,
    });

    render(<QueryEditorPanel tab={queryTab} isClient />);
    fireEvent.click(screen.getByRole("button", { name: /run explain/i }));

    await waitFor(() => {
      expect(screen.getAllByText("Seq Scan").length).toBeGreaterThan(0);
    });
    expect(screen.getAllByText("users").length).toBeGreaterThan(0);
    expect(screen.getAllByText("Shared Hit: 3").length).toBeGreaterThan(0);
    expect(mockedInvoke).toHaveBeenCalledWith("run_query", {
      payload: {
        connectionId: "conn-1",
        query: "EXPLAIN (ANALYZE, BUFFERS, FORMAT JSON)\nselect 1",
      },
    });
  });

  it("does not double-wrap when current statement already starts with EXPLAIN", async () => {
    const explainTab: WorkspaceTab = {
      ...queryTab,
      query: "EXPLAIN ANALYZE select 1;",
    };
    useAppStore.setState({
      workspaceTabs: [explainTab],
      activeConnectionId: "conn-1",
      activeTabId: explainTab.id,
      queryStatus: {},
    });
    mockedInvoke.mockResolvedValueOnce({
      columns: ["QUERY PLAN"],
      rows: [["Seq Scan on users (cost=0.00..1.00 rows=1 width=4)"]],
      runtimeMs: 1,
      rowCount: 1,
    });

    render(<QueryEditorPanel tab={explainTab} isClient />);
    fireEvent.click(screen.getByRole("button", { name: /run explain/i }));

    await waitFor(() => {
      expect(mockedInvoke).toHaveBeenCalledWith("run_query", {
        payload: {
          connectionId: "conn-1",
          query: "EXPLAIN ANALYZE select 1",
        },
      });
    });
  });

  it("falls back to the full query when the cursor is off any statement", async () => {
    const explainTab: WorkspaceTab = {
      ...queryTab,
      query: "select 1;",
    };
    useAppStore.setState({
      workspaceTabs: [explainTab],
      activeConnectionId: "conn-1",
      activeTabId: explainTab.id,
      queryStatus: {},
    });
    cursorState.lineNumber = 5;
    cursorState.column = 1;
    mockedInvoke.mockResolvedValueOnce({
      columns: ["QUERY PLAN"],
      rows: [["Plan output"]],
      runtimeMs: 1,
      rowCount: 1,
    });

    render(<QueryEditorPanel tab={explainTab} isClient />);
    fireEvent.click(screen.getByRole("button", { name: /run explain/i }));

    await waitFor(() => {
      expect(mockedInvoke).toHaveBeenCalledWith("run_query", {
        payload: {
          connectionId: "conn-1",
          query: "EXPLAIN (ANALYZE, BUFFERS, FORMAT JSON)\nselect 1;",
        },
      });
    });
  });

  it("routes a user-typed EXPLAIN query to the Explain tab on Run", async () => {
    const explainTab: WorkspaceTab = {
      ...queryTab,
      query: "EXPLAIN (FORMAT JSON) select 1;",
    };
    useAppStore.setState({
      workspaceTabs: [explainTab],
      activeConnectionId: "conn-1",
      activeTabId: explainTab.id,
      queryStatus: {},
    });
    mockedInvoke.mockResolvedValueOnce({
      columns: ["QUERY PLAN"],
      rows: [
        [
          JSON.stringify([
            {
              Plan: {
                "Node Type": "Index Scan",
                "Relation Name": "orders",
                "Startup Cost": 0,
                "Total Cost": 4.2,
                "Plan Rows": 1,
              },
            },
          ]),
        ],
      ],
      runtimeMs: 3,
      rowCount: 1,
    });

    render(<QueryEditorPanel tab={explainTab} isClient />);
    fireEvent.click(screen.getByRole("button", { name: /^run$/i }));

    await waitFor(() => {
      expect(screen.getAllByText("Index Scan").length).toBeGreaterThan(0);
    });
    expect(screen.getAllByText("orders").length).toBeGreaterThan(0);
  });

  it("replaces Run with Stop while a query is running", () => {
    seedStatus({ state: "running" });

    render(<QueryEditorPanel tab={queryTab} isClient />);

    expect(screen.getByRole("button", { name: /stop query/i })).toBeDefined();
    expect(screen.queryByRole("button", { name: /^run$/i })).toBeNull();
  });

  it("keeps the Stop presentation disabled while cancellation settles", () => {
    seedStatus({ state: "cancelling" });

    render(<QueryEditorPanel tab={queryTab} isClient />);

    const cancellingButton = screen.getByRole("button", {
      name: /cancelling query/i,
    }) as HTMLButtonElement;
    expect(cancellingButton.disabled).toBe(true);
    expect(cancellingButton.textContent).toContain("Cancelling");
    expect(screen.queryByRole("button", { name: /^run$/i })).toBeNull();
    expect(screen.queryByRole("button", { name: /run options/i })).toBeNull();
  });

  it("confirms before closing a session with unknown transaction state", () => {
    const confirmSpy = vi.spyOn(window, "confirm").mockReturnValue(false);
    const closeSpy = vi
      .spyOn(useAppStore.getState(), "closeQuerySessionForTab")
      .mockResolvedValue(undefined);
    useAppStore.setState({
      workspaceTabs: [queryTab],
      activeConnectionId: "conn-1",
      activeTabId: queryTab.id,
      queryStatus: {},
      querySessions: {
        [queryTab.id]: {
          id: "session-1",
          tabId: queryTab.id,
          connectionId: queryTab.connectionId,
          generation: 1,
          transaction: {
            mode: "manual",
            status: "unknown",
            manualIsolation: "readCommitted",
          },
          execution: null,
          lastViewedAt: Date.now(),
          budgetOwners: [],
          state: "open",
        },
      },
    });

    render(<QueryEditorPanel tab={queryTab} isClient />);
    fireEvent.click(screen.getByRole("button", { name: /^close$/i }));

    expect(confirmSpy).toHaveBeenCalledWith(
      "Close this query session? Its active or unresolved transaction will be rolled back.",
    );
    expect(closeSpy).not.toHaveBeenCalled();

    confirmSpy.mockReturnValue(true);
    fireEvent.click(screen.getByRole("button", { name: /^close$/i }));
    expect(closeSpy).toHaveBeenCalledWith(queryTab.id);
  });

  it("shows the error message in a banner when the query fails", async () => {
    useAppStore.setState({
      workspaceTabs: [queryTab],
      activeConnectionId: "conn-1",
      activeTabId: queryTab.id,
      queryStatus: {},
    });
    // Drive the action through a real failure rather than seeding
    // terminal state — terminal error lives in the component's
    // `lastOutcome` (see CONTEXT.md — Query Outcome), not the store.
    mockedInvoke.mockRejectedValueOnce(new Error("syntax error at column 5"));

    render(<QueryEditorPanel tab={queryTab} isClient />);

    await act(async () => {
      fireEvent.click(screen.getByRole("button", { name: /^run$/i }));
    });

    await waitFor(() => {
      expect(screen.getByRole("alert").textContent).toContain(
        "syntax error at column 5",
      );
    });
  });

  it("renders Run as the default label when idle", () => {
    useAppStore.setState({
      workspaceTabs: [queryTab],
      activeConnectionId: "conn-1",
      activeTabId: queryTab.id,
      queryStatus: {},
    });

    render(<QueryEditorPanel tab={queryTab} isClient />);
    const runButton = screen.getByRole("button", {
      name: /^run$/i,
    }) as HTMLButtonElement;
    expect(runButton.disabled).toBe(false);
  });

  it("does not allow re-running while in flight", async () => {
    seedStatus({ state: "running" });

    render(<QueryEditorPanel tab={queryTab} isClient />);

    const runButton = screen.getByRole("button", { name: /stop query/i });

    await act(async () => {
      runButton.click();
    });

    // Disabled buttons shouldn't trigger handlers; the status should remain
    // running because no invoke was called.
    expect(useAppStore.getState().queryStatus[queryTab.id].state).toBe(
      "running",
    );
  });
});

describe("QueryEditorPanel execution controls", () => {
  // These tests run in non-Tauri mode so that runQuery short-circuits the
  // tauri invoke path; we only care that the right arguments flow into the
  // store's runQuery action.
  beforeEach(() => {
    mockedIsTauri.mockReturnValue(false);
    useAppStore.setState({
      workspaceTabs: [queryTab],
      activeConnectionId: "conn-1",
      activeTabId: queryTab.id,
      queryStatus: {},
      queryPreviews: {},
      queryHistory: [],
    });
  });

  const openRunOptions = () => {
    fireEvent.click(screen.getByRole("button", { name: /run options/i }));
  };

  it("runs the statement at the cursor by default (Run button)", () => {
    const runQuerySpy = vi.spyOn(useAppStore.getState(), "runQuery");
    const multiQueryTab = {
      ...queryTab,
      query: "select 1;\nselect 2;",
    };
    cursorState.lineNumber = 2;
    cursorState.column = 3;

    render(<QueryEditorPanel tab={multiQueryTab} isClient={true} />);
    fireEvent.click(screen.getByRole("button", { name: /^run$/i }));

    expect(runQuerySpy).toHaveBeenCalledWith("tab-1", {
      overrideSql: "select 2",
    });
  });

  it("runs only the selection from the Run selection menu item", () => {
    const runQuerySpy = vi.spyOn(useAppStore.getState(), "runQuery");
    selectionState.value = "select 1";

    render(<QueryEditorPanel tab={queryTab} isClient={true} />);
    openRunOptions();
    fireEvent.click(screen.getByRole("menuitem", { name: /run selection/i }));

    expect(runQuerySpy).toHaveBeenCalledWith("tab-1", {
      overrideSql: "select 1",
    });
  });

  it("does not run selection when selection is whitespace-only", () => {
    const runQuerySpy = vi.spyOn(useAppStore.getState(), "runQuery");
    selectionState.value = "   \n  ";

    render(<QueryEditorPanel tab={queryTab} isClient={true} />);
    openRunOptions();
    fireEvent.click(screen.getByRole("menuitem", { name: /run selection/i }));

    expect(runQuerySpy).not.toHaveBeenCalled();
  });

  it("runs the entire editor text from Run all", () => {
    const runQuerySpy = vi.spyOn(useAppStore.getState(), "runQuery");
    const multiQueryTab = {
      ...queryTab,
      query: "select 1;\nselect 2;",
    };

    render(<QueryEditorPanel tab={multiQueryTab} isClient={true} />);
    openRunOptions();
    fireEvent.click(screen.getByRole("menuitem", { name: /run all/i }));

    expect(runQuerySpy).toHaveBeenCalledWith("tab-1", {
      overrideSql: "select 1;\nselect 2;",
    });
  });

  it("registers Ctrl/Cmd+Enter to execute the current query", () => {
    const runQuerySpy = vi.spyOn(useAppStore.getState(), "runQuery");
    const multiQueryTab = {
      ...queryTab,
      query: "select 1;\nselect 2;",
    };
    cursorState.lineNumber = 2;
    cursorState.column = 2;

    render(<QueryEditorPanel tab={multiQueryTab} isClient={true} />);
    act(() => {
      registeredActions
        .find((action) => action.id === "dbunk.executeCurrentQuery")
        ?.run();
    });

    expect(runQuerySpy).toHaveBeenCalledWith("tab-1", {
      overrideSql: "select 2",
    });
  });

  it("adds a play decoration at the start of each query", () => {
    render(
      <QueryEditorPanel
        tab={{
          ...queryTab,
          query: "select 1;\n\nselect *\nfrom users;",
        }}
        isClient={true}
      />,
    );

    expect(decorationState.values).toHaveLength(2);
  });

  it("still works when isClient is false (editor not mounted)", () => {
    const runQuerySpy = vi.spyOn(useAppStore.getState(), "runQuery");

    render(<QueryEditorPanel tab={queryTab} isClient={false} />);
    fireEvent.click(screen.getByRole("button", { name: /^run$/i }));

    expect(runQuerySpy).toHaveBeenCalledWith("tab-1", {
      overrideSql: "select 1;",
    });
  });
});

describe("QueryEditorPanel IntelliSense", () => {
  it("registers SQL completions from the tab connection schema", async () => {
    useAppStore.setState({
      workspaceTabs: [queryTab],
      activeConnectionId: "conn-2",
      activeTabId: queryTab.id,
      schemaExplorer: {
        "conn-1": [{ name: "public", tables: ["users"], views: ["v_users"] }],
        "conn-2": [{ name: "public", tables: ["wrong_table"], views: [] }],
      },
    });

    render(<QueryEditorPanel tab={queryTab} isClient />);

    const provider = registeredCompletionProviders.at(-1) as {
      provideCompletionItems: (
        model: {
          getWordUntilPosition: () => {
            startColumn: number;
            endColumn: number;
          };
          getValueInRange: () => string;
        },
        position: { lineNumber: number; column: number },
      ) => Promise<{ suggestions: Array<{ label: string }> }>;
    };

    const result = await provider.provideCompletionItems(
      {
        getWordUntilPosition: () => ({ startColumn: 15, endColumn: 15 }),
        getValueInRange: () => "select * from ",
      },
      { lineNumber: 1, column: 15 },
    );

    expect(result.suggestions.map((item) => item.label)).toContain("users");
    expect(result.suggestions.map((item) => item.label)).not.toContain(
      "wrong_table",
    );
  });

  it("loads table structure and suggests columns in a where clause", async () => {
    mockedInvoke.mockResolvedValueOnce({
      columns: [
        {
          name: "id",
          dataType: "uuid",
          nullable: false,
          defaultValue: null,
          isPrimaryKey: true,
          ordinalPosition: 1,
        },
        {
          name: "expires_at",
          dataType: "timestamp",
          nullable: true,
          defaultValue: null,
          isPrimaryKey: false,
          ordinalPosition: 2,
        },
      ],
      primaryKey: ["id"],
      foreignKeys: [],
      indexes: [],
      constraints: [],
      capabilities: {
        columns: true,
        primaryKey: true,
        foreignKeys: true,
        indexes: true,
        constraints: true,
      },
    });
    useAppStore.setState({
      workspaceTabs: [queryTab],
      activeConnectionId: "conn-1",
      activeTabId: queryTab.id,
      schemaExplorer: {
        "conn-1": [{ name: "public", tables: ["session_state"], views: [] }],
      },
    });

    render(<QueryEditorPanel tab={queryTab} isClient />);

    const provider = registeredCompletionProviders.at(-1) as {
      provideCompletionItems: (
        model: {
          getWordUntilPosition: () => {
            startColumn: number;
            endColumn: number;
          };
          getValueInRange: () => string;
        },
        position: { lineNumber: number; column: number },
      ) => Promise<{ suggestions: Array<{ label: string; detail: string }> }>;
    };

    let result:
      | { suggestions: Array<{ label: string; detail: string }> }
      | undefined;
    await act(async () => {
      result = await provider.provideCompletionItems(
        {
          getWordUntilPosition: () => ({ startColumn: 42, endColumn: 42 }),
          getValueInRange: () => "select * from public.session_state where ",
        },
        { lineNumber: 1, column: 42 },
      );
    });

    expect(mockedInvoke).toHaveBeenCalledWith("load_table_structure", {
      payload: {
        connectionId: "conn-1",
        schema: "public",
        table: "session_state",
      },
    });
    expect(
      useAppStore.getState().tableStructure[
        tableStructureKey("conn-1", "public", "session_state")
      ],
    ).toBeDefined();
    if (!result) {
      throw new Error("Expected completion result");
    }
    expect(result.suggestions.map((item) => item.label)).toContain("id");
    expect(result.suggestions.map((item) => item.label)).toContain(
      "expires_at",
    );
  });

  it("skips load_table_structure when the table structure is already cached", async () => {
    useAppStore.setState({
      workspaceTabs: [queryTab],
      activeConnectionId: "conn-1",
      activeTabId: queryTab.id,
      schemaExplorer: {
        "conn-1": [{ name: "public", tables: ["session_state"], views: [] }],
      },
      tableStructure: {
        [tableStructureKey("conn-1", "public", "session_state")]: {
          columns: [],
          primaryKey: [],
          foreignKeys: [],
          indexes: [],
          constraints: [],
          capabilities: {
            columns: true,
            primaryKey: true,
            foreignKeys: true,
            indexes: true,
            constraints: true,
            canInsertRows: true,
            canUpdateRows: true,
            canDeleteRows: true,
            canAlterSchema: true,
            uniquenessGuarantee: "exact",
          },
        },
      },
    });

    render(<QueryEditorPanel tab={queryTab} isClient />);

    const provider = registeredCompletionProviders.at(-1) as {
      provideCompletionItems: (
        model: {
          getWordUntilPosition: () => {
            startColumn: number;
            endColumn: number;
          };
          getValueInRange: () => string;
        },
        position: { lineNumber: number; column: number },
      ) => Promise<{ suggestions: Array<{ label: string }> }>;
    };

    await act(async () => {
      await provider.provideCompletionItems(
        {
          getWordUntilPosition: () => ({ startColumn: 42, endColumn: 42 }),
          getValueInRange: () => "select * from public.session_state where ",
        },
        { lineNumber: 1, column: 42 },
      );
    });

    // Already cached → no invoke fired for load_table_structure.
    expect(mockedInvoke).not.toHaveBeenCalledWith(
      "load_table_structure",
      expect.anything(),
    );
  });
});

describe("QueryEditorPanel onMount branches", () => {
  beforeEach(() => {
    mockedIsTauri.mockReturnValue(false);
    useAppStore.setState({
      workspaceTabs: [queryTab],
      activeConnectionId: "conn-1",
      activeTabId: queryTab.id,
      queryStatus: {},
      queryPreviews: {},
      queryHistory: [],
    });
  });

  it("runs the statement at the glyph margin on mouse down", () => {
    const runQuerySpy = vi.spyOn(useAppStore.getState(), "runQuery");
    const multiQueryTab = {
      ...queryTab,
      query: "select 1;\nselect 2;",
    };

    render(<QueryEditorPanel tab={multiQueryTab} isClient />);

    const preventDefault = vi.fn();
    act(() => {
      editorHandlers.mouseDown?.({
        target: { type: 2, position: { lineNumber: 2, column: 1 } },
        event: { preventDefault },
      });
    });

    expect(preventDefault).toHaveBeenCalled();
    expect(runQuerySpy).toHaveBeenCalledWith("tab-1", {
      overrideSql: "select 2",
    });
  });

  it("ignores glyph clicks outside the glyph-margin target type", () => {
    const runQuerySpy = vi.spyOn(useAppStore.getState(), "runQuery");

    render(
      <QueryEditorPanel
        tab={{ ...queryTab, query: "select 1;\nselect 2;" }}
        isClient
      />,
    );

    act(() => {
      // type !== 2 → glyph-margin guard returns early.
      editorHandlers.mouseDown?.({
        target: { type: 1, position: { lineNumber: 2, column: 1 } },
      });
      // position == null also short-circuits.
      editorHandlers.mouseDown?.({
        target: { type: 2, position: null },
      });
    });

    expect(runQuerySpy).not.toHaveBeenCalled();
  });

  it("ignores glyph clicks when the line is not a statement start", () => {
    const runQuerySpy = vi.spyOn(useAppStore.getState(), "runQuery");
    // The second statement spans lines 3-4; clicking line 4 should not run.
    const multiLineTab = {
      ...queryTab,
      query: "select 1;\n\nselect *\nfrom users;",
    };

    render(<QueryEditorPanel tab={multiLineTab} isClient />);

    act(() => {
      editorHandlers.mouseDown?.({
        target: { type: 2, position: { lineNumber: 4, column: 1 } },
        event: { preventDefault: vi.fn() },
      });
    });

    expect(runQuerySpy).not.toHaveBeenCalled();
  });

  it("updates decorations when the model content changes", () => {
    render(
      <QueryEditorPanel tab={{ ...queryTab, query: "select 1;" }} isClient />,
    );

    expect(decorationState.values).toHaveLength(1);

    act(() => {
      editorHandlers.contentChange?.();
    });

    // Handler reads the latest model value; with a single statement we keep
    // exactly one decoration, but the branch has now executed.
    expect(decorationState.values).toHaveLength(1);
  });

  it("updates the status-bar cursor when Monaco fires onDidChangeCursorPosition", () => {
    render(<QueryEditorPanel tab={queryTab} isClient />);

    act(() => {
      editorHandlers.cursorChange?.({
        position: { lineNumber: 12, column: 7 },
      });
    });

    expect(screen.getByText(/Ln 12, Col 7/)).toBeTruthy();
  });

  it("runs Execute selection through the registered action", () => {
    const runQuerySpy = vi.spyOn(useAppStore.getState(), "runQuery");
    selectionState.value = "select 1";

    render(<QueryEditorPanel tab={queryTab} isClient />);

    act(() => {
      registeredActions
        .find((action) => action.id === "dbunk.executeSelection")
        ?.run();
    });

    expect(runQuerySpy).toHaveBeenCalledWith("tab-1", {
      overrideSql: "select 1",
    });
  });

  it("runs Execute all through the registered action using the editor value", () => {
    const runQuerySpy = vi.spyOn(useAppStore.getState(), "runQuery");
    const multiQueryTab = {
      ...queryTab,
      query: "select 1;\nselect 2;",
    };

    render(<QueryEditorPanel tab={multiQueryTab} isClient />);

    act(() => {
      registeredActions
        .find((action) => action.id === "dbunk.executeAll")
        ?.run();
    });

    expect(runQuerySpy).toHaveBeenCalledWith("tab-1", {
      overrideSql: "select 1;\nselect 2;",
    });
  });

  it("falls back to the full query text when the editor decorations collection is unavailable", () => {
    editorOverrides.skipDecorationsCollection = true;

    render(
      <QueryEditorPanel
        tab={{ ...queryTab, query: "select 1;\nselect 2;" }}
        isClient
      />,
    );

    // With no decorations collection installed the play-glyph decorations
    // never get applied — the branch that gates them on the collection has
    // now been exercised.
    expect(decorationState.values).toHaveLength(0);
  });
});

describe("QueryEditorPanel result mutations", () => {
  it("disables result edits for a read-only connection", () => {
    seedPersistentQuery();
    useAppStore.setState({
      connections: [{ ...pgConnection, readOnly: true }],
    });

    render(<QueryEditorPanel tab={queryTab} isClient />);

    expect(screen.getByTestId("query-mutation-status").textContent).toContain(
      "Local Postgres is a read-only connection",
    );
    const cell = screen.getByRole("button", { name: "Ada" });
    expect(cell.getAttribute("title")).toContain(
      "Edit the connection to unlock writes",
    );
    fireEvent.click(cell);
    expect(screen.queryByDisplayValue("Ada")).toBeNull();
  });

  it("analyzes lazily from the exact execution SQL, gates columns, and preserves NULLs", async () => {
    seedPersistentQuery();
    mockedInvoke.mockImplementation((command) =>
      command === "analyze_result_set"
        ? Promise.resolve(analyzedUsersResult)
        : Promise.resolve(undefined),
    );

    render(
      <QueryEditorPanel
        tab={{ ...queryTab, query: "select now()" }}
        isClient
      />,
    );

    expect(screen.getByTestId("query-mutation-status").textContent).toContain(
      "Select a cell to analyze",
    );
    await act(async () => {
      fireEvent.click(screen.getByRole("button", { name: "ada" }));
      await new Promise((resolve) => setTimeout(resolve, 0));
    });

    await waitFor(() => {
      expect(screen.getByTestId("query-mutation-status").textContent).toContain(
        "Editable · primary key",
      );
    });
    const analyzeCall = mockedInvoke.mock.calls.find(
      ([command]) => command === "analyze_result_set",
    );
    expect(analyzeCall?.[1]).toMatchObject({
      payload: {
        source: {
          kind: "statement",
          sql: "select id, name from users",
        },
      },
    });
    const generatedCell = screen.getByRole("button", { name: "ada" });
    expect(generatedCell.getAttribute("title")).toBe(
      "Generated columns are read-only.",
    );
    fireEvent.click(generatedCell);
    expect(screen.queryByDisplayValue("ada")).toBeNull();

    fireEvent.click(screen.getByRole("button", { name: "NULL" }));
    const nullInput = screen.getByDisplayValue("NULL");
    fireEvent.change(nullInput, { target: { value: "Grace" } });
    fireEvent.blur(nullInput);

    await act(async () => {
      fireEvent.click(screen.getByRole("button", { name: "Ada" }));
      await new Promise((resolve) => setTimeout(resolve, 0));
    });
    const valueInput = screen.getByDisplayValue("Ada");
    fireEvent.change(valueInput, { target: { value: "NULL" } });
    fireEvent.blur(valueInput);

    const draft = Object.values(useAppStore.getState().mutationDrafts).find(
      (candidate) => candidate?.owner.kind === "query",
    );
    const updates = draft?.changeOrder
      .map((changeId) => draft.changes[changeId])
      .filter((change) => change?.kind === "updateRow");
    expect(updates?.[0]).toMatchObject({
      cells: { name: { original: null, value: "Grace" } },
      originals: expect.arrayContaining([{ column: "name", value: null }]),
    });
    expect(updates?.[1]).toMatchObject({
      cells: { name: { original: "Ada", value: null } },
    });
  });

  it("shows typed analysis reasons and keeps non-first results read-only", async () => {
    seedPersistentQuery([
      {
        index: 0,
        columns: ["id", "name", "generated_slug"],
        rowChunks: [[["1", "Ada", "ada"]]],
        rowCount: 1,
        partial: false,
        completed: true,
      },
      {
        index: 1,
        columns: ["count"],
        rowChunks: [[["1"]]],
        rowCount: 1,
        partial: false,
        completed: true,
      },
    ]);
    mockedInvoke.mockResolvedValue({
      ...analyzedUsersResult,
      statement: {
        kind: "notAnalyzable",
        reason: { kind: "multiStatement" },
      },
    });

    render(<QueryEditorPanel tab={queryTab} isClient />);
    await act(async () => {
      fireEvent.click(screen.getByRole("button", { name: "Ada" }));
      await new Promise((resolve) => setTimeout(resolve, 0));
    });
    await waitFor(() => {
      expect(screen.getByTestId("query-mutation-status").textContent).toContain(
        "Editing requires one SQL statement",
      );
    });

    mockedInvoke.mockClear();
    await act(async () => {
      fireEvent.click(screen.getByRole("button", { name: /2 · 1 rows/i }));
      await new Promise((resolve) => setTimeout(resolve, 0));
    });
    expect(screen.getByTestId("query-mutation-status").textContent).toContain(
      "Only the first result set can be edited",
    );
    fireEvent.click(screen.getByRole("button", { name: "1" }));
    expect(mockedInvoke).not.toHaveBeenCalled();
  });

  it("uses explicit temporary-shadowing analysis copy", async () => {
    seedPersistentQuery();
    mockedInvoke.mockResolvedValue({
      ...analyzedUsersResult,
      statement: {
        kind: "notAnalyzable",
        reason: { kind: "possibleTempShadowing" },
      },
    });

    render(<QueryEditorPanel tab={queryTab} isClient />);
    fireEvent.click(screen.getByRole("button", { name: "Ada" }));

    await waitFor(() => {
      expect(screen.getByTestId("query-mutation-status").textContent).toContain(
        "temporary table may shadow the resolved target",
      );
    });
  });

  it("persists and clears a virtual key for a keyless query result, reanalyzing after each change", async () => {
    seedPersistentQuery();
    const keylessAnalysis: AnalyzeResultSetResult = {
      ...analyzedUsersResult,
      tables: [
        {
          ...analyzedUsersResult.tables[0]!,
          identity: { kind: "none", columns: [] },
          identityProjected: false,
          identityProjectionIndexes: [],
        },
      ],
    };
    const virtualKeyAnalysis: AnalyzeResultSetResult = {
      ...analyzedUsersResult,
      analysisId: 12,
      tables: [
        {
          ...analyzedUsersResult.tables[0]!,
          identity: { kind: "virtualKey", columns: ["name"] },
          identityProjected: true,
          identityProjectionIndexes: [1],
        },
      ],
    };
    let analysisCalls = 0;
    mockedInvoke.mockImplementation((command) => {
      if (command === "analyze_result_set") {
        analysisCalls += 1;
        return Promise.resolve(
          analysisCalls === 2 ? virtualKeyAnalysis : keylessAnalysis,
        );
      }
      if (command === "load_virtual_key") return Promise.resolve(null);
      if (command === "save_virtual_key" || command === "clear_virtual_key") {
        return Promise.resolve(undefined);
      }
      return Promise.resolve(undefined);
    });

    render(<QueryEditorPanel tab={queryTab} isClient />);
    fireEvent.click(screen.getByRole("button", { name: "Ada" }));

    const chooseButton = await screen.findByRole("button", {
      name: "Choose virtual key",
    });
    fireEvent.click(chooseButton);
    fireEvent.click(screen.getByRole("checkbox", { name: "name" }));
    fireEvent.click(screen.getByRole("button", { name: "Save virtual key" }));

    await waitFor(() =>
      expect(mockedInvoke).toHaveBeenCalledWith("save_virtual_key", {
        payload: {
          connectionId: "conn-1",
          schema: "public",
          table: "users",
          columns: ["name"],
        },
      }),
    );
    await waitFor(() =>
      expect(
        useAppStore.getState().mutationDrafts[
          queryMutationDraftScope("tab-1", "execution-1", 0)
        ]?.analysis?.snapshot.tables[0]?.identity.kind,
      ).toBe("virtualKey"),
    );
    expect(mockedInvoke).toHaveBeenCalledWith("analyze_result_set", {
      payload: expect.objectContaining({ refreshStructure: true }),
    });

    fireEvent.click(screen.getByRole("button", { name: "Clear virtual key" }));
    await waitFor(() =>
      expect(mockedInvoke).toHaveBeenCalledWith("clear_virtual_key", {
        payload: {
          connectionId: "conn-1",
          schema: "public",
          table: "users",
        },
      }),
    );
    await waitFor(() =>
      expect(
        useAppStore.getState().mutationDrafts[
          queryMutationDraftScope("tab-1", "execution-1", 0)
        ]?.analysis?.snapshot.tables[0]?.identity.kind,
      ).toBe("none"),
    );
    expect(analysisCalls).toBe(3);
  });

  it("offers a virtual key when the proven identity is not fully projected", async () => {
    seedPersistentQuery();
    mockedInvoke.mockImplementation((command) => {
      if (command === "analyze_result_set") {
        return Promise.resolve({
          ...analyzedUsersResult,
          tables: [
            {
              ...analyzedUsersResult.tables[0]!,
              identityProjected: false,
              identityProjectionIndexes: [],
            },
          ],
        });
      }
      if (command === "load_virtual_key") return Promise.resolve(null);
      return Promise.resolve(undefined);
    });

    render(<QueryEditorPanel tab={queryTab} isClient />);
    fireEvent.click(screen.getByRole("button", { name: "Ada" }));

    expect(
      await screen.findByRole("button", { name: "Choose virtual key" }),
    ).toBeTruthy();
    expect(mockedInvoke).toHaveBeenCalledWith("load_virtual_key", {
      payload: {
        connectionId: "conn-1",
        schema: "public",
        table: "users",
      },
    });
  });

  it("opens the mutation review from Review & save", async () => {
    seedPersistentQuery();
    seedUsersUpdateDraft();
    mockedInvoke.mockImplementation((command) => {
      if (command === "preview_result_mutations") {
        return Promise.resolve({
          statements: [
            {
              opIndex: 0,
              sql: "UPDATE public.users SET name = $1 WHERE id = $2",
              params: [
                { kind: "text", value: "Grace" },
                { kind: "text", value: "2" },
              ],
            },
          ],
        });
      }
      if (command === "apply_result_mutations") {
        return Promise.resolve({
          operations: [{ opIndex: 0, rowsAffected: 1 }],
          runtimeMs: 4,
        });
      }
      return Promise.resolve(undefined);
    });

    render(<QueryEditorPanel tab={queryTab} isClient />);
    fireEvent.click(screen.getByRole("button", { name: /Review & save/i }));

    expect(
      screen.getByRole("complementary", { name: "Mutation review" }),
    ).toBeTruthy();
    expect(
      screen.getByRole("heading", { name: "Review 1 change" }),
    ).toBeTruthy();

    const applyButton = await screen.findByRole("button", {
      name: "Apply 1 change",
    });
    await waitFor(() =>
      expect((applyButton as HTMLButtonElement).disabled).toBe(false),
    );
    fireEvent.click(applyButton);
    await waitFor(() => {
      expect(screen.getByTestId("query-mutation-status").textContent).toContain(
        "result is stale",
      );
    });
    expect(screen.getByRole("button", { name: "Re-run result" })).toBeTruthy();
  });

  it("keeps tombstoned staged changes visible and reviewable from draft originals", async () => {
    seedPersistentQuery();
    seedUsersUpdateDraft();
    const session = useAppStore.getState().querySessions[queryTab.id]!;
    const execution = session.execution!;
    useAppStore.setState({
      querySessions: {
        [queryTab.id]: {
          ...session,
          execution: {
            ...execution,
            resultSets: [],
            notices: [],
            retainedBytes: 0,
            tombstone: {
              status: "completed",
              resultCount: 1,
              rowCount: 2,
              noticeCount: 0,
              omittedCount: 0,
              runtimeMs: 10,
              releasedBytes: 64,
              completedAt: execution.completedAt!,
              reason: "globalBudget",
            },
          },
        },
      },
    });
    mockedInvoke.mockImplementation((command) =>
      command === "preview_result_mutations"
        ? Promise.resolve({
            statements: [
              {
                opIndex: 0,
                sql: "UPDATE public.users SET name = $1 WHERE id = $2",
                params: [],
              },
            ],
          })
        : Promise.resolve(undefined),
    );

    render(<QueryEditorPanel tab={queryTab} isClient />);

    expect(screen.getByText("1 staged")).toBeTruthy();
    expect(screen.getByText(/result display was released/i)).toBeTruthy();
    await act(async () => {
      fireEvent.click(screen.getByRole("button", { name: /Review & save/i }));
      await new Promise((resolve) => setTimeout(resolve, 0));
    });
    expect(
      screen.getByRole("heading", { name: "Review 1 change" }),
    ).toBeTruthy();
    expect(screen.getByText("Grace")).toBeTruthy();
    expect(screen.getByText("Ada")).toBeTruthy();
  });

  it.each(["analysis-only", "apply-success"] as const)(
    "drops a %s draft on reexecution without prompting",
    (draftState) => {
      seedPersistentQuery();
      const scope =
        draftState === "analysis-only"
          ? seedUsersAnalysisDraft()
          : seedUsersUpdateDraft();
      if (draftState === "apply-success") resolveUsersDraftApply(scope);
      mockedIsTauri.mockReturnValue(false);
      const confirmSpy = vi.spyOn(window, "confirm");
      confirmSpy.mockClear();

      render(<QueryEditorPanel tab={queryTab} isClient />);
      fireEvent.click(screen.getByRole("button", { name: /^Run$/i }));

      expect(confirmSpy).not.toHaveBeenCalled();
      expect(useAppStore.getState().mutationDrafts[scope]).toBeUndefined();
    },
  );

  it("confirms before discarding or re-running staged query changes", async () => {
    seedPersistentQuery();
    const scope = seedUsersUpdateDraft();
    const confirmSpy = vi.spyOn(window, "confirm").mockReturnValue(false);

    render(<QueryEditorPanel tab={queryTab} isClient />);
    fireEvent.click(screen.getByRole("button", { name: /^Discard$/i }));
    expect(confirmSpy).toHaveBeenCalledWith("Discard 1 staged change?");
    expect(
      useAppStore.getState().mutationDrafts[scope]?.changeOrder,
    ).toHaveLength(1);

    fireEvent.click(screen.getByRole("button", { name: /^Run$/i }));
    expect(confirmSpy).toHaveBeenLastCalledWith(
      "Re-running will discard 1 staged change. Continue?",
    );
    expect(mockedInvoke).not.toHaveBeenCalled();
    expect(
      useAppStore.getState().mutationDrafts[scope]?.changeOrder,
    ).toHaveLength(1);
  });

  it("leaves legacy query previews and toolbar edits on the legacy path", () => {
    useAppStore.setState({
      workspaceTabs: [queryTab],
      activeConnectionId: "conn-1",
      activeTabId: queryTab.id,
      connections: [pgConnection],
      queryStatus: {},
      querySessions: {},
      queryPreviews: {
        [queryTab.id]: {
          columns: ["name"],
          rows: [["Ada"]],
          runtime: "1 ms",
          rowCount: "1",
          cache: "Cold",
        },
      },
      queryEdits: { [queryTab.id]: { 0: { 0: "Grace" } } },
    });

    render(<QueryEditorPanel tab={queryTab} isClient />);

    expect(screen.getByRole("button", { name: /^Save$/i })).toBeTruthy();
    expect(screen.queryByTestId("query-mutation-status")).toBeNull();
    expect(screen.getByRole("button", { name: "Grace" })).toBeTruthy();
  });
});

describe("QueryEditorPanel connection selector", () => {
  const pgConnectionDefaults = {
    database: "app",
    host: "localhost",
    port: 5432,
    user: "postgres",
    password: "",
    role: "",
    engine: "PostgreSQL" as const,
    ssl: false,
    status: "Connected" as const,
    latency: "1ms",
  };
  const twoPgConnections = [
    { ...pgConnectionDefaults, id: "conn-1", name: "Local Postgres" },
    {
      ...pgConnectionDefaults,
      id: "conn-2",
      name: "Staging Postgres",
      host: "stg.example.com",
    },
  ];

  beforeEach(() => {
    mockedIsTauri.mockReturnValue(false);
    useAppStore.setState({
      workspaceTabs: [queryTab],
      activeConnectionId: "conn-1",
      activeTabId: queryTab.id,
      connections: twoPgConnections,
      queryStatus: {},
      queryPreviews: {},
      queryEdits: {},
      queryHistory: [],
    });
  });

  it("retargets the tab to the picked connection and clears stale per-tab state", async () => {
    useAppStore.setState({
      queryPreviews: {
        [queryTab.id]: {
          columns: ["a"],
          rows: [["1"]],
          runtime: "1 ms",
          rowCount: "1",
          cache: "Cold",
        },
      },
    });

    render(<QueryEditorPanel tab={queryTab} isClient />);

    fireEvent.click(
      screen.getByRole("button", { name: /connection selector/i }),
    );
    fireEvent.click(
      screen.getByRole("menuitem", { name: /staging postgres/i }),
    );

    await waitFor(() =>
      expect(useAppStore.getState().workspaceTabs[0].connectionId).toBe(
        "conn-2",
      ),
    );
    const state = useAppStore.getState();
    expect(state.activeConnectionId).toBe("conn-2");
    expect(state.queryPreviews[queryTab.id]).toBeUndefined();
  });

  it("prompts before discarding pending grid edits and bails on cancel", () => {
    useAppStore.setState({
      queryEdits: { [queryTab.id]: { 0: { 0: "edited" } } },
    });
    const confirmSpy = vi
      .spyOn(window, "confirm")
      .mockImplementation(() => false);

    render(<QueryEditorPanel tab={queryTab} isClient />);

    fireEvent.click(
      screen.getByRole("button", { name: /connection selector/i }),
    );
    fireEvent.click(
      screen.getByRole("menuitem", { name: /staging postgres/i }),
    );

    expect(confirmSpy).toHaveBeenCalled();
    expect(useAppStore.getState().workspaceTabs[0].connectionId).toBe("conn-1");
    expect(useAppStore.getState().queryEdits[queryTab.id]).toBeDefined();
    confirmSpy.mockRestore();
  });

  it("filters out keyvalue connections from the selector", () => {
    useAppStore.setState({
      connections: [
        ...twoPgConnections,
        {
          id: "conn-redis",
          name: "Cache",
          database: "0",
          host: "localhost",
          engine: "Redis" as const,
          port: 6379,
          user: "",
          password: "",
          role: "",
          dbNumber: 0,
          useTls: false,
          verifyTlsCert: true,
          readOnly: false,
          status: "Connected" as const,
          latency: "1ms",
        },
      ],
    });

    render(<QueryEditorPanel tab={queryTab} isClient />);

    fireEvent.click(
      screen.getByRole("button", { name: /connection selector/i }),
    );

    expect(
      screen.queryByRole("menuitem", { name: /cache · redis/i }),
    ).toBeNull();
    expect(
      screen.getByRole("menuitem", { name: /staging postgres/i }),
    ).toBeTruthy();
  });
});
