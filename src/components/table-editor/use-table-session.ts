import { useCallback, useEffect, useMemo, useRef, useState } from "react";

import type { DataGridRowState } from "@/components/data-grid";
import type { ServerBrowseGridModel } from "@/components/data-grid/browse-model";
import { formatTlsFailure } from "@/lib/connection-diagnosis";
import type { InsertRowPayloadEntry } from "@/lib/insert-row-form";
import type {
  AnalyzeResultSetResult,
  AnalyzedTable,
  MutationValue,
  ResultMutationError,
} from "@/lib/result-mutation";
import { supportsResultMutations } from "@/lib/result-mutation";
import {
  analyzeResultSet,
  clearVirtualKey,
  loadVirtualKey,
  saveVirtualKey,
} from "@/lib/result-mutation-client";
import { readOnlyPolicyReason } from "@/lib/safety-policy";
import {
  type EditOutcome,
  isConnectedStatus,
  type MutationDraft,
  type TableDataState,
  type TableRef,
  tableMutationDraftScope,
  tableSessionKey,
  useAppStore,
  type WorkspaceTab,
} from "@/lib/store";
import {
  browseCellsToGrid,
  browseIdentityReadOnlyCopy,
  cycleSort,
  GRID_NULL_SENTINEL,
  gridCellToEditValue,
  identityIsEditable,
  supportsServerTableBrowse,
} from "@/lib/table-browse";
import { formatTableBrowseError } from "@/lib/table-browse-error";
import {
  buildTableSessionSnapshot,
  tableSessionStructureKey,
} from "@/lib/table-session";

import { useTablePagination } from "./use-table-pagination";

const NOOP_OUTCOME: EditOutcome = { kind: "noop" };

type MutationAnalysisState =
  | { state: "idle" }
  | { state: "loading" }
  | { state: "error"; message: string };

const mutationErrorCopy = (error: ResultMutationError): string => {
  switch (error.kind) {
    case "notAnalyzable":
      return error.reason.kind === "possibleTempShadowing"
        ? "A temporary table may shadow this relation. Editing is disabled."
        : "This relation could not be analyzed for editing.";
    case "unsupportedEngine":
      return "Staged mutations are unavailable for this database.";
    case "cancelled":
    case "superseded":
      return "Analysis was replaced by a newer request.";
    case "connectionClosing":
    case "connectionLost":
      return "The connection closed before analysis completed.";
    case "tlsFailed":
      return formatTlsFailure(error.tlsKind, error.message);
    case "timeout":
      return `Analysis timed out during ${error.operation}.`;
    case "database":
      return error.message;
    default:
      return "This relation is not currently editable.";
  }
};

const analyzedRelation = (
  analysis: AnalyzeResultSetResult | null,
  ref: TableRef | null,
): AnalyzedTable | null =>
  analysis?.tables.find(
    (table) => table.schema === ref?.schema && table.table === ref.table,
  ) ?? null;

const notAnalyzableCopy = (analysis: AnalyzeResultSetResult): string | null => {
  if (analysis.statement.kind === "analyzed") return null;
  switch (analysis.statement.reason.kind) {
    case "possibleTempShadowing":
      return "A temporary table may shadow this relation. Editing is disabled.";
    case "multiStatement":
      return "Only a single analyzed relation can be edited.";
    case "noProjectedColumns":
    case "noTableOrigins":
      return "No editable relation columns were found.";
    case "database":
      return analysis.statement.reason.message;
  }
};

const columnReadOnlyReason = (
  analysis: AnalyzeResultSetResult,
  table: AnalyzedTable | null,
  colIndex: number,
): string | undefined => {
  const statementReason = notAnalyzableCopy(analysis);
  if (statementReason) return statementReason;
  const column = analysis.columns[colIndex];
  if (!column) return "Column analysis is unavailable.";
  if (column.origin.kind !== "table") return "Expressions are read-only.";
  if (
    !table ||
    column.origin.schema !== table.schema ||
    column.origin.table !== table.table
  ) {
    return "This column belongs to a different relation.";
  }
  switch (column.writability.kind) {
    case "generated":
      return "Generated columns are read-only.";
    case "identityAlways":
      return "Identity-always columns are read-only.";
    case "systemColumn":
      return "System columns are read-only.";
    case "writable":
      break;
  }
  if (!table.updatable.allowed) return "This relation cannot be updated.";
  if (table.identity.kind === "none") return "Choose a virtual key to edit.";
  if (!table.identityProjected) return "The row identity is not available.";
  return undefined;
};

const rowOriginals = (
  columns: string[],
  row: Array<string | null>,
): MutationValue[] =>
  columns.map((column, index) => ({ column, value: row[index] ?? null }));

const guardedRowOriginals = (
  columns: string[],
  row: Array<string | null>,
  identity: MutationValue[],
): MutationValue[] => {
  const originals = rowOriginals(columns, row);
  const visibleColumns = new Set(originals.map(({ column }) => column));
  return [
    ...originals,
    ...identity.filter(({ column }) => !visibleColumns.has(column)),
  ];
};

const identityForBrowseRow = (
  table: AnalyzedTable,
  columns: string[],
  row: Array<string | null>,
  rowIdentity: string[] | null,
): MutationValue[] | null => {
  if (table.identity.kind === "none") return null;
  if (table.identity.kind === "virtualKey") {
    return table.identity.columns.map((column) => ({
      column,
      value: row[columns.indexOf(column)] ?? null,
    }));
  }
  if (rowIdentity?.length === table.identity.columns.length) {
    return table.identity.columns.map((column, index) => ({
      column,
      value: rowIdentity[index] ?? null,
    }));
  }
  const derived = table.identity.columns.map((column) => {
    const index = columns.indexOf(column);
    return index < 0 ? null : { column, value: row[index] ?? null };
  });
  return derived.every((value): value is MutationValue => value !== null)
    ? derived
    : null;
};

const mutationGridEdits = (
  draft: MutationDraft | undefined,
  columns: string[],
) => {
  const edits = new Map<number, Map<number, string>>();
  if (!draft) return {};
  for (const changeId of draft.changeOrder) {
    const change = draft.changes[changeId];
    if (change?.kind !== "updateRow" || change.rowIndex === null) continue;
    const rowEdits = new Map<number, string>();
    for (const column of change.cellOrder) {
      const colIndex = columns.indexOf(column);
      const cell = change.cells[column];
      if (colIndex >= 0 && cell) {
        rowEdits.set(colIndex, cell.value ?? GRID_NULL_SENTINEL);
      }
    }
    edits.set(change.rowIndex, rowEdits);
  }
  return Object.fromEntries(
    [...edits].map(([rowIndex, rowEdits]) => [
      rowIndex,
      Object.fromEntries(rowEdits),
    ]),
  );
};

export function useTableSession(tab: WorkspaceTab) {
  const ref = useMemo(() => {
    if (tab.kind !== "table" || !tab.table) return null;
    return {
      connectionId: tab.connectionId,
      schema: tab.schema,
      table: tab.table,
    };
  }, [tab.kind, tab.connectionId, tab.schema, tab.table]);
  const refKey = ref ? tableSessionKey(ref) : "";
  const structureKey = ref ? tableSessionStructureKey(ref) : "";

  const connection = useAppStore((state) =>
    state.connections.find((candidate) => candidate.id === tab.connectionId),
  );
  const engine = connection?.engine;
  const isConnected = isConnectedStatus(connection?.status);
  const policyReadOnlyCopy = connection
    ? readOnlyPolicyReason(connection)
    : null;
  const browseEnabled = Boolean(engine && supportsServerTableBrowse(engine));
  const mutationEnabled = Boolean(
    browseEnabled && engine && supportsResultMutations(engine),
  );
  const mutationScope = tableMutationDraftScope(tab.id);
  const mutationDraft = useAppStore(
    (state) => state.mutationDrafts[mutationScope],
  );
  const [mutationAnalysisState, setMutationAnalysisState] =
    useState<MutationAnalysisState>({ state: "idle" });
  const [virtualKeyColumns, setVirtualKeyColumns] = useState<string[]>([]);
  const analysisRequest = useRef<symbol | null>(null);
  const browseForTab = useAppStore((state) => state.tableBrowses[tab.id]);
  const browse =
    ref &&
    browseForTab?.connectionId === ref.connectionId &&
    browseForTab.schema === ref.schema &&
    browseForTab.table === ref.table
      ? browseForTab
      : undefined;

  const storeData = useAppStore((state) =>
    refKey ? state.tableData[refKey] : undefined,
  );
  const structure = useAppStore((state) =>
    structureKey ? state.tableStructure[structureKey] : undefined,
  );
  const status = useAppStore((state) =>
    refKey ? state.tableLoadStatus[refKey] : undefined,
  );
  /**
   * Restored tab whose connection hasn't been connected yet (Plan 010):
   * nothing has been fetched and nothing will be until the user
   * connects. Keyed on "no browse/session exists", not on raw status —
   * a live tab whose connection later drops (health-check flap,
   * explicit disconnect) keeps its grid and only the status pill moves.
   */
  const awaitingConnection =
    connection !== undefined &&
    !isConnected &&
    browse === undefined &&
    status === undefined &&
    storeData === undefined;
  const commitStatus = useAppStore((state) =>
    refKey ? state.tableEditsCommitStatus[refKey] : undefined,
  );
  const structureStatus = useAppStore((state) =>
    structureKey ? state.tableStructureStatus[structureKey] : undefined,
  );
  const currentEdits = useAppStore((state) =>
    refKey ? state.tableEdits[refKey] : undefined,
  );
  const openTableSession = useAppStore((state) => state.openTableSession);
  const openTableBrowse = useAppStore((state) => state.openTableBrowse);
  const loadTableStructure = useAppStore((state) => state.loadTableStructure);
  const refreshTableSession = useAppStore((state) => state.refreshTableSession);
  const refreshTableBrowse = useAppStore((state) => state.refreshTableBrowse);
  const loadTableData = useAppStore((state) => state.loadTableData);
  const setTableCellEdit = useAppStore((state) => state.setTableCellEdit);
  const discardTableCellEdits = useAppStore(
    (state) => state.discardTableCellEdits,
  );
  const commitTableCellEdits = useAppStore(
    (state) => state.commitTableCellEdits,
  );
  const insertTableRow = useAppStore((state) => state.insertTableRow);
  const deleteTableRows = useAppStore((state) => state.deleteTableRows);
  const openMutationDraft = useAppStore((state) => state.openMutationDraft);
  const setMutationDraftAnalysis = useAppStore(
    (state) => state.setMutationDraftAnalysis,
  );
  const stageMutationDraftUpdate = useAppStore(
    (state) => state.stageMutationDraftUpdate,
  );
  const stageMutationDraftDelete = useAppStore(
    (state) => state.stageMutationDraftDelete,
  );
  const stageMutationDraftInsert = useAppStore(
    (state) => state.stageMutationDraftInsert,
  );
  const discardMutationDraft = useAppStore(
    (state) => state.discardMutationDraft,
  );
  const rebindMutationDraftRows = useAppStore(
    (state) => state.rebindMutationDraftRows,
  );
  const setTableBrowseFilters = useAppStore(
    (state) => state.setTableBrowseFilters,
  );
  const clearTableBrowseFilters = useAppStore(
    (state) => state.clearTableBrowseFilters,
  );
  const setTableBrowseRawFilter = useAppStore(
    (state) => state.setTableBrowseRawFilter,
  );
  const setTableBrowseFilterMode = useAppStore(
    (state) => state.setTableBrowseFilterMode,
  );
  const setTableBrowseSort = useAppStore((state) => state.setTableBrowseSort);
  const setTableBrowsePageSize = useAppStore(
    (state) => state.setTableBrowsePageSize,
  );
  const goToTableBrowsePage = useAppStore((state) => state.goToTableBrowsePage);
  const goToTableBrowseNextPage = useAppStore(
    (state) => state.goToTableBrowseNextPage,
  );
  const goToTableBrowsePrevPage = useAppStore(
    (state) => state.goToTableBrowsePrevPage,
  );
  const goToTableBrowseFirstPage = useAppStore(
    (state) => state.goToTableBrowseFirstPage,
  );
  const goToTableBrowseLastPage = useAppStore(
    (state) => state.goToTableBrowseLastPage,
  );
  const countTableBrowseRows = useAppStore(
    (state) => state.countTableBrowseRows,
  );
  const cancelTableBrowse = useAppStore((state) => state.cancelTableBrowse);
  const applyTableBrowsePreset = useAppStore(
    (state) => state.applyTableBrowsePreset,
  );
  const saveTableBrowsePreset = useAppStore(
    (state) => state.saveTableBrowsePreset,
  );
  const applyTableBrowseHistory = useAppStore(
    (state) => state.applyTableBrowseHistory,
  );

  // One initial load per target. Session-restored tabs mount
  // disconnected, so the load waits for the first connect; after that
  // a status flap (health check, reconnect) must not re-open the
  // browse/session and reset the user's page.
  const loadFiredForRef = useRef<string | null>(null);
  useEffect(() => {
    if (!ref) return;
    if (!isConnected) return;
    if (loadFiredForRef.current === refKey) return;
    loadFiredForRef.current = refKey;
    if (browseEnabled) {
      void openTableBrowse(tab.id, ref.connectionId, ref.schema, ref.table);
      void loadTableStructure(ref.connectionId, ref.schema, ref.table);
      return;
    }
    void openTableSession(ref);
  }, [
    browseEnabled,
    isConnected,
    loadTableStructure,
    openTableBrowse,
    openTableSession,
    ref,
    refKey,
    tab.id,
  ]);

  useEffect(() => {
    setMutationAnalysisState({ state: "idle" });
    setVirtualKeyColumns([]);
    analysisRequest.current = null;
  }, [mutationScope, refKey]);

  const analyzeMutation = useCallback(
    async (
      refreshStructure = false,
      force = false,
    ): Promise<AnalyzeResultSetResult | null> => {
      if (policyReadOnlyCopy || !mutationEnabled || !ref) return null;
      if (!force && mutationDraft?.analysis) {
        return mutationDraft.analysis.snapshot;
      }
      if (analysisRequest.current) return null;
      const token = Symbol(mutationScope);
      analysisRequest.current = token;
      setMutationAnalysisState({ state: "loading" });
      const handle = openMutationDraft({
        owner: { kind: "table", tabId: tab.id },
        connectionId: ref.connectionId,
        source: {
          kind: "relation",
          schema: ref.schema,
          table: ref.table,
        },
      });
      if (!handle) {
        analysisRequest.current = null;
        setMutationAnalysisState({
          state: "error",
          message:
            "This relation cannot replace a draft that still has staged changes. Discard or apply them first.",
        });
        return null;
      }
      const result = await analyzeResultSet({
        connectionId: ref.connectionId,
        tabId: tab.id,
        source: {
          kind: "relation",
          schema: ref.schema,
          table: ref.table,
        },
        refreshStructure,
      });
      if (analysisRequest.current !== token) return null;
      analysisRequest.current = null;
      if (result.kind === "ok") {
        setMutationDraftAnalysis(handle, result.value);
        setMutationAnalysisState({ state: "idle" });
        const analyzed = analyzedRelation(result.value, ref);
        if (
          analyzed &&
          (analyzed.identity.kind === "none" || !analyzed.identityProjected)
        ) {
          const key = await loadVirtualKey({
            connectionId: ref.connectionId,
            schema: ref.schema,
            table: ref.table,
          });
          if (key.kind === "ok") setVirtualKeyColumns(key.value?.columns ?? []);
        } else if (analyzed?.identity.kind === "virtualKey") {
          setVirtualKeyColumns(analyzed.identity.columns);
        }
        return result.value;
      }
      if (result.kind === "superseded" || result.kind === "cancelled") {
        setMutationAnalysisState({ state: "idle" });
        return null;
      }
      setMutationAnalysisState({
        state: "error",
        message: mutationErrorCopy(result.error),
      });
      return null;
    },
    [
      mutationDraft?.analysis,
      mutationEnabled,
      mutationScope,
      openMutationDraft,
      policyReadOnlyCopy,
      ref,
      setMutationDraftAnalysis,
      tab.id,
    ],
  );

  const mutationAnalysis = mutationDraft?.analysis?.snapshot ?? null;
  const mutationTable = analyzedRelation(mutationAnalysis, ref);
  const browseColumns = useMemo(
    () => browse?.result?.columns.map((column) => column.name) ?? [],
    [browse?.result?.columns],
  );
  const rawBrowseRows = browse?.result?.rows ?? [];
  const insertedChanges = useMemo(
    () =>
      mutationEnabled && mutationDraft
        ? mutationDraft.changeOrder.flatMap((changeId) => {
            const change = mutationDraft.changes[changeId];
            return change?.kind === "insertRow" ? [change] : [];
          })
        : [],
    [mutationDraft, mutationEnabled],
  );

  const loadedRowsForRebind = useMemo(() => {
    if (!mutationTable || !browse?.result) return [];
    return browse.result.rows.flatMap((row, rowIndex) => {
      const identity = identityForBrowseRow(
        mutationTable,
        browseColumns,
        row,
        browse.result?.rowIdentity?.[rowIndex] ?? null,
      );
      return identity
        ? [
            {
              rowIndex,
              identity,
              values: guardedRowOriginals(browseColumns, row, identity),
            },
          ]
        : [];
    });
  }, [browse?.result, browseColumns, mutationTable]);

  useEffect(() => {
    if (!mutationEnabled || !mutationTable) return;
    rebindMutationDraftRows(
      mutationScope,
      { schema: mutationTable.schema, table: mutationTable.table },
      loadedRowsForRebind,
    );
  }, [
    loadedRowsForRebind,
    mutationEnabled,
    mutationScope,
    mutationTable,
    rebindMutationDraftRows,
  ]);

  const data: TableDataState | undefined = useMemo(() => {
    if (!browseEnabled || !ref || !browse?.result) return storeData;
    const count = browse.exactCount?.value ?? browse.result.count.value;
    const rows = browseCellsToGrid(browse.result.rows);
    if (mutationEnabled) {
      for (const change of insertedChanges) {
        const values = new Map(
          change.values.map(({ column, value }) => [column, value]),
        );
        rows.push(
          browseColumns.map(
            (column) => values.get(column) ?? GRID_NULL_SENTINEL,
          ),
        );
      }
    }
    return {
      connectionId: ref.connectionId,
      schema: ref.schema,
      table: ref.table,
      columns: browse.result.columns.map((column) => column.name),
      rows,
      page: browse.page,
      pageSize: browse.pageSize,
      totalRows: count ?? undefined,
      runtimeMs: browse.result.runtimeMs,
    };
  }, [
    browse?.exactCount,
    browse?.page,
    browse?.pageSize,
    browse?.result,
    browseColumns,
    browseEnabled,
    insertedChanges,
    mutationEnabled,
    ref,
    storeData,
  ]);

  const legacyPagination = useTablePagination({
    tab,
    data: storeData,
    loadTableData,
  });

  const countKind = browse?.exactCount?.kind ?? browse?.result?.count.kind;
  const countValue = browse?.exactCount?.value ?? browse?.result?.count.value;
  const browsePagination = useMemo(() => {
    const page = browse?.page ?? 1;
    const pageSize = browse?.pageSize ?? 100;
    const rowCount = data?.rows.length ?? 0;
    const totalRows = countValue ?? undefined;
    const canJump = countKind === "exact" || countKind === "estimated";
    const totalPages =
      canJump && totalRows !== undefined
        ? Math.max(1, Math.ceil(totalRows / pageSize))
        : undefined;
    const countLabel =
      countKind === "estimated" && totalRows !== undefined
        ? `~${totalRows.toLocaleString()} rows (estimated)`
        : countKind === "exact" && totalRows !== undefined
          ? `${totalRows.toLocaleString()} rows`
          : countKind === "unknown" || totalRows === undefined
            ? "unknown"
            : `${rowCount.toLocaleString()} rows`;
    return {
      page,
      pageSize,
      totalRows,
      totalPages,
      isLastPage: !(browse?.result?.pageInfo.hasMore ?? false),
      runtimeMs: browse?.result?.runtimeMs,
      rowCount,
      startRow: rowCount === 0 ? 0 : (page - 1) * pageSize + 1,
      endRow: (page - 1) * pageSize + rowCount,
      countLabel,
      canJump,
      countApproximate: countKind === "estimated",
      counting: browse?.countStatus.state === "loading",
      onCountRows: () => {
        void countTableBrowseRows(tab.id);
      },
      goToPage: (next: number) => {
        void goToTableBrowsePage(tab.id, next);
      },
      onPrevPage: () => {
        void goToTableBrowsePrevPage(tab.id);
      },
      onNextPage: () => {
        void goToTableBrowseNextPage(tab.id);
      },
      onFirstPage: () => {
        void goToTableBrowseFirstPage(tab.id);
      },
      onLastPage: () => {
        void goToTableBrowseLastPage(tab.id);
      },
    };
  }, [
    browse,
    countKind,
    countTableBrowseRows,
    countValue,
    data?.rows.length,
    goToTableBrowseFirstPage,
    goToTableBrowseLastPage,
    goToTableBrowseNextPage,
    goToTableBrowsePage,
    goToTableBrowsePrevPage,
    tab.id,
  ]);

  const session = useMemo(
    () =>
      ref
        ? buildTableSessionSnapshot({
            ref,
            data,
            structure,
            loadStatus: browseEnabled
              ? browse?.loadStatus.state === "error"
                ? {
                    state: "error",
                    error: formatTableBrowseError(browse.loadStatus.error),
                  }
                : browse?.loadStatus.state === "loading"
                  ? { state: "loading" }
                  : browse?.loadStatus.state === "success"
                    ? { state: "success" }
                    : { state: "idle" }
              : status,
            structureStatus,
            writeStatus: commitStatus,
            edits: currentEdits,
            browseIdentityKind: browse?.result?.identity.kind,
          })
        : null,
    [
      browse?.loadStatus,
      browse?.result?.identity.kind,
      browseEnabled,
      commitStatus,
      currentEdits,
      data,
      ref,
      status,
      structure,
      structureStatus,
    ],
  );

  const withOutcomeRef =
    <T extends unknown[]>(
      fn: (ref: TableRef, ...args: T) => Promise<EditOutcome>,
    ) =>
    async (...args: T): Promise<EditOutcome> => {
      if (!ref) return NOOP_OUTCOME;
      return fn(ref, ...args);
    };

  const currentMutationAnalysis = useCallback(
    () =>
      useAppStore.getState().mutationDrafts[mutationScope]?.analysis
        ?.snapshot ?? null,
    [mutationScope],
  );

  const stageCellEdit = useCallback(
    (rowIndex: number, colIndex: number, value: string) => {
      if (!mutationEnabled || !ref || !browse?.result) {
        if (ref) setTableCellEdit(ref, rowIndex, colIndex, value);
        return;
      }
      const analysis = currentMutationAnalysis();
      const table = analyzedRelation(analysis, ref);
      if (
        !analysis ||
        !table ||
        table.identity.kind === "none" ||
        columnReadOnlyReason(analysis, table, colIndex)
      ) {
        return;
      }
      const row = browse.result.rows[rowIndex];
      const analyzedColumn = analysis.columns[colIndex];
      if (!row || analyzedColumn?.origin.kind !== "table") return;
      const identity = identityForBrowseRow(
        table,
        browseColumns,
        row,
        browse.result.rowIdentity?.[rowIndex] ?? null,
      );
      if (!identity) return;
      stageMutationDraftUpdate(mutationScope, {
        table: { schema: table.schema, table: table.table },
        identityKind: table.identity.kind,
        identity,
        originals: guardedRowOriginals(browseColumns, row, identity),
        cells: [
          {
            column: analyzedColumn.origin.column,
            original: row[colIndex] ?? null,
            value: gridCellToEditValue(value),
          },
        ],
        rowIndex,
      });
    },
    [
      browse?.result,
      browseColumns,
      currentMutationAnalysis,
      mutationEnabled,
      mutationScope,
      ref,
      setTableCellEdit,
      stageMutationDraftUpdate,
    ],
  );

  const stageDeleteRows = useCallback(
    async (indices: number[]): Promise<boolean> => {
      if (!mutationEnabled || !ref || !browse?.result) return false;
      const analysis = currentMutationAnalysis() ?? (await analyzeMutation());
      const table = analyzedRelation(analysis, ref);
      if (
        !table ||
        !table.deletable.allowed ||
        table.identity.kind === "none" ||
        !table.identityProjected
      ) {
        return false;
      }
      let staged = false;
      for (const rowIndex of indices) {
        const row = browse.result.rows[rowIndex];
        if (!row) continue;
        const identity = identityForBrowseRow(
          table,
          browseColumns,
          row,
          browse.result.rowIdentity?.[rowIndex] ?? null,
        );
        if (!identity) continue;
        staged =
          stageMutationDraftDelete(mutationScope, {
            table: { schema: table.schema, table: table.table },
            identityKind: table.identity.kind,
            identity,
            originals: guardedRowOriginals(browseColumns, row, identity),
            rowIndex,
          }) !== null || staged;
      }
      return staged;
    },
    [
      analyzeMutation,
      browse?.result,
      browseColumns,
      currentMutationAnalysis,
      mutationEnabled,
      mutationScope,
      ref,
      stageMutationDraftDelete,
    ],
  );

  const stageInsert = useCallback(
    async (
      values: InsertRowPayloadEntry[],
      source: "new" | "duplicate" = "new",
    ): Promise<boolean> => {
      if (!mutationEnabled || !ref) return false;
      const analysis = currentMutationAnalysis() ?? (await analyzeMutation());
      const table = analyzedRelation(analysis, ref);
      if (!table?.insertable.allowed) return false;
      return (
        stageMutationDraftInsert(mutationScope, {
          table: { schema: table.schema, table: table.table },
          source,
          values,
        }) !== null
      );
    },
    [
      analyzeMutation,
      currentMutationAnalysis,
      mutationEnabled,
      mutationScope,
      ref,
      stageMutationDraftInsert,
    ],
  );

  const duplicateRowValues = useCallback(
    async (rowIndex: number): Promise<InsertRowPayloadEntry[] | null> => {
      if (!mutationEnabled || !ref || !browse?.result) return null;
      const analysis = currentMutationAnalysis() ?? (await analyzeMutation());
      const table = analyzedRelation(analysis, ref);
      const row = browse.result.rows[rowIndex];
      if (!analysis || !table?.insertable.allowed || !row) return null;
      return analysis.columns.flatMap((column, colIndex) => {
        if (
          column.origin.kind !== "table" ||
          column.origin.schema !== table.schema ||
          column.origin.table !== table.table ||
          column.writability.kind === "generated" ||
          column.writability.kind === "identityAlways" ||
          column.writability.kind === "systemColumn"
        ) {
          return [];
        }
        return [{ column: column.origin.column, value: row[colIndex] ?? null }];
      });
    },
    [
      analyzeMutation,
      browse?.result,
      currentMutationAnalysis,
      mutationEnabled,
      ref,
    ],
  );

  const stageBulkEdit = useCallback(
    async (
      indices: number[],
      columnName: string,
      value: string | null,
    ): Promise<number> => {
      if (!mutationEnabled || !ref || !browse?.result) return 0;
      const analysis = currentMutationAnalysis() ?? (await analyzeMutation());
      const table = analyzedRelation(analysis, ref);
      const colIndex = analysis?.columns.findIndex(
        (column) =>
          column.origin.kind === "table" &&
          column.origin.schema === table?.schema &&
          column.origin.table === table.table &&
          column.origin.column === columnName,
      );
      if (
        !analysis ||
        !table ||
        table.identity.kind === "none" ||
        colIndex === undefined ||
        colIndex < 0 ||
        columnReadOnlyReason(analysis, table, colIndex)
      ) {
        return 0;
      }
      let staged = 0;
      for (const rowIndex of indices) {
        const row = browse.result.rows[rowIndex];
        if (!row) continue;
        const identity = identityForBrowseRow(
          table,
          browseColumns,
          row,
          browse.result.rowIdentity?.[rowIndex] ?? null,
        );
        if (!identity) continue;
        const changeId = stageMutationDraftUpdate(mutationScope, {
          table: { schema: table.schema, table: table.table },
          identityKind: table.identity.kind,
          identity,
          originals: guardedRowOriginals(browseColumns, row, identity),
          cells: [
            {
              column: columnName,
              original: row[colIndex] ?? null,
              value,
            },
          ],
          rowIndex,
        });
        if (changeId) staged += 1;
      }
      return staged;
    },
    [
      analyzeMutation,
      browse?.result,
      browseColumns,
      currentMutationAnalysis,
      mutationEnabled,
      mutationScope,
      ref,
      stageMutationDraftUpdate,
    ],
  );

  const saveMutationVirtualKey = useCallback(
    async (columns: string[]): Promise<boolean> => {
      if (!mutationEnabled || !ref || columns.length === 0) return false;
      const result = await saveVirtualKey({
        connectionId: ref.connectionId,
        schema: ref.schema,
        table: ref.table,
        columns,
      });
      if (result.kind !== "ok") {
        if (result.kind === "error") {
          setMutationAnalysisState({
            state: "error",
            message: mutationErrorCopy(result.error),
          });
        }
        return false;
      }
      setVirtualKeyColumns(columns);
      return (await analyzeMutation(true, true)) !== null;
    },
    [analyzeMutation, mutationEnabled, ref],
  );

  const clearMutationVirtualKey = useCallback(async (): Promise<boolean> => {
    if (!mutationEnabled || !ref) return false;
    const result = await clearVirtualKey({
      connectionId: ref.connectionId,
      schema: ref.schema,
      table: ref.table,
    });
    if (result.kind !== "ok") return false;
    setVirtualKeyColumns([]);
    return (await analyzeMutation(true, true)) !== null;
  }, [analyzeMutation, mutationEnabled, ref]);

  const serverBrowse: ServerBrowseGridModel | undefined =
    browseEnabled && browse
      ? {
          typedFilters: browse.typedFilters,
          rawFilterText: browse.rawFilterText,
          filterMode: browse.filterMode,
          sort: browse.sort,
          pageSize: browse.pageSize,
          loadStatus: browse.loadStatus,
          error:
            browse.loadStatus.state === "error"
              ? browse.loadStatus.error
              : null,
          inspection: browse.result?.inspection ?? null,
          omittedRows: browse.result?.omittedRows ?? 0,
          truncatedCells: browse.result?.truncatedCells ?? 0,
          count: browse.result?.count ?? { kind: "unknown", value: null },
          exactCount: browse.exactCount,
          countStatus: browse.countStatus,
          pageInfo: browse.result?.pageInfo ?? null,
          history: browse.prefs.filterHistory,
          presets: browse.prefs.presets,
          onApplyTypedFilter: (filter) => {
            const next = [
              ...browse.typedFilters.filter((item) => {
                if (item.kind === "rawSql" || filter.kind === "rawSql")
                  return true;
                return item.column !== filter.column;
              }),
              filter,
            ];
            void setTableBrowseFilters(tab.id, next);
          },
          onRemoveTypedFilter: (column) => {
            void setTableBrowseFilters(
              tab.id,
              browse.typedFilters.filter(
                (item) => item.kind === "rawSql" || item.column !== column,
              ),
            );
          },
          onClearTypedFilters: () => {
            void clearTableBrowseFilters(tab.id);
          },
          onRawFilterApply: (text) => {
            void setTableBrowseRawFilter(tab.id, text);
          },
          onFilterModeChange: (mode) => {
            void setTableBrowseFilterMode(tab.id, mode);
          },
          onSortChange: (sort) => {
            void setTableBrowseSort(tab.id, sort);
          },
          onPageSizeChange: (pageSize) => {
            void setTableBrowsePageSize(tab.id, pageSize);
          },
          onHeaderSort: (column, append) => {
            void setTableBrowseSort(
              tab.id,
              cycleSort(browse.sort, column, append),
            );
          },
          onCountRows: () => {
            void countTableBrowseRows(tab.id);
          },
          onCancel: () => {
            void cancelTableBrowse(tab.id);
          },
          onApplyPreset: (name) => {
            void applyTableBrowsePreset(tab.id, name);
          },
          onSavePreset: (name) => {
            void saveTableBrowsePreset(tab.id, name);
          },
          onApplyHistory: (index) => {
            void applyTableBrowseHistory(tab.id, index);
          },
        }
      : undefined;

  const identityKind = browse?.result?.identity.kind;
  const legacyReadOnlyCopy =
    policyReadOnlyCopy ??
    (browseEnabled && identityKind && !identityIsEditable(identityKind)
      ? browseIdentityReadOnlyCopy(identityKind)
      : undefined);
  const mutationLocked = mutationDraft?.apply.state === "applying";
  const resolvedMutationEdits = useMemo(
    () => mutationGridEdits(mutationDraft, browseColumns),
    [browseColumns, mutationDraft],
  );
  const editableMutationColumns = useMemo(
    () =>
      mutationAnalysis
        ? mutationAnalysis.columns.flatMap((column, colIndex) =>
            column.origin.kind === "table" &&
            !columnReadOnlyReason(mutationAnalysis, mutationTable, colIndex)
              ? [column.origin.column]
              : [],
          )
        : [],
    [mutationAnalysis, mutationTable],
  );
  const insertableMutationColumns = useMemo(
    () =>
      mutationAnalysis
        ? mutationAnalysis.columns.flatMap((column) =>
            column.origin.kind === "table" &&
            column.origin.schema === mutationTable?.schema &&
            column.origin.table === mutationTable.table &&
            column.writability.kind === "writable"
              ? [column.origin.column]
              : [],
          )
        : [],
    [mutationAnalysis, mutationTable],
  );
  const mutationStatusCopy = (() => {
    if (!mutationEnabled) return undefined;
    if (policyReadOnlyCopy) return policyReadOnlyCopy;
    if (mutationAnalysisState.state === "loading") {
      return "Analyzing relation editability…";
    }
    if (mutationAnalysisState.state === "error") {
      return mutationAnalysisState.message;
    }
    if (!mutationAnalysis) return "Edit a cell to analyze this relation.";
    const statementReason = notAnalyzableCopy(mutationAnalysis);
    if (statementReason) return statementReason;
    if (!mutationTable) return "No editable target relation was resolved.";
    if (mutationTable.identity.kind === "none") {
      return "No proven identity. Choose projected columns as a virtual key.";
    }
    if (!mutationTable.identityProjected) {
      return "The identity is not projected. Choose a virtual key.";
    }
    if (mutationTable.identity.kind === "ctidFallback") {
      return "Editing with ctid and full-row guards. Stale rows fail closed.";
    }
    if (mutationTable.identity.kind === "virtualKey") {
      return `Virtual key: ${mutationTable.identity.columns.join(", ")}. Full-row guards apply.`;
    }
    return "Staged editing ready. Changes apply only after review.";
  })();
  const mutationReadOnly = Boolean(
    policyReadOnlyCopy ||
    (mutationAnalysis &&
      (!mutationTable ||
        mutationTable.identity.kind === "none" ||
        !mutationTable.identityProjected ||
        editableMutationColumns.length === 0)),
  );
  const mutationCapabilities = mutationEnabled
    ? {
        structureLoaded: Boolean(structure),
        isReadOnly: mutationReadOnly,
        isWriting: mutationLocked,
        canAddRow:
          Boolean(structure) &&
          !policyReadOnlyCopy &&
          !mutationLocked &&
          (mutationTable ? mutationTable.insertable.allowed : true),
        canDeleteRows:
          !policyReadOnlyCopy &&
          !mutationLocked &&
          (mutationTable
            ? mutationTable.deletable.allowed &&
              mutationTable.identity.kind !== "none" &&
              mutationTable.identityProjected
            : true),
        canEditCells:
          !policyReadOnlyCopy &&
          !mutationLocked &&
          editableMutationColumns.length > 0,
      }
    : undefined;
  const getMutationCellReadOnlyReason = (
    rowIndex: number,
    colIndex: number,
  ): string | undefined => {
    if (!mutationEnabled) return undefined;
    if (policyReadOnlyCopy) return policyReadOnlyCopy;
    if (rowIndex >= rawBrowseRows.length) {
      return "Edit staged inserts from the add-row form.";
    }
    if (!mutationAnalysis) {
      return mutationAnalysisState.state === "error"
        ? mutationAnalysisState.message
        : "Click to analyze relation editability.";
    }
    return columnReadOnlyReason(mutationAnalysis, mutationTable, colIndex);
  };
  const getMutationRowState = (
    rowIndex: number,
  ): DataGridRowState | undefined => {
    if (!mutationDraft) return undefined;
    if (rowIndex >= rawBrowseRows.length) {
      const insert = insertedChanges[rowIndex - rawBrowseRows.length];
      if (!insert) return undefined;
      if (!insert.included) return "excluded";
      return insert.source === "duplicate" ? "duplicate" : "inserted";
    }
    const changes = mutationDraft.changeOrder.flatMap((changeId) => {
      const change = mutationDraft.changes[changeId];
      return change &&
        change.kind !== "insertRow" &&
        change.rowIndex === rowIndex
        ? [change]
        : [];
    });
    if (changes.some((change) => !change.included)) return "excluded";
    return changes.some((change) => change.kind === "deleteRow")
      ? "deleted"
      : undefined;
  };

  return {
    ref,
    key: refKey,
    tableName: ref?.table ?? "",
    awaitingConnection,
    data: session?.data,
    structure: session?.structure,
    status: session?.loadStatus,
    commitStatus: session?.writeStatus,
    currentEdits: mutationEnabled ? resolvedMutationEdits : session?.edits,
    hasEdits: mutationEnabled
      ? (mutationDraft?.changeOrder.length ?? 0) > 0
      : Object.keys(session?.edits ?? {}).length > 0,
    capabilities:
      mutationCapabilities ??
      (policyReadOnlyCopy && session?.capabilities
        ? {
            ...session.capabilities,
            isReadOnly: true,
            canAddRow: false,
            canDeleteRows: false,
            canEditCells: false,
          }
        : session?.capabilities),
    pagination: browseEnabled ? browsePagination : legacyPagination,
    browseEnabled,
    serverBrowse,
    appliedBrowseRequestId: browse?.appliedRequestId ?? null,
    readOnlyCopy: mutationEnabled ? mutationStatusCopy : legacyReadOnlyCopy,
    mutationEnabled,
    mutationScope,
    mutationDraft,
    mutationAnalysis,
    mutationTable,
    mutationAnalysisState,
    mutationStatusCopy,
    mutationLocked,
    editableMutationColumns,
    insertableMutationColumns,
    virtualKeyColumns,
    needsVirtualKey:
      mutationEnabled &&
      Boolean(
        mutationTable &&
        (mutationTable.identity.kind === "none" ||
          !mutationTable.identityProjected),
      ),
    ensureMutationAnalysis: analyzeMutation,
    getCellReadOnlyReason: getMutationCellReadOnlyReason,
    getRowState: getMutationRowState,
    refresh: async () => {
      if (!ref) return;
      if (browseEnabled) {
        await refreshTableBrowse(tab.id);
        return;
      }
      await refreshTableSession(ref);
    },
    // Data-only writes can change row counts without invalidating the
    // backend relation descriptor. Legacy sessions already refresh data only.
    refreshData: async () => {
      if (!ref) return;
      if (browseEnabled) {
        await refreshTableBrowse(tab.id, {
          refreshStructure: false,
          invalidateExactCount: true,
        });
        return;
      }
      await refreshTableSession(ref);
    },
    setCellEdit: stageCellEdit,
    onEditIntent: () => {
      void analyzeMutation();
    },
    discardEdits: () => {
      if (mutationEnabled) {
        discardMutationDraft(mutationScope);
        return;
      }
      if (ref) discardTableCellEdits(ref);
    },
    commitEdits: withOutcomeRef((tableRef) =>
      commitTableCellEdits(tableRef, tab.id),
    ),
    addRow: withOutcomeRef((tableRef, values: InsertRowPayloadEntry[]) =>
      insertTableRow(tableRef, values),
    ),
    deleteRows: withOutcomeRef((tableRef, indices: number[]) =>
      deleteTableRows(tableRef, indices, tab.id),
    ),
    stageDeleteRows,
    stageInsert,
    duplicateRowValues,
    stageBulkEdit,
    saveMutationVirtualKey,
    clearMutationVirtualKey,
  };
}
