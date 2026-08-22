/* oxlint-disable anti-slop/no-module-mocking anti-slop/no-unknown-parameters anti-slop/no-runtime-typeof -- Tests control the mutation command boundary and inspect arbitrary console arguments to verify secrets never reach logging. */
// @vitest-environment jsdom
import {
  act,
  cleanup,
  fireEvent,
  render,
  screen,
  waitFor,
} from "@testing-library/react";
import type { ComponentProps } from "react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("@/lib/result-mutation-client", () => ({
  analyzeResultSet: vi.fn(),
  applyResultMutations: vi.fn(),
  cancelResultMutation: vi.fn(),
  previewResultMutations: vi.fn(),
}));

import { MutationReviewPanel } from "@/components/mutation-review";
import type {
  AnalyzeResultSetResult,
  ApplyResult,
  PreviewResult,
} from "@/lib/result-mutation";
import {
  analyzeResultSet,
  applyResultMutations,
  cancelResultMutation,
  previewResultMutations,
} from "@/lib/result-mutation-client";
import { type MutationDraftScope, useAppStore } from "@/lib/store";

const mockedAnalyze = vi.mocked(analyzeResultSet);
const mockedApply = vi.mocked(applyResultMutations);
const mockedCancel = vi.mocked(cancelResultMutation);
const mockedPreview = vi.mocked(previewResultMutations);

const SECRET_SQL = 'UPDATE "public"."users" SET "email" = $1';
const SECRET_VALUE = "new-secret@example.test";
const initialStoreState = useAppStore.getState();

const consoleSpies = {
  log: vi.spyOn(console, "log"),
  debug: vi.spyOn(console, "debug"),
  info: vi.spyOn(console, "info"),
  warn: vi.spyOn(console, "warn"),
  error: vi.spyOn(console, "error"),
};

const analysis = (analysisId = 41): AnalyzeResultSetResult => ({
  requestId: analysisId,
  analysisId,
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
      castType: "integer",
      nullable: false,
      writability: { kind: "writable" },
    },
    {
      name: "email",
      origin: {
        kind: "table",
        schema: "public",
        table: "users",
        column: "email",
        attnum: 2,
      },
      castType: "text",
      nullable: true,
      writability: { kind: "writable" },
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
  statement: { kind: "analyzed" },
});

const preview = (
  sql = SECRET_SQL,
  value = SECRET_VALUE,
  operations = 2,
): PreviewResult => ({
  statements: Array.from({ length: operations }, (_, opIndex) => ({
    opIndex,
    sql: `${sql} /* op ${opIndex + 1} */`,
    params: [{ kind: "text", value }],
  })),
});

const applySuccess = (operations = 2): ApplyResult => ({
  operations: Array.from({ length: operations }, (_, opIndex) => ({
    opIndex,
    rowsAffected: 1,
  })),
  runtimeMs: 4,
});

const openDraft = ({
  owner = "table",
  withAnalysis = true,
  offPage = false,
}: {
  owner?: "table" | "query";
  withAnalysis?: boolean;
  offPage?: boolean;
} = {}): MutationDraftScope => {
  const state = useAppStore.getState();
  const handle = state.openMutationDraft({
    owner:
      owner === "table"
        ? { kind: "table", tabId: "tab-1" }
        : {
            kind: "query",
            tabId: "tab-1",
            executionId: "exec-1",
            resultSetIndex: 0,
          },
    connectionId: "conn-1",
    source:
      owner === "table"
        ? { kind: "relation", schema: "public", table: "users" }
        : { kind: "statement", sql: "select id, email from public.users" },
  });
  if (withAnalysis) state.setMutationDraftAnalysis(handle, analysis());
  state.stageMutationDraftUpdate(handle.scope, {
    table: { schema: "public", table: "users" },
    identityKind: "primaryKey",
    identity: [{ column: "id", value: "1" }],
    originals: [
      { column: "id", value: "1" },
      { column: "email", value: "old@example.test" },
    ],
    cells: [
      {
        column: "email",
        original: "old@example.test",
        value: SECRET_VALUE,
      },
    ],
    rowIndex: offPage ? undefined : 0,
  });
  state.stageMutationDraftInsert(handle.scope, {
    table: { schema: "public", table: "users" },
    values: [
      { column: "email", value: null },
      { column: "status", value: "trial" },
    ],
  });
  return handle.scope;
};

const renderPanel = (
  scope: MutationDraftScope,
  props: Partial<ComponentProps<typeof MutationReviewPanel>> = {},
) => render(<MutationReviewPanel scope={scope} onClose={vi.fn()} {...props} />);

const loggedText = (): string =>
  Object.values(consoleSpies)
    .flatMap((spy) => spy.mock.calls)
    .flat()
    .map((value) => (typeof value === "string" ? value : JSON.stringify(value)))
    .join("\n");

beforeEach(() => {
  useAppStore.setState(initialStoreState, true);
  mockedAnalyze.mockReset().mockResolvedValue({
    kind: "ok",
    value: analysis(42),
  });
  mockedPreview.mockReset().mockResolvedValue({
    kind: "ok",
    value: preview(),
  });
  mockedApply.mockReset().mockResolvedValue({
    kind: "ok",
    value: applySuccess(),
  });
  mockedCancel.mockReset().mockResolvedValue({
    kind: "ok",
    value: { cancelRequested: true },
  });
  Object.defineProperty(navigator, "clipboard", {
    configurable: true,
    value: { writeText: vi.fn(() => Promise.resolve()) },
  });
  for (const spy of Object.values(consoleSpies)) {
    spy.mockClear();
    spy.mockImplementation(() => undefined);
  }
});

afterEach(() => {
  expect(loggedText()).not.toContain(SECRET_SQL);
  expect(loggedText()).not.toContain(SECRET_VALUE);
  cleanup();
  useAppStore.setState(initialStoreState, true);
});

describe("MutationReviewPanel", () => {
  it("labels off-page updates and deletes", async () => {
    const scope = openDraft({ offPage: true });
    useAppStore.getState().stageMutationDraftDelete(scope, {
      table: { schema: "public", table: "users" },
      identityKind: "primaryKey",
      identity: [{ column: "id", value: "2" }],
      originals: [
        { column: "id", value: "2" },
        { column: "email", value: "grace@example.test" },
      ],
    });
    renderPanel(scope);

    await screen.findByText(/Generated DML/);
    expect(screen.getAllByText("Off page")).toHaveLength(2);
  });

  it("renders dense grouped changes, qualified targets, DML, ordered params, and copy actions", async () => {
    const scope = openDraft();
    renderPanel(scope);

    expect(
      await screen.findByText(/Generated DML · 2 operations/),
    ).toBeTruthy();
    expect(screen.getAllByText("public.users").length).toBeGreaterThan(0);
    expect(screen.getByText("Updates · public.users")).toBeTruthy();
    expect(screen.getByText("Inserts · public.users")).toBeTruthy();
    expect(screen.getByText("old@example.test")).toBeTruthy();
    expect(screen.getAllByText(SECRET_VALUE).length).toBeGreaterThan(0);
    expect(screen.getByText("NULL")).toBeTruthy();
    expect(screen.getAllByText(/1\. new-secret@example\.test/).length).toBe(2);

    expect(mockedPreview).toHaveBeenCalledWith(
      expect.objectContaining({
        connectionId: "conn-1",
        tabId: "tab-1",
        analysisId: 41,
        plan: expect.objectContaining({ operations: expect.any(Array) }),
      }),
    );

    fireEvent.click(screen.getAllByRole("button", { name: "Copy SQL" })[0]);
    await waitFor(() =>
      expect(navigator.clipboard.writeText).toHaveBeenCalledWith(
        `${SECRET_SQL} /* op 1 */`,
      ),
    );
  });

  it("refreshes preview after include, per-change revert, and global revert", async () => {
    const scope = openDraft();
    renderPanel(scope);
    await screen.findByText(/Generated DML/);

    fireEvent.click(
      screen.getByRole("checkbox", {
        name: /Include Update row id = 1/,
      }),
    );
    await waitFor(() => expect(mockedPreview).toHaveBeenCalledTimes(2));
    const secondPayload = mockedPreview.mock.calls[1]?.[0];
    expect(secondPayload?.plan.operations).toHaveLength(1);

    fireEvent.click(screen.getByRole("button", { name: /Revert New row/ }));
    await waitFor(() =>
      expect(
        screen.getByRole("heading", { name: "Review 1 change" }),
      ).toBeTruthy(),
    );
    fireEvent.click(screen.getByRole("button", { name: "Revert all" }));
    expect(
      screen.getByRole("heading", { name: "Review 0 changes" }),
    ).toBeTruthy();
  });

  it("attributes rollback failures without dropping changes and offers refresh", async () => {
    mockedApply.mockResolvedValueOnce({
      kind: "error",
      error: { kind: "conflict", opIndex: 0 },
    });
    const refresh = vi.fn();
    const scope = openDraft();
    renderPanel(scope, { onRefresh: refresh });
    await screen.findByText(/Generated DML/);

    fireEvent.click(screen.getByRole("button", { name: "Apply 2 changes" }));
    expect(
      await screen.findByText(/An edited-column guard no longer matched/),
    ).toBeTruthy();
    expect(
      useAppStore.getState().mutationDrafts[scope]?.changeOrder,
    ).toHaveLength(2);
    fireEvent.click(screen.getByRole("button", { name: /Refresh rows/ }));
    await waitFor(() => expect(refresh).toHaveBeenCalledTimes(1));
  });

  it("requests apply cancellation and waits for the apply result", async () => {
    let finishApply: (
      value: Awaited<ReturnType<typeof applyResultMutations>>,
    ) => void = () => undefined;
    mockedApply.mockImplementationOnce(
      () =>
        new Promise((resolve) => {
          finishApply = resolve;
        }),
    );
    const scope = openDraft();
    renderPanel(scope);
    await screen.findByText(/Generated DML/);

    fireEvent.click(screen.getByRole("button", { name: "Apply 2 changes" }));
    fireEvent.click(
      await screen.findByRole("button", { name: "Cancel apply" }),
    );
    await waitFor(() => expect(mockedCancel).toHaveBeenCalledTimes(1));
    expect(screen.getByText(/Cancellation requested/)).toBeTruthy();

    await act(async () => {
      finishApply({ kind: "cancelled" });
    });
    expect(await screen.findByText(/mutation was cancelled/i)).toBeTruthy();
    expect(
      useAppStore.getState().mutationDrafts[scope]?.changeOrder,
    ).toHaveLength(2);
  });

  it("reports per-op success and exposes the query stale re-run affordance", async () => {
    const applied = vi.fn();
    const rerun = vi.fn();
    const scope = openDraft({ owner: "query" });
    renderPanel(scope, { onApplySuccess: applied, onRerunQuery: rerun });
    await screen.findByText(/Generated DML/);

    fireEvent.click(screen.getByRole("button", { name: "Apply 2 changes" }));
    expect(await screen.findByText(/Rows affected: 1, 1/)).toBeTruthy();
    expect(screen.getByText(/Query result is stale/)).toBeTruthy();
    expect(applied).toHaveBeenCalledWith(applySuccess());
    fireEvent.click(screen.getByRole("button", { name: "Re-run result" }));
    await waitFor(() => expect(rerun).toHaveBeenCalledTimes(1));
  });

  it("reanalyzes an expired preview once before showing it", async () => {
    mockedPreview
      .mockResolvedValueOnce({
        kind: "error",
        error: { kind: "analysisExpired" },
      })
      .mockResolvedValueOnce({ kind: "ok", value: preview() });
    const scope = openDraft();
    renderPanel(scope);

    expect(await screen.findByText(/Generated DML/)).toBeTruthy();
    expect(mockedAnalyze).toHaveBeenCalledTimes(1);
    expect(mockedPreview).toHaveBeenCalledTimes(2);
    expect(
      useAppStore.getState().mutationDrafts[scope]?.analysis?.analysisId,
    ).toBe(42);
  });

  it("retries expired apply only when regenerated DML is byte-identical", async () => {
    mockedApply
      .mockResolvedValueOnce({
        kind: "error",
        error: { kind: "analysisExpired" },
      })
      .mockResolvedValueOnce({ kind: "ok", value: applySuccess() });
    const scope = openDraft();
    renderPanel(scope);
    await screen.findByText(/Generated DML/);

    fireEvent.click(screen.getByRole("button", { name: "Apply 2 changes" }));
    await waitFor(() => expect(mockedApply).toHaveBeenCalledTimes(2));
    expect(await screen.findByText(/Rows affected: 1, 1/)).toBeTruthy();
    expect(mockedAnalyze).toHaveBeenCalledWith({
      connectionId: "conn-1",
      tabId: "tab-1",
      source: { kind: "relation", schema: "public", table: "users" },
      refreshStructure: false,
    });
  });

  it("requires explicit re-review when expiry recovery changes SQL or params", async () => {
    const changed = preview(
      "UPDATE public.users SET email = $1, touched = now()",
      "changed",
    );
    mockedPreview
      .mockResolvedValueOnce({ kind: "ok", value: preview() })
      .mockResolvedValueOnce({ kind: "ok", value: changed });
    mockedApply
      .mockResolvedValueOnce({
        kind: "error",
        error: { kind: "analysisExpired" },
      })
      .mockResolvedValueOnce({ kind: "ok", value: applySuccess() });
    const scope = openDraft();
    renderPanel(scope);
    await screen.findByText(/Generated DML/);

    fireEvent.click(screen.getByRole("button", { name: "Apply 2 changes" }));
    const review = await screen.findByRole("button", {
      name: "Review refreshed DML",
    });
    expect(mockedApply).toHaveBeenCalledTimes(1);
    expect(screen.getAllByText(/1\. changed/).length).toBeGreaterThan(0);

    fireEvent.click(review);
    fireEvent.click(screen.getByRole("button", { name: "Apply 2 changes" }));
    await waitFor(() => expect(mockedApply).toHaveBeenCalledTimes(2));
    expect(await screen.findByText(/Rows affected: 1, 1/)).toBeTruthy();
  });

  it("analyzes a draft that reaches review before an analysis snapshot", async () => {
    const scope = openDraft({ withAnalysis: false });
    renderPanel(scope);

    expect(await screen.findByText(/Generated DML/)).toBeTruthy();
    expect(mockedAnalyze).toHaveBeenCalledTimes(1);
    expect(mockedPreview).toHaveBeenCalledTimes(1);
  });
});
