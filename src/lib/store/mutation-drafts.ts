/**
 * Identity-keyed staged result mutations.
 *
 * Draft changes retain every value required to build a backend MutationPlan,
 * so pagination and query-result budget eviction never make a draft unusable.
 * Async owners fence completions with the handle returned by
 * `openMutationDraft`; user edits only need the scope.
 */

import type { StateCreator } from "zustand";

import type {
  AnalyzeResultSetResult,
  AnalyzeSource,
  ApplyResult,
  MutationIdentityKind,
  MutationPlan,
  MutationTable,
  MutationValue,
  PreviewResult,
  ResultMutationError,
} from "@/lib/result-mutation";

import type { AppStoreState } from "./types";

export type TableMutationDraftScope = `table:${string}`;
export type QueryMutationDraftScope = `query:${string}:${string}:${number}`;
export type MutationDraftScope =
  | TableMutationDraftScope
  | QueryMutationDraftScope;

export type MutationDraftOwner =
  | { kind: "table"; tabId: string }
  | {
      kind: "query";
      tabId: string;
      executionId: string;
      resultSetIndex: number;
    };

export type MutationDraftHandle = {
  scope: MutationDraftScope;
  generation: number;
};

type EditableIdentityKind = Exclude<MutationIdentityKind, "none">;

export type MutationDraftCell = {
  original: string | null;
  value: string | null;
};

type AttributableMutationError = Extract<
  ResultMutationError,
  {
    kind: "conflict" | "identityNotUnique" | "lockTimeout" | "database";
  }
>;

export type MutationDraftChangeFailure = {
  error: AttributableMutationError;
  opIndex: number;
};

type MutationDraftChangeBase = {
  changeId: string;
  included: boolean;
  failure: MutationDraftChangeFailure | null;
  table: MutationTable;
};

export type MutationDraftUpdate = MutationDraftChangeBase & {
  kind: "updateRow";
  identityKind: EditableIdentityKind;
  identityKey: string;
  identity: MutationValue[];
  /** Full projected row at staging time; required by virtual-key/ctid guards. */
  originals: MutationValue[];
  cells: Record<string, MutationDraftCell>;
  cellOrder: string[];
  /** Presentation metadata only. Never participates in plan construction. */
  rowIndex: number | null;
};

export type MutationDraftDelete = MutationDraftChangeBase & {
  kind: "deleteRow";
  identityKind: EditableIdentityKind;
  identityKey: string;
  identity: MutationValue[];
  /** Full projected row, including true NULLs, captured at staging time. */
  originals: MutationValue[];
  rowIndex: number | null;
};

export type MutationDraftInsert = MutationDraftChangeBase & {
  kind: "insertRow";
  /** Presentation-only provenance; both variants build the same insert op. */
  source: "new" | "duplicate";
  values: MutationValue[];
};

export type MutationDraftChange =
  | MutationDraftUpdate
  | MutationDraftDelete
  | MutationDraftInsert;

export type MutationDraftPlanBuild = {
  plan: MutationPlan;
  /** Array index is the backend opIndex; value is the stable changeId. */
  opIndexToChangeId: string[];
};

export type MutationDraftPreviewState =
  | { state: "idle" }
  | {
      state: "loading";
      requestId: number;
      build: MutationDraftPlanBuild;
    }
  | {
      state: "ready";
      requestId: number;
      build: MutationDraftPlanBuild;
      result: PreviewResult;
      /** False when expiry recovery changed the reviewed DML. */
      reviewed: boolean;
    }
  | { state: "error"; requestId: number; error: ResultMutationError };

export type MutationDraftApplyState =
  | { state: "idle" }
  | { state: "applying"; build: MutationDraftPlanBuild }
  | {
      state: "failed";
      error: ResultMutationError;
      changeId: string | null;
    }
  | {
      state: "success";
      result: ApplyResult;
      opIndexToChangeId: string[];
    };

export type MutationDraft = {
  scope: MutationDraftScope;
  owner: MutationDraftOwner;
  connectionId: string;
  source: AnalyzeSource;
  generation: number;
  analysis: {
    analysisId: number;
    snapshot: AnalyzeResultSetResult;
  } | null;
  changes: Record<string, MutationDraftChange>;
  changeOrder: string[];
  nextChangeOrdinal: number;
  nextPreviewRequestId: number;
  preview: MutationDraftPreviewState;
  apply: MutationDraftApplyState;
};

export type OpenMutationDraftInput = {
  owner: MutationDraftOwner;
  connectionId: string;
  source: AnalyzeSource;
};

export type StageMutationDraftUpdateInput = {
  table: MutationTable;
  identityKind: EditableIdentityKind;
  identity: MutationValue[];
  originals: MutationValue[];
  cells: Array<{
    column: string;
    original: string | null;
    value: string | null;
  }>;
  rowIndex?: number;
};

export type StageMutationDraftDeleteInput = {
  table: MutationTable;
  identityKind: EditableIdentityKind;
  identity: MutationValue[];
  originals: MutationValue[];
  rowIndex?: number;
};

export type StageMutationDraftInsertInput = {
  table: MutationTable;
  source?: "new" | "duplicate";
  values: MutationValue[];
};

export type MutationDraftLoadedRow = {
  rowIndex: number;
  identity: MutationValue[];
  /** Full projected row in the same value domain as staged originals. */
  values: MutationValue[];
};

export type MutationDraftPreviewRequest = MutationDraftHandle & {
  requestId: number;
  analysisId: number;
  build: MutationDraftPlanBuild;
};

export type MutationDraftApplyRequest = MutationDraftHandle & {
  analysisId: number;
  build: MutationDraftPlanBuild;
};

export type MutationDraftAnalysisRecovery = "stale" | "identical" | "changed";

export type MutationDraftsSlice = {
  mutationDrafts: Partial<Record<MutationDraftScope, MutationDraft>>;
  mutationDraftGenerations: Partial<Record<MutationDraftScope, number>>;
  openMutationDraft: (input: OpenMutationDraftInput) => MutationDraftHandle;
  setMutationDraftAnalysis: (
    handle: MutationDraftHandle,
    snapshot: AnalyzeResultSetResult,
  ) => boolean;
  stageMutationDraftUpdate: (
    scope: MutationDraftScope,
    input: StageMutationDraftUpdateInput,
  ) => string | null;
  stageMutationDraftDelete: (
    scope: MutationDraftScope,
    input: StageMutationDraftDeleteInput,
  ) => string | null;
  stageMutationDraftInsert: (
    scope: MutationDraftScope,
    input: StageMutationDraftInsertInput,
  ) => string | null;
  replaceMutationDraftInsertValues: (
    scope: MutationDraftScope,
    changeId: string,
    values: MutationValue[],
  ) => boolean;
  rebindMutationDraftRows: (
    scope: MutationDraftScope,
    table: MutationTable,
    rows: MutationDraftLoadedRow[],
  ) => boolean;
  setMutationDraftChangeIncluded: (
    scope: MutationDraftScope,
    changeId: string,
    included: boolean,
  ) => boolean;
  revertMutationDraftChange: (
    scope: MutationDraftScope,
    changeId: string,
  ) => boolean;
  revertAllMutationDraftChanges: (scope: MutationDraftScope) => boolean;
  beginMutationDraftPreview: (
    scope: MutationDraftScope,
  ) => MutationDraftPreviewRequest | null;
  resolveMutationDraftPreview: (
    request: MutationDraftPreviewRequest,
    result: PreviewResult,
  ) => boolean;
  failMutationDraftPreview: (
    request: MutationDraftPreviewRequest,
    error: ResultMutationError,
  ) => boolean;
  acknowledgeMutationDraftPreview: (scope: MutationDraftScope) => boolean;
  beginMutationDraftApply: (
    scope: MutationDraftScope,
  ) => MutationDraftApplyRequest | null;
  resolveMutationDraftApply: (
    request: MutationDraftApplyRequest,
    result: ApplyResult,
  ) => boolean;
  failMutationDraftApply: (
    request: MutationDraftApplyRequest,
    error: ResultMutationError,
  ) => boolean;
  recoverMutationDraftAnalysis: (
    handle: MutationDraftHandle,
    snapshot: AnalyzeResultSetResult,
    candidate: PreviewResult,
  ) => MutationDraftAnalysisRecovery;
  retryMutationDraftApplyAfterRecovery: (
    handle: MutationDraftHandle,
  ) => MutationDraftApplyRequest | null;
  discardMutationDraft: (scope: MutationDraftScope) => boolean;
  dropMutationDraftForScope: (scope: MutationDraftScope) => void;
  dropMutationDraftsForTab: (tabId: string) => void;
  dropMutationDraftsForExecution: (tabId: string, executionId: string) => void;
  dropMutationDraftsForConnection: (connectionId: string) => void;
};

export const tableMutationDraftScope = (
  tabId: string,
): TableMutationDraftScope => `table:${tabId}`;

export const queryMutationDraftScope = (
  tabId: string,
  executionId: string,
  resultSetIndex: number,
): QueryMutationDraftScope => `query:${tabId}:${executionId}:${resultSetIndex}`;

const scopeForOwner = (owner: MutationDraftOwner): MutationDraftScope =>
  owner.kind === "table"
    ? tableMutationDraftScope(owner.tabId)
    : queryMutationDraftScope(
        owner.tabId,
        owner.executionId,
        owner.resultSetIndex,
      );

const cloneValues = (values: MutationValue[]): MutationValue[] =>
  values.map(({ column, value }) => ({ column, value }));

const identityKey = (table: MutationTable, identity: MutationValue[]): string =>
  JSON.stringify([
    table.schema,
    table.table,
    [...identity]
      .sort((left, right) => left.column.localeCompare(right.column))
      .map(({ column, value }) => [column, value]),
  ]);

const valuesKey = (values: MutationValue[]): string =>
  JSON.stringify(
    [...values]
      .sort((left, right) => left.column.localeCompare(right.column))
      .map(({ column, value }) => [column, value]),
  );

const removeChange = (
  draft: MutationDraft,
  changeId: string,
): MutationDraft => {
  const { [changeId]: _removed, ...changes } = draft.changes;
  return {
    ...draft,
    changes,
    changeOrder: draft.changeOrder.filter((id) => id !== changeId),
  };
};

const invalidateReview = (draft: MutationDraft): MutationDraft => ({
  ...draft,
  preview: { state: "idle" },
  apply: { state: "idle" },
});

const clearFailures = (
  changes: Record<string, MutationDraftChange>,
): Record<string, MutationDraftChange> =>
  Object.fromEntries(
    Object.entries(changes).map(([changeId, change]) => [
      changeId,
      change.failure === null ? change : { ...change, failure: null },
    ]),
  );

const isLocked = (draft: MutationDraft): boolean =>
  draft.apply.state === "applying";

const changeForIdentity = (
  draft: MutationDraft,
  table: MutationTable,
  identity: MutationValue[],
): MutationDraftUpdate | MutationDraftDelete | null => {
  const key = identityKey(table, identity);
  for (const changeId of draft.changeOrder) {
    const change = draft.changes[changeId];
    if (change && change.kind !== "insertRow" && change.identityKey === key) {
      return change;
    }
  }
  return null;
};

const nextChangeId = (draft: MutationDraft): string =>
  `${draft.scope}:change:${draft.nextChangeOrdinal}`;

/** Build included operations in display order and retain exact attribution. */
export const buildMutationDraftPlan = (
  draft: MutationDraft,
): MutationDraftPlanBuild => {
  const operations: MutationPlan["operations"] = [];
  const opIndexToChangeId: string[] = [];
  for (const changeId of draft.changeOrder) {
    const change = draft.changes[changeId];
    if (!change?.included) continue;
    if (change.kind === "updateRow") {
      const set = change.cellOrder.flatMap((column) => {
        const cell = change.cells[column];
        return cell ? [{ column, value: cell.value }] : [];
      });
      const guards =
        change.identityKind === "virtualKey" ||
        change.identityKind === "ctidFallback"
          ? cloneValues(change.originals)
          : change.cellOrder.flatMap((column) => {
              const cell = change.cells[column];
              return cell ? [{ column, value: cell.original }] : [];
            });
      operations.push({
        kind: "update",
        table: { ...change.table },
        identity: cloneValues(change.identity),
        guards,
        set,
      });
    } else if (change.kind === "deleteRow") {
      operations.push({
        kind: "delete",
        table: { ...change.table },
        identity: cloneValues(change.identity),
        guards: cloneValues(change.originals),
      });
    } else {
      operations.push({
        kind: "insert",
        table: { ...change.table },
        values: cloneValues(change.values),
      });
    }
    opIndexToChangeId.push(changeId);
  }
  return { plan: { operations }, opIndexToChangeId };
};

export const mutationDraftPreviewsEqual = (
  left: PreviewResult,
  right: PreviewResult,
): boolean => JSON.stringify(left) === JSON.stringify(right);

/**
 * Rebind display row indexes after a browse load. Proven identities match on
 * identity alone. Virtual keys and ctid additionally require one exact
 * full-row match, failing closed on stale or ambiguous rows.
 */
export const rebindMutationDraftChanges = (
  draft: MutationDraft,
  table: MutationTable,
  rows: MutationDraftLoadedRow[],
): MutationDraft => {
  let changed = false;
  const changes = { ...draft.changes };
  for (const changeId of draft.changeOrder) {
    const change = draft.changes[changeId];
    if (
      !change ||
      change.kind === "insertRow" ||
      change.table.schema !== table.schema ||
      change.table.table !== table.table
    ) {
      continue;
    }
    const candidates = rows.filter(
      (loaded) => identityKey(table, loaded.identity) === change.identityKey,
    );
    const safeCandidates =
      change.identityKind === "virtualKey" ||
      change.identityKind === "ctidFallback"
        ? candidates.filter(
            (loaded) =>
              valuesKey(loaded.values) === valuesKey(change.originals),
          )
        : candidates;
    const rowIndex =
      safeCandidates.length === 1
        ? (safeCandidates[0]?.rowIndex ?? null)
        : null;
    if (change.rowIndex !== rowIndex) {
      changed = true;
      changes[changeId] = { ...change, rowIndex };
    }
  }
  return changed ? { ...draft, changes } : draft;
};

const draftMatchesHandle = (
  draft: MutationDraft | undefined,
  handle: MutationDraftHandle,
): draft is MutationDraft => draft?.generation === handle.generation;

const attributedOpIndex = (error: ResultMutationError): number | null => {
  switch (error.kind) {
    case "conflict":
    case "identityNotUnique":
    case "lockTimeout":
      return error.opIndex;
    case "database":
      return error.opIndex ?? null;
    default:
      return null;
  }
};

const isAttributableError = (
  error: ResultMutationError,
): error is AttributableMutationError =>
  error.kind === "conflict" ||
  error.kind === "identityNotUnique" ||
  error.kind === "lockTimeout" ||
  error.kind === "database";

const initialDraft = (
  input: OpenMutationDraftInput,
  scope: MutationDraftScope,
  generation: number,
): MutationDraft => ({
  scope,
  owner: input.owner,
  connectionId: input.connectionId,
  source: input.source,
  generation,
  analysis: null,
  changes: {},
  changeOrder: [],
  nextChangeOrdinal: 1,
  nextPreviewRequestId: 1,
  preview: { state: "idle" },
  apply: { state: "idle" },
});

export const createMutationDraftsSlice: StateCreator<
  AppStoreState,
  [],
  [],
  MutationDraftsSlice
> = (set, get) => ({
  mutationDrafts: {},
  mutationDraftGenerations: {},

  openMutationDraft: (input) => {
    const scope = scopeForOwner(input.owner);
    const existing = get().mutationDrafts[scope];
    if (
      existing &&
      existing.connectionId === input.connectionId &&
      JSON.stringify(existing.source) === JSON.stringify(input.source)
    ) {
      return { scope, generation: existing.generation };
    }
    const generation = (get().mutationDraftGenerations[scope] ?? 0) + 1;
    set((state) => ({
      mutationDrafts: {
        ...state.mutationDrafts,
        [scope]: initialDraft(input, scope, generation),
      },
      mutationDraftGenerations: {
        ...state.mutationDraftGenerations,
        [scope]: generation,
      },
    }));
    return { scope, generation };
  },

  setMutationDraftAnalysis: (handle, snapshot) => {
    let updated = false;
    set((state) => {
      const draft = state.mutationDrafts[handle.scope];
      if (!draftMatchesHandle(draft, handle) || isLocked(draft)) return {};
      updated = true;
      return {
        mutationDrafts: {
          ...state.mutationDrafts,
          [handle.scope]: {
            ...invalidateReview(draft),
            analysis: { analysisId: snapshot.analysisId, snapshot },
          },
        },
      };
    });
    return updated;
  },

  stageMutationDraftUpdate: (scope, input) => {
    let stagedChangeId: string | null = null;
    set((state) => {
      const current = state.mutationDrafts[scope];
      if (!current || isLocked(current)) return {};
      let draft = invalidateReview(current);
      const existing = changeForIdentity(draft, input.table, input.identity);
      if (existing?.kind === "deleteRow") return {};
      const changeId = existing?.changeId ?? nextChangeId(draft);
      const cells = existing?.cells ? { ...existing.cells } : {};
      const cellOrder = existing?.cellOrder ? [...existing.cellOrder] : [];
      for (const cell of input.cells) {
        const capturedOriginal = cells[cell.column]?.original ?? cell.original;
        if (cell.value === capturedOriginal) {
          delete cells[cell.column];
          const index = cellOrder.indexOf(cell.column);
          if (index >= 0) cellOrder.splice(index, 1);
        } else {
          if (!Object.hasOwn(cells, cell.column)) cellOrder.push(cell.column);
          cells[cell.column] = {
            original: capturedOriginal,
            value: cell.value,
          };
        }
      }
      if (cellOrder.length === 0) {
        if (!existing) return {};
        draft = removeChange(draft, existing.changeId);
        stagedChangeId = existing.changeId;
      } else {
        const change: MutationDraftUpdate = {
          kind: "updateRow",
          changeId,
          included: existing?.included ?? true,
          failure: null,
          table: { ...input.table },
          identityKind: input.identityKind,
          identityKey: identityKey(input.table, input.identity),
          identity: cloneValues(input.identity),
          originals: existing?.originals ?? cloneValues(input.originals),
          cells,
          cellOrder,
          rowIndex: input.rowIndex ?? existing?.rowIndex ?? null,
        };
        draft = {
          ...draft,
          changes: { ...draft.changes, [changeId]: change },
          changeOrder: existing
            ? draft.changeOrder
            : [...draft.changeOrder, changeId],
          nextChangeOrdinal: existing
            ? draft.nextChangeOrdinal
            : draft.nextChangeOrdinal + 1,
        };
        stagedChangeId = changeId;
      }
      return {
        mutationDrafts: { ...state.mutationDrafts, [scope]: draft },
      };
    });
    return stagedChangeId;
  },

  stageMutationDraftDelete: (scope, input) => {
    let stagedChangeId: string | null = null;
    set((state) => {
      const current = state.mutationDrafts[scope];
      if (!current || isLocked(current)) return {};
      let draft = invalidateReview(current);
      const existing = changeForIdentity(draft, input.table, input.identity);
      const changeId = existing?.changeId ?? nextChangeId(draft);
      const change: MutationDraftDelete = {
        kind: "deleteRow",
        changeId,
        included: existing?.included ?? true,
        failure: null,
        table: { ...input.table },
        identityKind: input.identityKind,
        identityKey: identityKey(input.table, input.identity),
        identity: cloneValues(input.identity),
        originals: existing?.originals ?? cloneValues(input.originals),
        rowIndex: input.rowIndex ?? existing?.rowIndex ?? null,
      };
      draft = {
        ...draft,
        changes: { ...draft.changes, [changeId]: change },
        changeOrder: existing
          ? draft.changeOrder
          : [...draft.changeOrder, changeId],
        nextChangeOrdinal: existing
          ? draft.nextChangeOrdinal
          : draft.nextChangeOrdinal + 1,
      };
      stagedChangeId = changeId;
      return {
        mutationDrafts: { ...state.mutationDrafts, [scope]: draft },
      };
    });
    return stagedChangeId;
  },

  stageMutationDraftInsert: (scope, input) => {
    let stagedChangeId: string | null = null;
    set((state) => {
      const current = state.mutationDrafts[scope];
      if (!current || isLocked(current)) return {};
      const draft = invalidateReview(current);
      const changeId = nextChangeId(draft);
      const change: MutationDraftInsert = {
        kind: "insertRow",
        changeId,
        included: true,
        failure: null,
        table: { ...input.table },
        source: input.source ?? "new",
        values: cloneValues(input.values),
      };
      stagedChangeId = changeId;
      return {
        mutationDrafts: {
          ...state.mutationDrafts,
          [scope]: {
            ...draft,
            changes: { ...draft.changes, [changeId]: change },
            changeOrder: [...draft.changeOrder, changeId],
            nextChangeOrdinal: draft.nextChangeOrdinal + 1,
          },
        },
      };
    });
    return stagedChangeId;
  },

  replaceMutationDraftInsertValues: (scope, changeId, values) => {
    let updated = false;
    set((state) => {
      const current = state.mutationDrafts[scope];
      const change = current?.changes[changeId];
      if (!current || isLocked(current) || change?.kind !== "insertRow") {
        return {};
      }
      updated = true;
      const draft = invalidateReview(current);
      return {
        mutationDrafts: {
          ...state.mutationDrafts,
          [scope]: {
            ...draft,
            changes: {
              ...draft.changes,
              [changeId]: { ...change, values: cloneValues(values) },
            },
          },
        },
      };
    });
    return updated;
  },

  rebindMutationDraftRows: (scope, table, rows) => {
    let rebound = false;
    set((state) => {
      const draft = state.mutationDrafts[scope];
      if (!draft) return {};
      const next = rebindMutationDraftChanges(draft, table, rows);
      if (next === draft) return {};
      rebound = true;
      return {
        mutationDrafts: { ...state.mutationDrafts, [scope]: next },
      };
    });
    return rebound;
  },

  setMutationDraftChangeIncluded: (scope, changeId, included) => {
    let updated = false;
    set((state) => {
      const current = state.mutationDrafts[scope];
      const change = current?.changes[changeId];
      if (!current || isLocked(current) || !change) return {};
      if (change.included === included) return {};
      updated = true;
      const draft = invalidateReview(current);
      return {
        mutationDrafts: {
          ...state.mutationDrafts,
          [scope]: {
            ...draft,
            changes: {
              ...draft.changes,
              [changeId]: { ...change, included, failure: null },
            },
          },
        },
      };
    });
    return updated;
  },

  revertMutationDraftChange: (scope, changeId) => {
    let reverted = false;
    set((state) => {
      const current = state.mutationDrafts[scope];
      if (!current || isLocked(current) || !current.changes[changeId]) {
        return {};
      }
      reverted = true;
      const draft = invalidateReview(removeChange(current, changeId));
      return {
        mutationDrafts: { ...state.mutationDrafts, [scope]: draft },
      };
    });
    return reverted;
  },

  revertAllMutationDraftChanges: (scope) => {
    let reverted = false;
    set((state) => {
      const draft = state.mutationDrafts[scope];
      if (!draft || isLocked(draft) || draft.changeOrder.length === 0) {
        return {};
      }
      reverted = true;
      return {
        mutationDrafts: {
          ...state.mutationDrafts,
          [scope]: {
            ...draft,
            changes: {},
            changeOrder: [],
            preview: { state: "idle" },
            apply: { state: "idle" },
          },
        },
      };
    });
    return reverted;
  },

  beginMutationDraftPreview: (scope) => {
    const draft = get().mutationDrafts[scope];
    if (!draft?.analysis || isLocked(draft)) return null;
    const build = buildMutationDraftPlan(draft);
    if (build.plan.operations.length === 0) return null;
    const requestId = draft.nextPreviewRequestId;
    set((state) => {
      const current = state.mutationDrafts[scope];
      if (!current || current.generation !== draft.generation) return {};
      return {
        mutationDrafts: {
          ...state.mutationDrafts,
          [scope]: {
            ...current,
            nextPreviewRequestId: requestId + 1,
            preview: { state: "loading", requestId, build },
          },
        },
      };
    });
    return {
      scope,
      generation: draft.generation,
      requestId,
      analysisId: draft.analysis.analysisId,
      build,
    };
  },

  resolveMutationDraftPreview: (request, result) => {
    let resolved = false;
    set((state) => {
      const draft = state.mutationDrafts[request.scope];
      if (
        !draftMatchesHandle(draft, request) ||
        draft.preview.state !== "loading" ||
        draft.preview.requestId !== request.requestId
      ) {
        return {};
      }
      resolved = true;
      return {
        mutationDrafts: {
          ...state.mutationDrafts,
          [request.scope]: {
            ...draft,
            preview: {
              state: "ready",
              requestId: request.requestId,
              build: request.build,
              result,
              reviewed: true,
            },
          },
        },
      };
    });
    return resolved;
  },

  failMutationDraftPreview: (request, error) => {
    let failed = false;
    set((state) => {
      const draft = state.mutationDrafts[request.scope];
      if (
        !draftMatchesHandle(draft, request) ||
        draft.preview.state !== "loading" ||
        draft.preview.requestId !== request.requestId
      ) {
        return {};
      }
      failed = true;
      return {
        mutationDrafts: {
          ...state.mutationDrafts,
          [request.scope]: {
            ...draft,
            preview: { state: "error", requestId: request.requestId, error },
          },
        },
      };
    });
    return failed;
  },

  acknowledgeMutationDraftPreview: (scope) => {
    let acknowledged = false;
    set((state) => {
      const draft = state.mutationDrafts[scope];
      if (!draft || draft.preview.state !== "ready") return {};
      acknowledged = true;
      return {
        mutationDrafts: {
          ...state.mutationDrafts,
          [scope]: {
            ...draft,
            preview: { ...draft.preview, reviewed: true },
          },
        },
      };
    });
    return acknowledged;
  },

  beginMutationDraftApply: (scope) => {
    const draft = get().mutationDrafts[scope];
    if (
      !draft?.analysis ||
      draft.preview.state !== "ready" ||
      !draft.preview.reviewed ||
      isLocked(draft)
    ) {
      return null;
    }
    const build = buildMutationDraftPlan(draft);
    if (JSON.stringify(build) !== JSON.stringify(draft.preview.build)) {
      return null;
    }
    set((state) => {
      const current = state.mutationDrafts[scope];
      if (!current || current.generation !== draft.generation) return {};
      return {
        mutationDrafts: {
          ...state.mutationDrafts,
          [scope]: {
            ...current,
            changes: clearFailures(current.changes),
            apply: { state: "applying", build },
          },
        },
      };
    });
    return {
      scope,
      generation: draft.generation,
      analysisId: draft.analysis.analysisId,
      build,
    };
  },

  resolveMutationDraftApply: (request, result) => {
    let resolved = false;
    set((state) => {
      const draft = state.mutationDrafts[request.scope];
      if (!draftMatchesHandle(draft, request) || !isLocked(draft)) return {};
      resolved = true;
      return {
        mutationDrafts: {
          ...state.mutationDrafts,
          [request.scope]: {
            ...draft,
            changes: {},
            changeOrder: [],
            preview: { state: "idle" },
            apply: {
              state: "success",
              result,
              opIndexToChangeId: request.build.opIndexToChangeId,
            },
          },
        },
      };
    });
    return resolved;
  },

  failMutationDraftApply: (request, error) => {
    let failed = false;
    set((state) => {
      const draft = state.mutationDrafts[request.scope];
      if (!draftMatchesHandle(draft, request) || !isLocked(draft)) return {};
      const opIndex = attributedOpIndex(error);
      const changeId =
        opIndex === null
          ? null
          : (request.build.opIndexToChangeId[opIndex] ?? null);
      let changes = draft.changes;
      if (changeId && isAttributableError(error) && opIndex !== null) {
        const change = changes[changeId];
        if (change) {
          changes = {
            ...changes,
            [changeId]: { ...change, failure: { error, opIndex } },
          };
        }
      }
      failed = true;
      return {
        mutationDrafts: {
          ...state.mutationDrafts,
          [request.scope]: {
            ...draft,
            changes,
            apply: { state: "failed", error, changeId },
          },
        },
      };
    });
    return failed;
  },

  recoverMutationDraftAnalysis: (handle, snapshot, candidate) => {
    let recovery: MutationDraftAnalysisRecovery = "stale";
    set((state) => {
      const draft = state.mutationDrafts[handle.scope];
      if (
        !draftMatchesHandle(draft, handle) ||
        draft.preview.state !== "ready"
      ) {
        return {};
      }
      const identical = mutationDraftPreviewsEqual(
        draft.preview.result,
        candidate,
      );
      recovery = identical ? "identical" : "changed";
      return {
        mutationDrafts: {
          ...state.mutationDrafts,
          [handle.scope]: {
            ...draft,
            analysis: { analysisId: snapshot.analysisId, snapshot },
            preview: {
              ...draft.preview,
              result: candidate,
              reviewed: identical,
            },
            apply: identical ? draft.apply : { state: "idle" },
          },
        },
      };
    });
    return recovery;
  },

  retryMutationDraftApplyAfterRecovery: (handle) => {
    const draft = get().mutationDrafts[handle.scope];
    if (
      !draftMatchesHandle(draft, handle) ||
      !draft.analysis ||
      draft.apply.state !== "applying" ||
      draft.preview.state !== "ready" ||
      !draft.preview.reviewed ||
      JSON.stringify(draft.apply.build) !== JSON.stringify(draft.preview.build)
    ) {
      return null;
    }
    return {
      ...handle,
      analysisId: draft.analysis.analysisId,
      build: draft.apply.build,
    };
  },

  discardMutationDraft: (scope) => {
    const draft = get().mutationDrafts[scope];
    if (!draft || isLocked(draft)) return false;
    get().dropMutationDraftForScope(scope);
    return true;
  },

  dropMutationDraftForScope: (scope) => {
    set((state) => {
      const { [scope]: _dropped, ...mutationDrafts } = state.mutationDrafts;
      return { mutationDrafts };
    });
  },

  dropMutationDraftsForTab: (tabId) => {
    set((state) => ({
      mutationDrafts: Object.fromEntries(
        Object.entries(state.mutationDrafts).filter(
          ([, draft]) => draft?.owner.tabId !== tabId,
        ),
      ),
    }));
  },

  dropMutationDraftsForExecution: (tabId, executionId) => {
    set((state) => ({
      mutationDrafts: Object.fromEntries(
        Object.entries(state.mutationDrafts).filter(([, draft]) => {
          const owner = draft?.owner;
          return !(
            owner?.kind === "query" &&
            owner.tabId === tabId &&
            owner.executionId === executionId
          );
        }),
      ),
    }));
  },

  dropMutationDraftsForConnection: (connectionId) => {
    set((state) => ({
      mutationDrafts: Object.fromEntries(
        Object.entries(state.mutationDrafts).filter(
          ([, draft]) => draft?.connectionId !== connectionId,
        ),
      ),
    }));
  },
});
