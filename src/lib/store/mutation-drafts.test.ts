import { afterEach, beforeEach, describe, expect, it } from "vitest";

import type {
  AnalyzeResultSetResult,
  MutationTable,
  MutationValue,
  PreviewResult,
} from "@/lib/result-mutation";
import {
  buildMutationDraftPlan,
  queryMutationDraftScope,
  tableMutationDraftScope,
  useAppStore,
} from "@/lib/store";

const initialStoreState = useAppStore.getState();
const table: MutationTable = { schema: "public", table: "users" };
const identity = (value: string): MutationValue[] => [{ column: "id", value }];
const row = (
  id: string,
  name: string | null,
  note: string | null = null,
): MutationValue[] => [
  { column: "id", value: id },
  { column: "name", value: name },
  { column: "note", value: note },
];

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

const preview = (labels: string[]): PreviewResult => ({
  statements: labels.map((label, opIndex) => ({
    opIndex,
    sql: `UPDATE public.users /* ${label} */`,
    params: [{ kind: "text", value: label }],
  })),
});

const resetStore = () => useAppStore.setState(initialStoreState, true);

const openTableDraft = () => {
  const handle = useAppStore.getState().openMutationDraft({
    owner: { kind: "table", tabId: "tab-1" },
    connectionId: "conn-1",
    source: { kind: "relation", schema: "public", table: "users" },
  });
  expect(
    useAppStore.getState().setMutationDraftAnalysis(handle, analysis()),
  ).toBe(true);
  return handle;
};

const stageUpdate = (
  id: string,
  original: string | null,
  value: string | null,
) =>
  useAppStore
    .getState()
    .stageMutationDraftUpdate(tableMutationDraftScope("tab-1"), {
      table,
      identityKind: "primaryKey",
      identity: identity(id),
      originals: row(id, original),
      cells: [{ column: "name", original, value }],
      rowIndex: Number(id),
    });

const previewTableDraft = (labels: string[]) => {
  const request = useAppStore
    .getState()
    .beginMutationDraftPreview(tableMutationDraftScope("tab-1"));
  expect(request).not.toBeNull();
  if (!request) throw new Error("Expected a preview request");
  expect(
    useAppStore
      .getState()
      .resolveMutationDraftPreview(request, preview(labels)),
  ).toBe(true);
  return request;
};

beforeEach(resetStore);
afterEach(resetStore);

describe("mutation draft staging", () => {
  it("captures true NULL originals and merges same-identity cell edits under a stable changeId", () => {
    openTableDraft();
    const firstId = stageUpdate("1", null, "Ada");
    const secondId = useAppStore
      .getState()
      .stageMutationDraftUpdate(tableMutationDraftScope("tab-1"), {
        table,
        identityKind: "primaryKey",
        identity: identity("1"),
        originals: row("1", "a later grid value", "old"),
        cells: [{ column: "note", original: "old", value: null }],
        rowIndex: 99,
      });

    expect(secondId).toBe(firstId);
    const draft =
      useAppStore.getState().mutationDrafts[tableMutationDraftScope("tab-1")];
    expect(draft?.changeOrder).toEqual([firstId]);
    const change = firstId ? draft?.changes[firstId] : undefined;
    expect(change).toMatchObject({
      kind: "updateRow",
      rowIndex: 99,
      cells: {
        name: { original: null, value: "Ada" },
        note: { original: "old", value: null },
      },
    });

    const build = draft ? buildMutationDraftPlan(draft) : null;
    expect(build?.plan.operations).toEqual([
      {
        kind: "update",
        table,
        identity: identity("1"),
        guards: [
          { column: "name", value: null },
          { column: "note", value: "old" },
        ],
        set: [
          { column: "name", value: "Ada" },
          { column: "note", value: null },
        ],
      },
    ]);
  });

  it("removes reverted cells and then the identity-keyed update without reusing its ordinal", () => {
    openTableDraft();
    const firstId = stageUpdate("1", "old", "new");
    expect(stageUpdate("1", "ignored", "old")).toBe(firstId);
    expect(
      useAppStore.getState().mutationDrafts[tableMutationDraftScope("tab-1")]
        ?.changeOrder,
    ).toEqual([]);

    const nextId = stageUpdate("1", "old", "newer");
    expect(nextId).not.toBe(firstId);
    expect(nextId).toContain(":change:2");
  });

  it("stages deletes and inserts in order and preserves update changeId when delete replaces it", () => {
    openTableDraft();
    const updateId = stageUpdate("1", "old", "new");
    const deleteId = useAppStore
      .getState()
      .stageMutationDraftDelete(tableMutationDraftScope("tab-1"), {
        table,
        identityKind: "primaryKey",
        identity: identity("1"),
        originals: row("1", "wrong incoming original"),
        rowIndex: 1,
      });
    const insertId = useAppStore
      .getState()
      .stageMutationDraftInsert(tableMutationDraftScope("tab-1"), {
        table,
        values: [
          { column: "name", value: "Grace" },
          { column: "note", value: null },
        ],
      });

    expect(deleteId).toBe(updateId);
    const draft =
      useAppStore.getState().mutationDrafts[tableMutationDraftScope("tab-1")];
    expect(draft?.changeOrder).toEqual([deleteId, insertId]);
    expect(deleteId ? draft?.changes[deleteId] : undefined).toMatchObject({
      kind: "deleteRow",
      originals: row("1", "old"),
    });
    expect(
      draft && insertId ? draft.changes[insertId] : undefined,
    ).toMatchObject({
      kind: "insertRow",
      source: "new",
      values: [
        { column: "name", value: "Grace" },
        { column: "note", value: null },
      ],
    });
  });

  it("retains duplicate provenance and allows staged insert values to be revised", () => {
    openTableDraft();
    const scope = tableMutationDraftScope("tab-1");
    const changeId = useAppStore.getState().stageMutationDraftInsert(scope, {
      table,
      source: "duplicate",
      values: [{ column: "name", value: "copy" }],
    });
    if (!changeId) throw new Error("Expected insert change");
    expect(
      useAppStore.getState().replaceMutationDraftInsertValues(scope, changeId, [
        { column: "name", value: "revised copy" },
        { column: "note", value: null },
      ]),
    ).toBe(true);
    expect(
      useAppStore.getState().mutationDrafts[scope]?.changes[changeId],
    ).toMatchObject({
      kind: "insertRow",
      source: "duplicate",
      values: [
        { column: "name", value: "revised copy" },
        { column: "note", value: null },
      ],
    });
  });
});

describe("mutation plan guards and attribution", () => {
  it("uses full captured rows for virtual-key updates and all deletes", () => {
    openTableDraft();
    const scope = tableMutationDraftScope("tab-1");
    useAppStore.getState().stageMutationDraftUpdate(scope, {
      table,
      identityKind: "virtualKey",
      identity: [{ column: "name", value: null }],
      originals: row("7", null, "before"),
      cells: [{ column: "note", original: "before", value: "after" }],
    });
    useAppStore.getState().stageMutationDraftDelete(scope, {
      table,
      identityKind: "primaryKey",
      identity: identity("8"),
      originals: row("8", "Lin", null),
    });
    const draft = useAppStore.getState().mutationDrafts[scope];
    expect(draft && buildMutationDraftPlan(draft).plan.operations).toEqual([
      {
        kind: "update",
        table,
        identity: [{ column: "name", value: null }],
        guards: row("7", null, "before"),
        set: [{ column: "note", value: "after" }],
      },
      {
        kind: "delete",
        table,
        identity: identity("8"),
        guards: row("8", "Lin", null),
      },
    ]);
  });

  it("maps included changes to contiguous op indexes and attributes typed rollback failures", () => {
    openTableDraft();
    const firstId = stageUpdate("1", "A", "A1");
    const excludedId = stageUpdate("2", "B", "B1");
    const thirdId = stageUpdate("3", "C", "C1");
    if (!firstId || !excludedId || !thirdId) {
      throw new Error("Expected staged changes");
    }
    expect(
      useAppStore
        .getState()
        .setMutationDraftChangeIncluded(
          tableMutationDraftScope("tab-1"),
          excludedId,
          false,
        ),
    ).toBe(true);

    const previewRequest = previewTableDraft(["first", "third"]);
    expect(previewRequest.build.opIndexToChangeId).toEqual([firstId, thirdId]);
    const applyRequest = useAppStore
      .getState()
      .beginMutationDraftApply(tableMutationDraftScope("tab-1"));
    expect(applyRequest).not.toBeNull();
    if (!applyRequest) throw new Error("Expected apply request");

    expect(
      useAppStore.getState().failMutationDraftApply(applyRequest, {
        kind: "conflict",
        opIndex: 1,
      }),
    ).toBe(true);
    const draft =
      useAppStore.getState().mutationDrafts[tableMutationDraftScope("tab-1")];
    expect(draft?.changeOrder).toEqual([firstId, excludedId, thirdId]);
    expect(draft?.apply).toMatchObject({
      state: "failed",
      changeId: thirdId,
    });
    expect(draft?.changes[thirdId]?.failure).toEqual({
      error: { kind: "conflict", opIndex: 1 },
      opIndex: 1,
    });
    expect(draft?.changes[firstId]?.failure).toBeNull();
    expect(draft?.changes[excludedId]?.failure).toBeNull();
  });

  it("attributes database and identity failures but leaves unattributed failures global", () => {
    openTableDraft();
    const changeId = stageUpdate("1", "A", "B");
    previewTableDraft(["one"]);
    let request = useAppStore
      .getState()
      .beginMutationDraftApply(tableMutationDraftScope("tab-1"));
    if (!request || !changeId) throw new Error("Expected apply request");
    useAppStore.getState().failMutationDraftApply(request, {
      kind: "database",
      code: "23514",
      message: "constraint failed",
      severity: "ERROR",
      position: null,
      opIndex: 0,
    });
    expect(
      useAppStore.getState().mutationDrafts[tableMutationDraftScope("tab-1")]
        ?.changes[changeId]?.failure?.error.kind,
    ).toBe("database");

    request = useAppStore
      .getState()
      .beginMutationDraftApply(tableMutationDraftScope("tab-1"));
    if (!request) throw new Error("Expected retry request");
    useAppStore.getState().failMutationDraftApply(request, { kind: "busy" });
    const draft =
      useAppStore.getState().mutationDrafts[tableMutationDraftScope("tab-1")];
    expect(draft?.apply).toEqual({
      state: "failed",
      error: { kind: "busy" },
      changeId: null,
    });
    expect(draft?.changes[changeId]?.failure).toBeNull();
  });
});

describe("mutation draft locks and review integrity", () => {
  it("blocks every staging, inclusion, revert and discard mutation while apply is running", () => {
    openTableDraft();
    const changeId = stageUpdate("1", "A", "B");
    previewTableDraft(["one"]);
    const request = useAppStore
      .getState()
      .beginMutationDraftApply(tableMutationDraftScope("tab-1"));
    expect(request).not.toBeNull();
    if (!changeId) throw new Error("Expected staged update");

    expect(stageUpdate("2", "C", "D")).toBeNull();
    expect(
      useAppStore
        .getState()
        .stageMutationDraftDelete(tableMutationDraftScope("tab-1"), {
          table,
          identityKind: "primaryKey",
          identity: identity("2"),
          originals: row("2", "C"),
        }),
    ).toBeNull();
    expect(
      useAppStore
        .getState()
        .stageMutationDraftInsert(tableMutationDraftScope("tab-1"), {
          table,
          values: [{ column: "name", value: "C" }],
        }),
    ).toBeNull();
    expect(
      useAppStore
        .getState()
        .setMutationDraftChangeIncluded(
          tableMutationDraftScope("tab-1"),
          changeId,
          false,
        ),
    ).toBe(false);
    expect(
      useAppStore
        .getState()
        .revertMutationDraftChange(tableMutationDraftScope("tab-1"), changeId),
    ).toBe(false);
    expect(
      useAppStore
        .getState()
        .discardMutationDraft(tableMutationDraftScope("tab-1")),
    ).toBe(false);
    expect(
      useAppStore.getState().mutationDrafts[tableMutationDraftScope("tab-1")]
        ?.changeOrder,
    ).toEqual([changeId]);
  });

  it("requires renewed review when analysis-expiry recovery changes SQL or parameters", () => {
    const handle = openTableDraft();
    stageUpdate("1", "A", "B");
    previewTableDraft(["reviewed"]);
    const firstApply = useAppStore
      .getState()
      .beginMutationDraftApply(tableMutationDraftScope("tab-1"));
    expect(firstApply).not.toBeNull();

    expect(
      useAppStore
        .getState()
        .recoverMutationDraftAnalysis(
          handle,
          analysis(42),
          preview(["reviewed"]),
        ),
    ).toBe("identical");
    expect(
      useAppStore.getState().mutationDrafts[tableMutationDraftScope("tab-1")]
        ?.apply.state,
    ).toBe("applying");
    expect(
      useAppStore.getState().retryMutationDraftApplyAfterRecovery(handle)
        ?.analysisId,
    ).toBe(42);

    expect(
      useAppStore
        .getState()
        .recoverMutationDraftAnalysis(
          handle,
          analysis(43),
          preview(["changed"]),
        ),
    ).toBe("changed");
    expect(
      useAppStore
        .getState()
        .beginMutationDraftApply(tableMutationDraftScope("tab-1")),
    ).toBeNull();
    expect(
      useAppStore.getState().retryMutationDraftApplyAfterRecovery(handle),
    ).toBeNull();
    expect(
      useAppStore
        .getState()
        .acknowledgeMutationDraftPreview(tableMutationDraftScope("tab-1")),
    ).toBe(true);
    expect(
      useAppStore
        .getState()
        .beginMutationDraftApply(tableMutationDraftScope("tab-1"))?.analysisId,
    ).toBe(43);
  });

  it("clears staged changes after success while retaining the reporting shell", () => {
    openTableDraft();
    const changeId = stageUpdate("1", "A", "B");
    previewTableDraft(["one"]);
    const request = useAppStore
      .getState()
      .beginMutationDraftApply(tableMutationDraftScope("tab-1"));
    if (!request || !changeId) throw new Error("Expected apply request");
    expect(
      useAppStore.getState().resolveMutationDraftApply(request, {
        operations: [{ opIndex: 0, rowsAffected: 1 }],
        runtimeMs: 5,
      }),
    ).toBe(true);

    const draft =
      useAppStore.getState().mutationDrafts[tableMutationDraftScope("tab-1")];
    expect(draft?.changeOrder).toEqual([]);
    expect(draft?.changes).toEqual({});
    expect(draft?.preview).toEqual({ state: "idle" });
    expect(draft?.apply).toEqual({
      state: "success",
      result: {
        operations: [{ opIndex: 0, rowsAffected: 1 }],
        runtimeMs: 5,
      },
      opIndexToChangeId: [changeId],
    });
  });
});

describe("mutation draft display rebinding", () => {
  it("rebinds proven identities by identity and marks off-page rows as null", () => {
    openTableDraft();
    const changeId = stageUpdate("1", "A", "B");
    const scope = tableMutationDraftScope("tab-1");
    if (!changeId) throw new Error("Expected staged update");
    expect(
      useAppStore
        .getState()
        .rebindMutationDraftRows(scope, table, [
          { rowIndex: 4, identity: identity("1"), values: row("1", "changed") },
        ]),
    ).toBe(true);
    expect(
      useAppStore.getState().mutationDrafts[scope]?.changes[changeId],
    ).toMatchObject({ rowIndex: 4 });

    expect(
      useAppStore.getState().rebindMutationDraftRows(scope, table, []),
    ).toBe(true);
    expect(
      useAppStore.getState().mutationDrafts[scope]?.changes[changeId],
    ).toMatchObject({ rowIndex: null });
  });

  it.each(["virtualKey", "ctidFallback"] as const)(
    "fails closed for stale, absent, or ambiguous %s full-row matches",
    (identityKind) => {
      openTableDraft();
      const scope = tableMutationDraftScope("tab-1");
      const changeId = useAppStore.getState().stageMutationDraftUpdate(scope, {
        table,
        identityKind,
        identity: identity("7"),
        originals: row("7", "original", null),
        cells: [{ column: "name", original: "original", value: "staged" }],
        rowIndex: 0,
      });
      if (!changeId) throw new Error("Expected staged update");

      useAppStore.getState().rebindMutationDraftRows(scope, table, [
        {
          rowIndex: 5,
          identity: identity("7"),
          values: row("7", "someone else", null),
        },
      ]);
      expect(
        useAppStore.getState().mutationDrafts[scope]?.changes[changeId],
      ).toMatchObject({ rowIndex: null });

      useAppStore.getState().rebindMutationDraftRows(scope, table, [
        {
          rowIndex: 6,
          identity: identity("7"),
          values: row("7", "original", null),
        },
      ]);
      expect(
        useAppStore.getState().mutationDrafts[scope]?.changes[changeId],
      ).toMatchObject({ rowIndex: 6 });

      useAppStore.getState().rebindMutationDraftRows(scope, table, [
        {
          rowIndex: 6,
          identity: identity("7"),
          values: row("7", "original", null),
        },
        {
          rowIndex: 7,
          identity: identity("7"),
          values: row("7", "original", null),
        },
      ]);
      expect(
        useAppStore.getState().mutationDrafts[scope]?.changes[changeId],
      ).toMatchObject({ rowIndex: null });
    },
  );
});

describe("mutation draft fencing and cleanup", () => {
  it("rejects stale preview and apply completions after scope recreation", () => {
    openTableDraft();
    stageUpdate("1", "A", "B");
    const stalePreview = useAppStore
      .getState()
      .beginMutationDraftPreview(tableMutationDraftScope("tab-1"));
    if (!stalePreview) throw new Error("Expected preview request");

    useAppStore
      .getState()
      .dropMutationDraftForScope(tableMutationDraftScope("tab-1"));
    const replacement = openTableDraft();
    expect(replacement.generation).toBeGreaterThan(stalePreview.generation);
    expect(
      useAppStore
        .getState()
        .setMutationDraftAnalysis(stalePreview, analysis(99)),
    ).toBe(false);
    expect(
      useAppStore
        .getState()
        .resolveMutationDraftPreview(stalePreview, preview(["stale"])),
    ).toBe(false);

    stageUpdate("2", "C", "D");
    previewTableDraft(["fresh"]);
    const staleApply = useAppStore
      .getState()
      .beginMutationDraftApply(tableMutationDraftScope("tab-1"));
    if (!staleApply) throw new Error("Expected apply request");
    useAppStore
      .getState()
      .dropMutationDraftForScope(tableMutationDraftScope("tab-1"));
    openTableDraft();
    expect(
      useAppStore.getState().resolveMutationDraftApply(staleApply, {
        operations: [{ opIndex: 0, rowsAffected: 1 }],
        runtimeMs: 1,
      }),
    ).toBe(false);
  });

  it("cleans exact tab, execution, and connection owners while unrelated scopes survive", () => {
    const state = useAppStore.getState();
    state.openMutationDraft({
      owner: { kind: "table", tabId: "table-tab" },
      connectionId: "conn-1",
      source: { kind: "relation", schema: "public", table: "users" },
    });
    state.openMutationDraft({
      owner: {
        kind: "query",
        tabId: "query-tab",
        executionId: "exec-1",
        resultSetIndex: 0,
      },
      connectionId: "conn-1",
      source: { kind: "statement", sql: "select * from users" },
    });
    state.openMutationDraft({
      owner: {
        kind: "query",
        tabId: "query-tab",
        executionId: "exec-2",
        resultSetIndex: 0,
      },
      connectionId: "conn-2",
      source: { kind: "statement", sql: "select * from users" },
    });

    useAppStore
      .getState()
      .dropMutationDraftsForExecution("query-tab", "exec-1");
    expect(
      useAppStore.getState().mutationDrafts[
        queryMutationDraftScope("query-tab", "exec-1", 0)
      ],
    ).toBeUndefined();
    expect(
      useAppStore.getState().mutationDrafts[
        queryMutationDraftScope("query-tab", "exec-2", 0)
      ],
    ).toBeDefined();

    useAppStore.getState().dropMutationDraftsForTab("table-tab");
    expect(
      useAppStore.getState().mutationDrafts[
        tableMutationDraftScope("table-tab")
      ],
    ).toBeUndefined();

    useAppStore.getState().dropMutationDraftsForConnection("conn-2");
    expect(Object.values(useAppStore.getState().mutationDrafts)).toEqual([]);
  });

  it("keeps a self-contained query draft when its result rows are tombstoned or released", () => {
    const handle = useAppStore.getState().openMutationDraft({
      owner: {
        kind: "query",
        tabId: "query-tab",
        executionId: "exec-1",
        resultSetIndex: 0,
      },
      connectionId: "conn-1",
      source: { kind: "statement", sql: "select id, name from users" },
    });
    useAppStore.getState().setMutationDraftAnalysis(handle, analysis());
    useAppStore.getState().stageMutationDraftUpdate(handle.scope, {
      table,
      identityKind: "primaryKey",
      identity: identity("1"),
      originals: row("1", null),
      cells: [{ column: "name", original: null, value: "kept" }],
    });

    // Result-budget release owns query-session rows, not mutationDrafts.
    useAppStore.setState((state) => ({
      querySessions: { ...state.querySessions },
    }));
    const draft = useAppStore.getState().mutationDrafts[handle.scope];
    expect(draft?.changes[draft.changeOrder[0] ?? ""]).toMatchObject({
      kind: "updateRow",
      originals: row("1", null),
      cells: { name: { original: null, value: "kept" } },
    });
  });
});
