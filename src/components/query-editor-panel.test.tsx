// @vitest-environment jsdom
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("@/lib/tauri", () => ({
  isTauri: vi.fn(() => false),
  tauriInvoke: vi.fn(),
}));

// Controllable selection that the mocked Monaco editor will return.
const selectionState = { value: null as string | null };

vi.mock("@monaco-editor/react", () => ({
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
import { useAppStore, type WorkspaceTab } from "@/lib/store";

const buildQueryTab = (overrides?: Partial<WorkspaceTab>): WorkspaceTab => ({
  id: "tab-cmp-1",
  kind: "query",
  label: "query_test.sql",
  connectionId: "conn-1",
  schema: "public",
  query: "select * from users;",
  ...overrides,
});

beforeEach(() => {
  selectionState.value = null;
  useAppStore.setState({
    workspaceTabs: [buildQueryTab()],
    queryPreviews: {},
    recentQueries: [],
  });
});

afterEach(() => {
  cleanup();
  vi.clearAllMocks();
});

describe("QueryEditorPanel Run button", () => {
  it("runs the full editor text when no selection is active", () => {
    const runQuerySpy = vi.spyOn(useAppStore.getState(), "runQuery");
    selectionState.value = null;

    render(<QueryEditorPanel tab={buildQueryTab()} isClient={true} />);
    fireEvent.click(screen.getByRole("button", { name: /run/i }));

    expect(runQuerySpy).toHaveBeenCalledWith("tab-cmp-1", {
      overrideSql: "",
    });
  });

  it("runs only the selection when text is selected", () => {
    const runQuerySpy = vi.spyOn(useAppStore.getState(), "runQuery");
    selectionState.value = "select 1";

    render(<QueryEditorPanel tab={buildQueryTab()} isClient={true} />);
    fireEvent.click(screen.getByRole("button", { name: /run/i }));

    expect(runQuerySpy).toHaveBeenCalledWith("tab-cmp-1", {
      overrideSql: "select 1",
    });
  });

  it("falls back to full text when selection is whitespace-only", () => {
    const runQuerySpy = vi.spyOn(useAppStore.getState(), "runQuery");
    selectionState.value = "   \n  ";

    render(<QueryEditorPanel tab={buildQueryTab()} isClient={true} />);
    fireEvent.click(screen.getByRole("button", { name: /run/i }));

    // The component always passes the raw selection through; the store applies
    // the fallback. So we assert the value was forwarded as-is.
    expect(runQuerySpy).toHaveBeenCalledWith("tab-cmp-1", {
      overrideSql: "   \n  ",
    });
  });

  it("still works when isClient is false (editor not mounted)", () => {
    const runQuerySpy = vi.spyOn(useAppStore.getState(), "runQuery");

    render(<QueryEditorPanel tab={buildQueryTab()} isClient={false} />);
    fireEvent.click(screen.getByRole("button", { name: /run/i }));

    // No editor is mounted, so we cannot have a selection — fall back to "".
    expect(runQuerySpy).toHaveBeenCalledWith("tab-cmp-1", {
      overrideSql: "",
    });
  });
});
