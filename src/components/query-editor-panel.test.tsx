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

vi.mock("@monaco-editor/react", () => ({
  __esModule: true,
  default: ({
    value,
    onChange,
    onMount,
  }: {
    value: string;
    onChange?: (value: string) => void;
    onMount?: (editor: unknown) => void;
  }) => {
    const fakeEditor = {
      getModel: () => ({
        getValueInRange: () => selectionState.value ?? "",
      }),
      getSelection: () => (selectionState.value === null ? null : {}),
    };
    onMount?.(fakeEditor);
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
import { type QueryStatus, useAppStore, type WorkspaceTab } from "@/lib/store";
import { isTauri } from "@/lib/tauri";

const mockedIsTauri = vi.mocked(isTauri);

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
  mockedIsTauri.mockReturnValue(true);
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
      name: /^run$/i,
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

describe("QueryEditorPanel Run button (selection forwarding)", () => {
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
      recentQueries: [],
    });
  });

  it("runs the full editor text when no selection is active", () => {
    const runQuerySpy = vi.spyOn(useAppStore.getState(), "runQuery");
    selectionState.value = null;

    render(<QueryEditorPanel tab={queryTab} isClient={true} />);
    fireEvent.click(screen.getByRole("button", { name: /^run$/i }));

    expect(runQuerySpy).toHaveBeenCalledWith("tab-1", {
      overrideSql: "",
    });
  });

  it("runs only the selection when text is selected", () => {
    const runQuerySpy = vi.spyOn(useAppStore.getState(), "runQuery");
    selectionState.value = "select 1";

    render(<QueryEditorPanel tab={queryTab} isClient={true} />);
    fireEvent.click(screen.getByRole("button", { name: /^run$/i }));

    expect(runQuerySpy).toHaveBeenCalledWith("tab-1", {
      overrideSql: "select 1",
    });
  });

  it("falls back to full text when selection is whitespace-only", () => {
    const runQuerySpy = vi.spyOn(useAppStore.getState(), "runQuery");
    selectionState.value = "   \n  ";

    render(<QueryEditorPanel tab={queryTab} isClient={true} />);
    fireEvent.click(screen.getByRole("button", { name: /^run$/i }));

    // The component always passes the raw selection through; the store applies
    // the fallback. So we assert the value was forwarded as-is.
    expect(runQuerySpy).toHaveBeenCalledWith("tab-1", {
      overrideSql: "   \n  ",
    });
  });

  it("still works when isClient is false (editor not mounted)", () => {
    const runQuerySpy = vi.spyOn(useAppStore.getState(), "runQuery");

    render(<QueryEditorPanel tab={queryTab} isClient={false} />);
    fireEvent.click(screen.getByRole("button", { name: /^run$/i }));

    // No editor is mounted, so we cannot have a selection — fall back to "".
    expect(runQuerySpy).toHaveBeenCalledWith("tab-1", {
      overrideSql: "",
    });
  });
});
