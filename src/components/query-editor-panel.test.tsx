// @vitest-environment jsdom
import {
  act,
  cleanup,
  fireEvent,
  render,
  screen,
} from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("@/lib/tauri", () => ({
  isTauri: vi.fn(() => true),
  tauriInvoke: vi.fn(),
}));

// Controllable selection that the mocked Monaco editor will return.
const selectionState = { value: null as string | null };
const cursorState = { lineNumber: 1, column: 1 };
const registeredCompletionProviders: unknown[] = [];
const registeredActions: Array<{ id: string; run: () => void }> = [];
const decorationState = { values: [] as unknown[] };

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
      createDecorationsCollection: () => ({
        set: (decorations: unknown[]) => {
          decorationState.values = decorations;
        },
        clear: () => {
          decorationState.values = [];
        },
      }),
      onMouseDown: () => ({ dispose: vi.fn() }),
      onDidChangeModelContent: () => ({ dispose: vi.fn() }),
    };
    const fakeMonaco = {
      KeyCode: {
        Enter: 3,
      },
      KeyMod: {
        CtrlCmd: 2048,
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
import {
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
  it("disables the Run button while a query is running", () => {
    seedStatus({ state: "running" });

    render(<QueryEditorPanel tab={queryTab} isClient />);

    const runButton = screen.getByRole("button", {
      name: /running/i,
    }) as HTMLButtonElement;
    expect(runButton.disabled).toBe(true);
  });

  it("shows the error message in a banner when the query fails", () => {
    seedStatus({ state: "error", error: "syntax error at column 5" });

    render(<QueryEditorPanel tab={queryTab} isClient />);

    expect(screen.getByRole("alert").textContent).toContain(
      "syntax error at column 5",
    );
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
      name: /run current/i,
    }) as HTMLButtonElement;
    expect(runButton.disabled).toBe(false);
  });

  it("does not allow re-running while in flight", async () => {
    seedStatus({ state: "running" });

    render(<QueryEditorPanel tab={queryTab} isClient />);

    const runButton = screen.getByRole("button", { name: /running/i });

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

  it("runs the statement at the cursor by default", () => {
    const runQuerySpy = vi.spyOn(useAppStore.getState(), "runQuery");
    const multiQueryTab = {
      ...queryTab,
      query: "select 1;\nselect 2;",
    };
    cursorState.lineNumber = 2;
    cursorState.column = 3;

    render(<QueryEditorPanel tab={multiQueryTab} isClient={true} />);
    fireEvent.click(screen.getByRole("button", { name: /run current/i }));

    expect(runQuerySpy).toHaveBeenCalledWith("tab-1", {
      overrideSql: "select 2",
    });
  });

  it("runs only the selection from the selection control", () => {
    const runQuerySpy = vi.spyOn(useAppStore.getState(), "runQuery");
    selectionState.value = "select 1";

    render(<QueryEditorPanel tab={queryTab} isClient={true} />);
    fireEvent.click(screen.getByRole("button", { name: /selection/i }));

    expect(runQuerySpy).toHaveBeenCalledWith("tab-1", {
      overrideSql: "select 1",
    });
  });

  it("does not run selection when selection is whitespace-only", () => {
    const runQuerySpy = vi.spyOn(useAppStore.getState(), "runQuery");
    selectionState.value = "   \n  ";

    render(<QueryEditorPanel tab={queryTab} isClient={true} />);
    fireEvent.click(screen.getByRole("button", { name: /selection/i }));

    expect(runQuerySpy).not.toHaveBeenCalled();
  });

  it("runs the entire editor text from the all control", () => {
    const runQuerySpy = vi.spyOn(useAppStore.getState(), "runQuery");
    const multiQueryTab = {
      ...queryTab,
      query: "select 1;\nselect 2;",
    };

    render(<QueryEditorPanel tab={multiQueryTab} isClient={true} />);
    fireEvent.click(screen.getByRole("button", { name: /^all$/i }));

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
    fireEvent.click(screen.getByRole("button", { name: /run current/i }));

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
});
